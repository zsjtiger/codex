//! Attachment bytes live either on local disk or in an S3 bucket. The database stores only the
//! key, so switching backends does not touch existing rows.

use std::path::PathBuf;

use aws_config::BehaviorVersion;
use aws_sdk_s3::{
    Client,
    config::{Region, RequestChecksumCalculation},
    primitives::ByteStream,
};
use axum::body::Bytes;

use crate::config::StorageConfig;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("stored object is missing")]
    NotFound,
    #[error("invalid storage key")]
    InvalidKey,
    #[error("storage failure: {0}")]
    Backend(String),
}

pub enum ObjectStore {
    Local {
        root: PathBuf,
    },
    S3 {
        client: Client,
        bucket: String,
        /// Object keys are stored under this prefix, without a trailing slash. Empty for none.
        prefix: String,
    },
}

impl ObjectStore {
    pub fn local(root: impl Into<PathBuf>) -> Self {
        Self::Local { root: root.into() }
    }

    /// Builds an S3 store. Credentials and region come from the standard AWS chain unless the
    /// configuration sets them.
    pub async fn from_config(config: &StorageConfig) -> Self {
        match config {
            StorageConfig::Local { dir } => Self::local(dir),
            StorageConfig::S3 {
                bucket,
                prefix,
                endpoint,
                region,
                force_path_style,
            } => {
                let mut loader = aws_config::defaults(BehaviorVersion::latest());
                if let Some(region) = region {
                    loader = loader.region(Region::new(region.clone()));
                }
                let shared = loader.load().await;
                let mut builder = aws_sdk_s3::config::Builder::from(&shared)
                    .force_path_style(*force_path_style)
                    // Attachments are small, so skip the default trailing checksums and send plain bodies.
                    .request_checksum_calculation(RequestChecksumCalculation::WhenRequired);
                if let Some(endpoint) = endpoint {
                    builder = builder.endpoint_url(endpoint.clone());
                }
                Self::S3 {
                    client: Client::from_conf(builder.build()),
                    bucket: bucket.clone(),
                    prefix: prefix.clone(),
                }
            }
        }
    }

    /// An S3 store over an existing client, for tests and custom setups.
    pub fn s3_with_client(client: Client, bucket: impl Into<String>, prefix: &str) -> Self {
        Self::S3 {
            client,
            bucket: bucket.into(),
            prefix: prefix.trim_matches('/').to_string(),
        }
    }

    pub async fn put(&self, key: &str, bytes: Bytes) -> Result<(), StorageError> {
        check_key(key)?;
        match self {
            Self::Local { root } => {
                let path = root.join(key);
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(backend)?;
                }
                tokio::fs::write(&path, &bytes).await.map_err(backend)
            }
            Self::S3 {
                client,
                bucket,
                prefix,
            } => {
                client
                    .put_object()
                    .bucket(bucket)
                    .key(full_key(prefix, key))
                    .body(ByteStream::from(bytes))
                    .send()
                    .await
                    .map_err(|err| StorageError::Backend(err.to_string()))?;
                Ok(())
            }
        }
    }

    pub async fn get(&self, key: &str) -> Result<Vec<u8>, StorageError> {
        check_key(key)?;
        match self {
            Self::Local { root } => tokio::fs::read(root.join(key)).await.map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    StorageError::NotFound
                } else {
                    backend(err)
                }
            }),
            Self::S3 {
                client,
                bucket,
                prefix,
            } => {
                let output = client
                    .get_object()
                    .bucket(bucket)
                    .key(full_key(prefix, key))
                    .send()
                    .await
                    .map_err(|err| {
                        if err.as_service_error().is_some_and(|e| e.is_no_such_key()) {
                            StorageError::NotFound
                        } else {
                            StorageError::Backend(err.to_string())
                        }
                    })?;
                let collected = output
                    .body
                    .collect()
                    .await
                    .map_err(|err| StorageError::Backend(err.to_string()))?;
                Ok(collected.into_bytes().to_vec())
            }
        }
    }

    /// Removes an object. Deleting a key that does not exist is not an error.
    pub async fn delete(&self, key: &str) -> Result<(), StorageError> {
        check_key(key)?;
        match self {
            Self::Local { root } => match tokio::fs::remove_file(root.join(key)).await {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(backend(err)),
            },
            Self::S3 {
                client,
                bucket,
                prefix,
            } => {
                client
                    .delete_object()
                    .bucket(bucket)
                    .key(full_key(prefix, key))
                    .send()
                    .await
                    .map_err(|err| StorageError::Backend(err.to_string()))?;
                Ok(())
            }
        }
    }
}

fn backend(err: std::io::Error) -> StorageError {
    StorageError::Backend(err.to_string())
}

fn full_key(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}/{key}")
    }
}

/// Keys are generated by the gateway, but they are still checked so a key can never leave the store.
fn check_key(key: &str) -> Result<(), StorageError> {
    // Checked segment by segment: `Path::components` silently drops `.` in the middle of a path.
    let valid = !key.is_empty()
        && !key.starts_with('/')
        && key.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment.contains(['\\', '\0'])
        });
    if valid {
        Ok(())
    } else {
        Err(StorageError::InvalidKey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_must_stay_inside_the_store() {
        assert!(check_key("workspaces/a/attachments/b").is_ok());
        assert!(check_key("").is_err());
        assert!(check_key("../etc/passwd").is_err());
        assert!(check_key("/absolute").is_err());
        assert!(check_key("a/./b").is_err());
    }

    #[tokio::test]
    async fn local_store_round_trips_and_deletes() {
        let root = std::env::temp_dir().join(format!("gateway-storage-{}", uuid::Uuid::new_v4()));
        let store = ObjectStore::local(&root);
        store
            .put("ws/att/1", Bytes::from_static(b"hello"))
            .await
            .unwrap();
        assert_eq!(store.get("ws/att/1").await.unwrap(), b"hello");
        store.delete("ws/att/1").await.unwrap();
        assert!(matches!(
            store.get("ws/att/1").await,
            Err(StorageError::NotFound)
        ));
        store.delete("ws/att/1").await.unwrap();
        let _ = std::fs::remove_dir_all(root);
    }
}

use std::{path::PathBuf, time::Duration};

use crate::runtime::{LocalLaunch, RuntimeConfig};

/// Process configuration read from the environment.
#[derive(Debug, Clone)]
pub struct Config {
    pub addr: String,
    pub database_url: String,
    /// Enables `POST /api/v1/auth/sessions`, which issues a session for any email.
    /// Development only; keep it off in any deployed environment.
    pub dev_login: bool,
    /// Path to the `codex` binary, used to start app-servers for environments with no endpoint.
    pub codex_bin: Option<PathBuf>,
    /// Root directory for per-environment `CODEX_HOME`s, required with `codex_bin`.
    pub codex_home: Option<PathBuf>,
    pub idle_timeout: Duration,
    pub turns_per_minute: usize,
    /// Where attachment bytes are kept.
    pub storage: StorageConfig,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must be set".to_string())?;
        let dev_login = std::env::var("GATEWAY_DEV_LOGIN")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let codex_bin = std::env::var_os("GATEWAY_CODEX_BIN").map(PathBuf::from);
        let codex_home = std::env::var_os("GATEWAY_CODEX_HOME").map(PathBuf::from);
        if codex_bin.is_some() && codex_home.is_none() {
            return Err("GATEWAY_CODEX_HOME must be set when GATEWAY_CODEX_BIN is set".to_string());
        }
        let idle_secs = match std::env::var("GATEWAY_IDLE_SECS") {
            Ok(value) => value
                .parse::<u64>()
                .map_err(|_| "GATEWAY_IDLE_SECS must be a whole number of seconds".to_string())?,
            Err(_) => 30 * 60,
        };
        let turns_per_minute = match std::env::var("GATEWAY_TURNS_PER_MINUTE") {
            Ok(value) => value
                .parse::<usize>()
                .map_err(|_| "GATEWAY_TURNS_PER_MINUTE must be a whole number".to_string())?,
            Err(_) => crate::state::DEFAULT_TURNS_PER_MINUTE,
        };
        let storage = StorageConfig::from_env()?;
        Ok(Self {
            storage,
            turns_per_minute,
            addr: std::env::var("GATEWAY_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string()),
            database_url,
            dev_login,
            codex_bin,
            codex_home,
            idle_timeout: Duration::from_secs(idle_secs),
        })
    }

    pub fn runtime_config(&self) -> RuntimeConfig {
        let local =
            self.codex_bin
                .clone()
                .zip(self.codex_home.clone())
                .map(|(codex_bin, home_root)| LocalLaunch {
                    codex_bin,
                    home_root,
                });
        RuntimeConfig {
            local,
            idle_timeout: self.idle_timeout,
        }
    }
}

/// Where attachment bytes are stored. Selected with `GATEWAY_STORAGE`: `local` (default) or `s3`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageConfig {
    Local {
        dir: PathBuf,
    },
    S3 {
        bucket: String,
        prefix: String,
        /// Custom endpoint for S3-compatible services such as MinIO.
        endpoint: Option<String>,
        region: Option<String>,
        force_path_style: bool,
    },
}

impl StorageConfig {
    fn from_env() -> Result<Self, String> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Parses the storage variables through `lookup`, so the rules can be tested without touching
    /// the process environment. Empty values count as unset.
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let get = |name: &str| lookup(name).filter(|v| !v.is_empty());
        match get("GATEWAY_STORAGE").as_deref().unwrap_or("local") {
            "local" => Ok(Self::Local {
                dir: get("GATEWAY_STORAGE_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("storage")),
            }),
            "s3" => {
                let bucket = get("GATEWAY_S3_BUCKET").ok_or_else(|| {
                    "GATEWAY_S3_BUCKET must be set when GATEWAY_STORAGE=s3".to_string()
                })?;
                let force_path_style = get("GATEWAY_S3_FORCE_PATH_STYLE")
                    .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
                Ok(Self::S3 {
                    bucket,
                    prefix: get("GATEWAY_S3_PREFIX")
                        .unwrap_or_default()
                        .trim_matches('/')
                        .to_string(),
                    endpoint: get("GATEWAY_S3_ENDPOINT"),
                    region: get("GATEWAY_S3_REGION"),
                    force_path_style,
                })
            }
            other => Err(format!(
                "GATEWAY_STORAGE must be \"local\" or \"s3\", not {other:?}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn local_is_the_default_and_s3_needs_a_bucket() {
        assert_eq!(
            StorageConfig::from_lookup(lookup(&[])),
            Ok(StorageConfig::Local {
                dir: PathBuf::from("storage")
            })
        );
        assert_eq!(
            StorageConfig::from_lookup(lookup(&[
                ("GATEWAY_STORAGE", "local"),
                ("GATEWAY_STORAGE_DIR", "/data")
            ])),
            Ok(StorageConfig::Local {
                dir: PathBuf::from("/data")
            })
        );
        assert!(StorageConfig::from_lookup(lookup(&[("GATEWAY_STORAGE", "s3")])).is_err());
        assert!(StorageConfig::from_lookup(lookup(&[("GATEWAY_STORAGE", "gcs")])).is_err());
    }

    #[test]
    fn s3_settings_are_read_and_prefixes_trimmed() {
        assert_eq!(
            StorageConfig::from_lookup(lookup(&[
                ("GATEWAY_STORAGE", "s3"),
                ("GATEWAY_S3_BUCKET", "docs"),
                ("GATEWAY_S3_PREFIX", "/gateway/"),
                ("GATEWAY_S3_ENDPOINT", "http://minio:9000"),
                ("GATEWAY_S3_REGION", "us-east-1"),
                ("GATEWAY_S3_FORCE_PATH_STYLE", "true"),
            ])),
            Ok(StorageConfig::S3 {
                bucket: "docs".to_string(),
                prefix: "gateway".to_string(),
                endpoint: Some("http://minio:9000".to_string()),
                region: Some("us-east-1".to_string()),
                force_path_style: true,
            })
        );
    }
}

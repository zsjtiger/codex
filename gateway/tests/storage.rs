//! Attachment bytes in an S3 bucket. S3 is replaced by a wiremock server that records requests.

mod common;

use aws_sdk_s3::{
    Client,
    config::{Credentials, Region, RequestChecksumCalculation},
};
use axum::{body::Bytes, http::StatusCode};
use codex_gateway::{
    AppState,
    runtime::RuntimeConfig,
    storage::{ObjectStore, StorageError},
};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

async fn s3_store(server: &MockServer) -> ObjectStore {
    let conf = aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new("AKID", "SECRET", None, None, "test"))
        .endpoint_url(server.uri())
        .force_path_style(true)
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .build();
    ObjectStore::s3_with_client(Client::from_conf(conf), "docs", "gateway/")
}

fn state_with(pool: &PgPool, store: ObjectStore) -> AppState {
    AppState::new(pool.clone(), true, RuntimeConfig::default()).with_object_store(store)
}

#[tokio::test]
async fn s3_store_puts_gets_and_deletes_under_the_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/docs/gateway/ws/att/1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/docs/gateway/ws/att/1"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello".to_vec()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/docs/gateway/ws/att/1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let store = s3_store(&server).await;
    store
        .put("ws/att/1", Bytes::from_static(b"hello"))
        .await
        .unwrap();
    assert_eq!(store.get("ws/att/1").await.unwrap(), b"hello");
    store.delete("ws/att/1").await.unwrap();

    // The upload is a plain body with no trailing checksum.
    let requests: Vec<Request> = server.received_requests().await.unwrap();
    let put = requests
        .iter()
        .find(|r| r.method.as_str() == "PUT")
        .unwrap();
    assert_eq!(put.body, b"hello");
    assert!(put.headers.get("x-amz-trailer").is_none());
}

#[tokio::test]
async fn s3_missing_key_is_reported_as_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_string(
            "<Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message></Error>",
        ))
        .mount(&server)
        .await;

    let store = s3_store(&server).await;
    assert!(matches!(
        store.get("ws/att/missing").await,
        Err(StorageError::NotFound)
    ));
}

#[tokio::test]
async fn keys_that_escape_the_prefix_are_rejected_before_any_request() {
    let server = MockServer::start().await;
    let store = s3_store(&server).await;
    assert!(matches!(
        store.put("../other/object", Bytes::from_static(b"x")).await,
        Err(StorageError::InvalidKey)
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn attachment_bytes_are_stored_in_s3_and_removed_on_delete(pool: PgPool) {
    let server = MockServer::start().await;
    let state = state_with(&pool, s3_store(&server).await);
    let owner = login(&state, "owner@example.com").await;
    let (_, ws) = call(
        &state,
        "POST",
        "/api/v1/workspaces",
        &owner,
        Some(json!({ "slug": "s3-files", "name": "S3 files" })),
    )
    .await;
    let ws_id = ws["id"].as_str().unwrap().to_string();

    let bytes = b"hello from s3".to_vec();
    let (_, created) = call(
        &state,
        "POST",
        &format!("/api/v1/workspaces/{ws_id}/attachments"),
        &owner,
        Some(json!({ "filename": "note.txt", "content_type": "text/plain", "size_bytes": bytes.len() })),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();
    let key = format!("/docs/gateway/workspaces/{ws_id}/attachments/{id}");

    Mock::given(method("PUT"))
        .and(path(key.as_str()))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(key.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(key.as_str()))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let content = format!("/api/v1/attachments/{id}/content");
    let (status, _, _) =
        call_raw(&state, "PUT", &content, &owner, "text/plain", bytes.clone()).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, downloaded) =
        call_raw(&state, "GET", &content, &owner, "text/plain", vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(downloaded, bytes);

    let (status, _) = call(
        &state,
        "DELETE",
        &format!("/api/v1/attachments/{id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Every expected request was received exactly once, including the DELETE.
    server.verify().await;
}

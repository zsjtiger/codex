use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    audit,
    auth::CurrentUser,
    authz::{Role, require_role},
    error::ApiError,
    state::AppState,
    validate,
};

/// Largest attachment accepted, in bytes.
pub const MAX_ATTACHMENT_BYTES: i64 = 50 * 1024 * 1024;

#[derive(Serialize, FromRow)]
pub struct AttachmentView {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub thread_id: Option<Uuid>,
    pub uploaded_by: Option<Uuid>,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: Option<String>,
    /// Key of the stored bytes in the attachment store.
    pub storage_key: String,
    /// True once the bytes have been uploaded to `/content`.
    pub uploaded: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct CreateAttachment {
    filename: String,
    content_type: String,
    size_bytes: i64,
    sha256: Option<String>,
    thread_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct ListAttachmentsQuery {
    thread_id: Option<Uuid>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces/{id}/attachments", get(list).post(create))
        .route("/attachments/{id}", axum::routing::delete(delete))
        .route(
            "/attachments/{id}/content",
            put(upload)
                .get(download)
                .layer(DefaultBodyLimit::max(MAX_ATTACHMENT_BYTES as usize)),
        )
}

const COLUMNS: &str = "id, workspace_id, thread_id, uploaded_by, filename, content_type, size_bytes, \
                       sha256, storage_key, uploaded, created_at";

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateAttachment>,
) -> Result<(StatusCode, Json<AttachmentView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    validate::length("filename", &req.filename, 1, 255)?;
    validate::length("content_type", &req.content_type, 3, 255)?;
    if !req.content_type.contains('/') {
        return Err(ApiError::BadRequest(
            "content_type must contain '/'".to_string(),
        ));
    }
    if !(0..=MAX_ATTACHMENT_BYTES).contains(&req.size_bytes) {
        return Err(ApiError::BadRequest(format!(
            "size_bytes must be between 0 and {MAX_ATTACHMENT_BYTES}"
        )));
    }
    if let Some(sha) = &req.sha256 {
        validate::sha256(sha)?;
    }
    if let Some(thread_id) = req.thread_id {
        let in_workspace: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM threads WHERE id = $1 AND workspace_id = $2)",
        )
        .bind(thread_id)
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
        if !in_workspace {
            return Err(ApiError::BadRequest(
                "thread does not belong to this workspace".to_string(),
            ));
        }
    }

    let attachment_id = Uuid::new_v4();
    let storage_key = format!("workspaces/{id}/attachments/{attachment_id}");
    let sql = format!(
        "INSERT INTO attachments (id, workspace_id, thread_id, uploaded_by, filename, content_type, size_bytes, sha256, storage_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING {COLUMNS}"
    );

    let mut tx = state.pool.begin().await?;
    let view = sqlx::query_as::<_, AttachmentView>(&sql)
        .bind(attachment_id)
        .bind(id)
        .bind(req.thread_id)
        .bind(user.id)
        .bind(&req.filename)
        .bind(&req.content_type)
        .bind(req.size_bytes)
        .bind(req.sha256.as_deref())
        .bind(&storage_key)
        .fetch_one(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "attachment.created",
        "attachment",
        attachment_id,
        json!({ "filename": req.filename, "size_bytes": req.size_bytes }),
    )
    .await?;
    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(view)))
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<ListAttachmentsQuery>,
) -> Result<Json<Vec<AttachmentView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let sql = format!(
        "SELECT {COLUMNS} FROM attachments
         WHERE workspace_id = $1 AND ($2::uuid IS NULL OR thread_id = $2)
         ORDER BY created_at DESC
         LIMIT 500"
    );
    let rows = sqlx::query_as::<_, AttachmentView>(&sql)
        .bind(id)
        .bind(query.thread_id)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows))
}

/// The uploader can delete their own file; admins can delete any file.
async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let (workspace_id, uploaded_by, storage_key): (Uuid, Option<Uuid>, String) = sqlx::query_as(
        "SELECT workspace_id, uploaded_by, storage_key FROM attachments WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    let role = require_role(&state.pool, workspace_id, user.id, Role::Member).await?;
    if uploaded_by != Some(user.id) && role < Role::Admin {
        return Err(ApiError::Forbidden);
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM attachments WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        workspace_id,
        user.id,
        "attachment.deleted",
        "attachment",
        id,
        json!({}),
    )
    .await?;
    tx.commit().await?;

    // The row is gone, so the bytes are unreachable even if this fails. Log it for cleanup.
    if let Err(err) = state.storage.delete(&storage_key).await {
        tracing::warn!(%err, %storage_key, "could not delete attachment bytes");
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Stores the bytes for an attachment created with metadata. Only the uploader can upload,
/// once, and the size and optional SHA-256 must match the metadata.
async fn upload(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let (workspace_id, uploaded_by, size_bytes, storage_key, sha256, uploaded): (
        Uuid,
        Option<Uuid>,
        i64,
        String,
        Option<String>,
        bool,
    ) = sqlx::query_as(
        "SELECT workspace_id, uploaded_by, size_bytes, storage_key, sha256, uploaded FROM attachments WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    require_role(&state.pool, workspace_id, user.id, Role::Member).await?;
    if uploaded_by != Some(user.id) {
        return Err(ApiError::Forbidden);
    }
    if uploaded {
        return Err(ApiError::Conflict(
            "attachment already uploaded".to_string(),
        ));
    }
    if body.len() as i64 != size_bytes {
        return Err(ApiError::BadRequest(format!(
            "body must be {size_bytes} bytes"
        )));
    }
    if let Some(expected) = sha256 {
        let actual = hex::encode(Sha256::digest(&body));
        if actual != expected {
            return Err(ApiError::BadRequest(
                "body does not match the declared sha256".to_string(),
            ));
        }
    }

    state.storage.put(&storage_key, body).await.map_err(|err| {
        tracing::error!(%err, "could not store attachment");
        ApiError::Internal
    })?;
    sqlx::query("UPDATE attachments SET uploaded = true WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Returns the stored bytes to workspace members.
async fn download(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let (workspace_id, storage_key, content_type, filename, uploaded): (Uuid, String, String, String, bool) =
        sqlx::query_as(
            "SELECT workspace_id, storage_key, content_type, filename, uploaded FROM attachments WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    require_role(&state.pool, workspace_id, user.id, Role::Member).await?;
    if !uploaded {
        return Err(ApiError::NotFound);
    }
    let bytes = state.storage.get(&storage_key).await.map_err(|err| {
        tracing::error!(%err, "could not read attachment");
        ApiError::Internal
    })?;
    // Quotes and control characters would break the header, so drop them from the name.
    let safe_name: String = filename
        .chars()
        .filter(|c| *c != '"' && *c != '\\' && !c.is_control())
        .collect();
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{safe_name}\""),
            ),
        ],
        bytes,
    )
        .into_response())
}

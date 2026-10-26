use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgExecutor};
use uuid::Uuid;

use crate::{
    auth::CurrentUser,
    authz::{Role, require_role},
    error::ApiError,
    state::AppState,
    validate,
};

/// Appends one audit entry. Pass a transaction so the entry commits with the change it describes.
pub async fn record(
    exec: impl PgExecutor<'_>,
    workspace_id: Uuid,
    actor_id: Uuid,
    action: &str,
    target_type: &str,
    target_id: impl ToString,
    metadata: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_logs (workspace_id, actor_id, action, target_type, target_id, metadata)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(workspace_id)
    .bind(actor_id)
    .bind(action)
    .bind(target_type)
    .bind(target_id.to_string())
    .bind(metadata)
    .execute(exec)
    .await?;
    Ok(())
}

#[derive(Serialize, FromRow)]
pub struct AuditLogView {
    pub id: i64,
    pub actor_id: Option<Uuid>,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct ListAuditQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/workspaces/{id}/audit-logs", get(list))
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<ListAuditQuery>,
) -> Result<Json<Vec<AuditLogView>>, ApiError> {
    require_role(&state.pool, workspace_id, user.id, Role::Admin).await?;
    let (limit, offset) = validate::page(query.limit, query.offset)?;
    let rows = sqlx::query_as::<_, AuditLogView>(
        "SELECT id, actor_id, action, target_type, target_id, metadata, created_at
         FROM audit_logs
         WHERE workspace_id = $1
         ORDER BY created_at DESC, id DESC
         LIMIT $2 OFFSET $3",
    )
    .bind(workspace_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

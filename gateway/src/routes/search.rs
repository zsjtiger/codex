use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    auth::CurrentUser,
    authz::{Role, require_role},
    error::ApiError,
    state::AppState,
    validate,
};

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    limit: Option<i64>,
}

#[derive(Serialize, FromRow)]
pub struct ThreadHit {
    pub id: Uuid,
    pub title: String,
    pub last_activity_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, FromRow)]
pub struct TurnHit {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub thread_title: String,
    pub input: String,
}

#[derive(Serialize, FromRow)]
pub struct ArtifactHit {
    pub id: Uuid,
    pub title: String,
    pub kind: String,
}

#[derive(Serialize, FromRow)]
pub struct ProjectHit {
    pub id: Uuid,
    pub name: String,
}

#[derive(Serialize)]
pub struct SearchResults {
    pub threads: Vec<ThreadHit>,
    pub turns: Vec<TurnHit>,
    pub artifacts: Vec<ArtifactHit>,
    pub projects: Vec<ProjectHit>,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/workspaces/{id}/search", get(search))
}

/// Escapes LIKE wildcards so the query is matched literally.
fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// Case-insensitive substring search over chat titles, message text, artifacts and projects.
async fn search(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResults>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let q = query.q.trim();
    validate::length("q", q, 1, 200)?;
    let limit = query.limit.unwrap_or(20).clamp(1, 50);
    let pattern = like_pattern(q);
    let pool = &state.pool;

    let threads = sqlx::query_as::<_, ThreadHit>(
        "SELECT id, title, last_activity_at FROM threads
         WHERE workspace_id = $1 AND archived_at IS NULL AND title ILIKE $2
         ORDER BY last_activity_at DESC LIMIT $3",
    )
    .bind(id)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let turns = sqlx::query_as::<_, TurnHit>(
        "SELECT t.id, t.thread_id, th.title AS thread_title, t.input
         FROM turns t
         JOIN threads th ON th.id = t.thread_id
         WHERE th.workspace_id = $1 AND t.input ILIKE $2
         ORDER BY t.created_at DESC LIMIT $3",
    )
    .bind(id)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let artifacts = sqlx::query_as::<_, ArtifactHit>(
        "SELECT id, title, kind FROM artifacts
         WHERE workspace_id = $1 AND title ILIKE $2
         ORDER BY updated_at DESC LIMIT $3",
    )
    .bind(id)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let projects = sqlx::query_as::<_, ProjectHit>(
        "SELECT id, name FROM projects
         WHERE workspace_id = $1 AND name ILIKE $2
         ORDER BY updated_at DESC LIMIT $3",
    )
    .bind(id)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(Json(SearchResults {
        threads,
        turns,
        artifacts,
        projects,
    }))
}

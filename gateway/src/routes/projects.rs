use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, PgPool};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    audit,
    auth::CurrentUser,
    authz::{Role, ensure_creator_or_admin, require_role},
    error::ApiError,
    state::AppState,
    validate,
};

#[derive(Serialize, FromRow)]
pub struct ProjectView {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    /// Added to every thread in the project, through developer instructions.
    pub instructions: String,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct CreateProject {
    name: String,
    instructions: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateProject {
    name: Option<String>,
    instructions: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces/{id}/projects", get(list).post(create))
        .route("/projects/{id}", get(get_one).patch(update).delete(delete))
        .route("/threads/{id}/save-to-project", axum::routing::post(save_thread_to_project))
}

#[derive(Deserialize)]
pub struct SaveToProject {
    project_id: Uuid,
}

/// Saves a snapshot of a chat's messages as a Markdown file in a project. Later messages in the
/// chat are not added; saving again makes another file.
async fn save_thread_to_project(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(thread_id): Path<Uuid>,
    Json(req): Json<SaveToProject>,
) -> Result<(StatusCode, Json<crate::routes::attachments::AttachmentView>), ApiError> {
    let (workspace_id, title): (Uuid, String) =
        sqlx::query_as("SELECT workspace_id, title FROM threads WHERE id = $1")
            .bind(thread_id)
            .fetch_one(&state.pool)
            .await?;
    require_role(&state.pool, workspace_id, user.id, Role::Member).await?;
    let in_workspace: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
    )
    .bind(req.project_id)
    .bind(workspace_id)
    .fetch_one(&state.pool)
    .await?;
    if !in_workspace {
        return Err(ApiError::BadRequest(
            "project does not belong to this workspace".to_string(),
        ));
    }

    let turns: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT input, reply FROM turns WHERE thread_id = $1 ORDER BY seq")
            .bind(thread_id)
            .fetch_all(&state.pool)
            .await?;
    let mut markdown = format!("# {title}\n\n");
    for (input, reply) in turns {
        markdown.push_str(&format!("## User\n\n{input}\n\n"));
        if let Some(reply) = reply {
            markdown.push_str(&format!("## Assistant\n\n{reply}\n\n"));
        }
    }
    let text = markdown.clone();
    let bytes = markdown.into_bytes();
    if bytes.len() as i64 > crate::routes::attachments::MAX_ATTACHMENT_BYTES {
        return Err(ApiError::BadRequest("the chat is too long to save to a project".to_string()));
    }

    let filename: String = format!("{title}.md")
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '/') && !c.is_control())
        .collect();
    let attachment_id = Uuid::new_v4();
    let storage_key = format!("workspaces/{workspace_id}/attachments/{attachment_id}");
    let sha = hex::encode(Sha256::digest(&bytes));
    let size = bytes.len() as i64;
    state
        .storage
        .put(&storage_key, axum::body::Bytes::from(bytes))
        .await
        .map_err(|err| {
            tracing::error!(%err, "could not store saved chat");
            ApiError::Internal
        })?;
    let sql = format!(
        "INSERT INTO attachments (id, workspace_id, thread_id, project_id, uploaded_by, filename, content_type, size_bytes, sha256, storage_key, uploaded, extracted_text)
         VALUES ($1, $2, NULL, $3, $4, $5, 'text/markdown', $6, $7, $8, true, $9)
         RETURNING {}",
        crate::routes::attachments::COLUMNS
    );
    let view = sqlx::query_as::<_, crate::routes::attachments::AttachmentView>(&sql)
        .bind(attachment_id)
        .bind(workspace_id)
        .bind(req.project_id)
        .bind(user.id)
        .bind(&filename)
        .bind(size)
        .bind(&sha)
        .bind(&storage_key)
        .bind(&text)
        .fetch_one(&state.pool)
        .await?;
    Ok((StatusCode::CREATED, Json(view)))
}

const COLUMNS: &str = "id, workspace_id, name, instructions, created_by, created_at, updated_at";

pub async fn load(pool: &PgPool, id: Uuid) -> Result<ProjectView, ApiError> {
    let sql = format!("SELECT {COLUMNS} FROM projects WHERE id = $1");
    Ok(sqlx::query_as::<_, ProjectView>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await?)
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ProjectView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let sql = format!(
        "SELECT {COLUMNS} FROM projects WHERE workspace_id = $1 ORDER BY updated_at DESC LIMIT 200"
    );
    Ok(Json(
        sqlx::query_as::<_, ProjectView>(&sql)
            .bind(id)
            .fetch_all(&state.pool)
            .await?,
    ))
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateProject>,
) -> Result<(StatusCode, Json<ProjectView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    validate::length("name", &req.name, 1, 100)?;
    let instructions = req.instructions.unwrap_or_default();
    validate::length("instructions", &instructions, 0, 20_000)?;

    let mut tx = state.pool.begin().await?;
    let project_id: Uuid = sqlx::query_scalar(
        "INSERT INTO projects (workspace_id, name, instructions, created_by) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(id)
    .bind(&req.name)
    .bind(&instructions)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "project.created",
        "project",
        project_id,
        json!({ "name": req.name }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(load(&state.pool, project_id).await?),
    ))
}

async fn get_one(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ProjectView>, ApiError> {
    let project = load(&state.pool, id).await?;
    require_role(&state.pool, project.workspace_id, user.id, Role::Member).await?;
    Ok(Json(project))
}

async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProject>,
) -> Result<Json<ProjectView>, ApiError> {
    let project = load(&state.pool, id).await?;
    ensure_creator_or_admin(
        &state.pool,
        project.workspace_id,
        project.created_by,
        user.id,
    )
    .await?;
    if let Some(name) = &req.name {
        validate::length("name", name, 1, 100)?;
    }
    if let Some(instructions) = &req.instructions {
        validate::length("instructions", instructions, 0, 20_000)?;
    }
    sqlx::query(
        "UPDATE projects SET name = COALESCE($2, name), instructions = COALESCE($3, instructions), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.name.as_deref())
    .bind(req.instructions.as_deref())
    .execute(&state.pool)
    .await?;
    Ok(Json(load(&state.pool, id).await?))
}

/// Threads in the project stay; they just lose their project.
async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let project = load(&state.pool, id).await?;
    ensure_creator_or_admin(
        &state.pool,
        project.workspace_id,
        project.created_by,
        user.id,
    )
    .await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        project.workspace_id,
        user.id,
        "project.deleted",
        "project",
        id,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

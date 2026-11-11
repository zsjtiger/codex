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
use uuid::Uuid;

use crate::{
    audit,
    auth::CurrentUser,
    authz::{Role, membership, require_role},
    error::ApiError,
    state::AppState,
    validate,
};

#[derive(Serialize, FromRow)]
pub struct MemberView {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub joined_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct AddMember {
    email: String,
    /// `admin` or `member`. Defaults to `member`. Ownership cannot be assigned.
    role: Option<String>,
}

#[derive(Deserialize)]
pub struct ChangeRole {
    role: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces/{id}/members", get(list).post(add))
        .route("/workspaces/{id}/invites", axum::routing::post(create_invite))
        .route("/invites/{token}", get(preview_invite))
        .route("/invites/{token}/accept", axum::routing::post(accept_invite))
        .route(
            "/workspaces/{id}/members/{user_id}",
            axum::routing::patch(change_role).delete(remove),
        )
}

fn assignable_role(value: &str) -> Result<Role, ApiError> {
    match Role::parse(value) {
        Some(Role::Owner) | None => Err(ApiError::BadRequest(
            "role must be admin or member".to_string(),
        )),
        Some(role) => Ok(role),
    }
}

async fn get_member(
    pool: &PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<MemberView, ApiError> {
    let view = sqlx::query_as::<_, MemberView>(
        "SELECT u.id AS user_id, u.email, u.display_name, m.role, m.created_at AS joined_at
         FROM workspace_members m
         JOIN users u ON u.id = m.user_id
         WHERE m.workspace_id = $1 AND m.user_id = $2",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(view)
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<MemberView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let rows = sqlx::query_as::<_, MemberView>(
        "SELECT u.id AS user_id, u.email, u.display_name, m.role, m.created_at AS joined_at
         FROM workspace_members m
         JOIN users u ON u.id = m.user_id
         WHERE m.workspace_id = $1
         ORDER BY m.created_at",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows))
}

/// Adds an existing user by email. Users are created at sign-in, so there are no invitations yet.
async fn add(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<AddMember>,
) -> Result<(StatusCode, Json<MemberView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Admin).await?;
    let email = validate::email(&req.email)?;
    let role = assignable_role(req.role.as_deref().unwrap_or("member"))?;

    let mut tx = state.pool.begin().await?;
    let target: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ApiError::NotFound)?;
    sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(target)
        .bind(role.as_str())
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "member.added",
        "user",
        target,
        json!({ "role": role.as_str() }),
    )
    .await?;
    tx.commit().await?;

    let view = get_member(&state.pool, id, target).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

/// Only the owner changes roles. Ownership itself cannot be reassigned through the API.
async fn change_role(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, target)): Path<(Uuid, Uuid)>,
    Json(req): Json<ChangeRole>,
) -> Result<Json<MemberView>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Owner).await?;
    let new_role = assignable_role(&req.role)?;
    let current = membership(&state.pool, id, target)
        .await?
        .ok_or(ApiError::NotFound)?;
    if current == Role::Owner {
        return Err(ApiError::BadRequest(
            "the owner role cannot be changed".to_string(),
        ));
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE workspace_members SET role = $3 WHERE workspace_id = $1 AND user_id = $2")
        .bind(id)
        .bind(target)
        .bind(new_role.as_str())
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "member.role_changed",
        "user",
        target,
        json!({ "from": current.as_str(), "to": new_role.as_str() }),
    )
    .await?;
    tx.commit().await?;

    Ok(Json(get_member(&state.pool, id, target).await?))
}

/// Members may leave; admins may remove members; only the owner may remove admins.
async fn remove(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, target)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    if target == user.id {
        let own = membership(&state.pool, id, user.id)
            .await?
            .ok_or(ApiError::NotFound)?;
        if own == Role::Owner {
            return Err(ApiError::BadRequest(
                "the owner cannot leave; delete the workspace instead".to_string(),
            ));
        }
    } else {
        let caller = require_role(&state.pool, id, user.id, Role::Admin).await?;
        let current = membership(&state.pool, id, target)
            .await?
            .ok_or(ApiError::NotFound)?;
        if current == Role::Owner {
            return Err(ApiError::BadRequest(
                "the owner cannot be removed".to_string(),
            ));
        }
        if current == Role::Admin && caller != Role::Owner {
            return Err(ApiError::Forbidden);
        }
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM workspace_members WHERE workspace_id = $1 AND user_id = $2")
        .bind(id)
        .bind(target)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "member.removed",
        "user",
        target,
        json!({}),
    )
    .await?;
    tx.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateInvite {
    role: String,
}

#[derive(Serialize)]
pub struct InviteCreated {
    /// The token to put in the invite link. It is shown once; only its hash is stored.
    pub token: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
}

/// How long an invite link works.
const INVITE_DAYS: i64 = 7;

/// Creates a single-use invite link for an admin of the workspace.
async fn create_invite(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateInvite>,
) -> Result<(StatusCode, Json<InviteCreated>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Admin).await?;
    let role = assignable_role(&req.role)?;
    let token = crate::auth::new_token();
    let expires_at = Utc::now() + chrono::Duration::days(INVITE_DAYS);
    sqlx::query(
        "INSERT INTO workspace_invites (workspace_id, role, token_hash, created_by, expires_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(role.as_str())
    .bind(crate::auth::hash_token(&token))
    .bind(user.id)
    .bind(expires_at)
    .execute(&state.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(InviteCreated { token, role: req.role, expires_at })))
}

#[derive(Serialize)]
pub struct InvitePreview {
    pub workspace_name: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
}

/// The workspace and role an invite is for, while it can still be used.
async fn preview_invite(
    State(state): State<AppState>,
    _user: CurrentUser,
    Path(token): Path<String>,
) -> Result<Json<InvitePreview>, ApiError> {
    let row: Option<(String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT w.name, i.role, i.expires_at FROM workspace_invites i JOIN workspaces w ON w.id = i.workspace_id
         WHERE i.token_hash = $1 AND i.used_at IS NULL AND i.expires_at > now()",
    )
    .bind(crate::auth::hash_token(&token))
    .fetch_optional(&state.pool)
    .await?;
    let (workspace_name, role, expires_at) = row.ok_or(ApiError::NotFound)?;
    Ok(Json(InvitePreview { workspace_name, role, expires_at }))
}

#[derive(Serialize)]
pub struct InviteAccepted {
    pub workspace_id: Uuid,
}

/// Joins the signed-in user to the invite's workspace. The invite can be used once.
async fn accept_invite(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(token): Path<String>,
) -> Result<Json<InviteAccepted>, ApiError> {
    let mut tx = state.pool.begin().await?;
    let invite: Option<(Uuid, String)> = sqlx::query_as(
        "UPDATE workspace_invites SET used_at = now(), used_by = $2
         WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
         RETURNING workspace_id, role",
    )
    .bind(crate::auth::hash_token(&token))
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    let (workspace_id, role) = invite.ok_or(ApiError::NotFound)?;
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (workspace_id, user_id) DO NOTHING",
    )
    .bind(workspace_id)
    .bind(user.id)
    .bind(&role)
    .execute(&mut *tx)
    .await?;
    audit::record(&mut *tx, workspace_id, user.id, "member.added", "user", user.id, json!({ "via": "invite", "role": role })).await?;
    tx.commit().await?;
    Ok(Json(InviteAccepted { workspace_id }))
}

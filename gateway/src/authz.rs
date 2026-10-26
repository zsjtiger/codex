use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiError;

/// Workspace roles, ordered from least to most privileged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Member,
    Admin,
    Owner,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Admin => "admin",
            Self::Owner => "owner",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "member" => Some(Self::Member),
            "admin" => Some(Self::Admin),
            "owner" => Some(Self::Owner),
            _ => None,
        }
    }
}

pub async fn membership(
    pool: &PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
) -> Result<Option<Role>, ApiError> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
    )
    .bind(workspace_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(role.as_deref().and_then(Role::parse))
}

/// Requires at least `min`. Non-members get `NotFound`, so workspace existence stays hidden.
pub async fn require_role(
    pool: &PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    min: Role,
) -> Result<Role, ApiError> {
    let role = membership(pool, workspace_id, user_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if role >= min {
        Ok(role)
    } else {
        Err(ApiError::Forbidden)
    }
}

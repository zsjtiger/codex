use axum::{
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, request::Parts},
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{error::ApiError, state::AppState};

pub const SESSION_TTL_DAYS: i64 = 30;

/// The authenticated caller, resolved from `Authorization: Bearer <token>`.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub session_id: Uuid,
}

/// Opaque bearer token: 244 bits from two v4 UUIDs, prefixed for easy recognition.
pub fn new_token() -> String {
    format!("cdx_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Resolves a bearer token to a live session. Used by HTTP extraction and the WebSocket upgrade.
pub async fn authenticate(pool: &PgPool, token: &str) -> Result<CurrentUser, ApiError> {
    let (id, email, display_name, session_id): (Uuid, String, String, Uuid) = sqlx::query_as(
        "SELECT u.id, u.email, u.display_name, s.id
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now()",
    )
    .bind(hash_token(token))
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::Unauthorized)?;

    Ok(CurrentUser {
        id,
        email,
        display_name,
        session_id,
    })
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;
        authenticate(&state.pool, token).await
    }
}

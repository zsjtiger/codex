use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use tracing::error;

/// Errors returned by handlers. Serialized as `{"error": {"code", "message"}}`.
#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized,
    Forbidden,
    /// Also returned to non-members, so workspace existence is not revealed.
    NotFound,
    Conflict(String),
    Internal,
}

impl ApiError {
    fn parts(&self) -> (StatusCode, &'static str, String) {
        match self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m.clone()),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "authentication required".to_string(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "insufficient permissions".to_string(),
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "resource not found".to_string(),
            ),
            Self::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone()),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "internal error".to_string(),
            ),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.parts();
        (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        match &err {
            sqlx::Error::RowNotFound => Self::NotFound,
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                Self::Conflict("resource already exists".to_string())
            }
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
                Self::BadRequest("referenced resource does not exist".to_string())
            }
            _ => {
                error!(%err, "database error");
                Self::Internal
            }
        }
    }
}

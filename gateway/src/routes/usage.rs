use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::{
    auth::CurrentUser,
    authz::{Role, require_role},
    error::ApiError,
    state::AppState,
};

const DEFAULT_WINDOW_DAYS: i64 = 30;

#[derive(Deserialize)]
pub struct UsageQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

#[derive(Serialize, FromRow)]
pub struct UsageTotals {
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Serialize, FromRow)]
pub struct UsageDay {
    pub day: DateTime<Utc>,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Serialize)]
pub struct UsageReport {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub totals: UsageTotals,
    pub daily: Vec<UsageDay>,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/workspaces/{id}/usage", get(report))
}

/// Admin-only. Defaults to the last 30 days.
async fn report(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<UsageReport>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Admin).await?;
    let to = query.to.unwrap_or_else(Utc::now);
    let from = query
        .from
        .unwrap_or_else(|| to - TimeDelta::days(DEFAULT_WINDOW_DAYS));
    if from >= to {
        return Err(ApiError::BadRequest("from must be before to".to_string()));
    }

    let totals = sqlx::query_as::<_, UsageTotals>(
        "SELECT COALESCE(SUM(input_tokens), 0)::bigint AS input_tokens,
                COALESCE(SUM(output_tokens), 0)::bigint AS output_tokens
         FROM usage_records
         WHERE workspace_id = $1 AND recorded_at >= $2 AND recorded_at < $3",
    )
    .bind(id)
    .bind(from)
    .bind(to)
    .fetch_one(&state.pool)
    .await?;

    let daily = sqlx::query_as::<_, UsageDay>(
        "SELECT date_trunc('day', recorded_at) AS day,
                SUM(input_tokens)::bigint AS input_tokens,
                SUM(output_tokens)::bigint AS output_tokens
         FROM usage_records
         WHERE workspace_id = $1 AND recorded_at >= $2 AND recorded_at < $3
         GROUP BY 1
         ORDER BY 1",
    )
    .bind(id)
    .bind(from)
    .bind(to)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(UsageReport {
        from,
        to,
        totals,
        daily,
    }))
}

/// Records token usage for a finished turn. Called by the runtime, not over HTTP.
pub async fn record_usage(
    pool: &PgPool,
    workspace_id: Uuid,
    user_id: Option<Uuid>,
    thread_id: Option<Uuid>,
    turn_id: Option<Uuid>,
    input_tokens: i64,
    output_tokens: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO usage_records (workspace_id, user_id, thread_id, turn_id, input_tokens, output_tokens)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(workspace_id)
    .bind(user_id)
    .bind(thread_id)
    .bind(turn_id)
    .bind(input_tokens)
    .bind(output_tokens)
    .execute(pool)
    .await?;
    Ok(())
}

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::{
    audit,
    auth::CurrentUser,
    authz::{Role, ensure_creator_or_admin, require_role},
    error::ApiError,
    runtime::{
        dispatch,
        schedule::{self, DueTask, Frequency},
    },
    serde_helpers::double_option,
    state::AppState,
    validate,
};

#[derive(Serialize, FromRow)]
pub struct ScheduledTaskView {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub prompt: String,
    pub frequency: String,
    /// "HH:MM" in UTC.
    pub time_of_day: String,
    /// ISO weekday, 1 = Monday.
    pub weekday: Option<i16>,
    pub environment_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub enabled: bool,
    pub next_run_at: DateTime<Utc>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_thread_id: Option<Uuid>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct CreateScheduledTask {
    name: String,
    prompt: String,
    frequency: String,
    time_of_day: String,
    weekday: Option<i16>,
    environment_id: Option<Uuid>,
    project_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct UpdateScheduledTask {
    name: Option<String>,
    prompt: Option<String>,
    frequency: Option<String>,
    time_of_day: Option<String>,
    weekday: Option<i16>,
    enabled: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    environment_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "double_option")]
    project_id: Option<Option<Uuid>>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/workspaces/{id}/scheduled-tasks", get(list).post(create))
        .route(
            "/scheduled-tasks/{id}",
            get(get_one).patch(update).delete(delete),
        )
        .route("/scheduled-tasks/{id}/run", post(run_now))
}

const COLUMNS: &str = "id, workspace_id, name, prompt, frequency, time_of_day, weekday, environment_id, project_id, \
                       enabled, next_run_at, last_run_at, last_thread_id, created_by, created_at, updated_at";

async fn load(pool: &PgPool, id: Uuid) -> Result<ScheduledTaskView, ApiError> {
    let sql = format!("SELECT {COLUMNS} FROM scheduled_tasks WHERE id = $1");
    Ok(sqlx::query_as::<_, ScheduledTaskView>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await?)
}

/// Checks a frequency, its "HH:MM" time and its weekday together.
fn check_schedule(
    frequency: &str,
    time_of_day: &str,
    weekday: Option<i16>,
) -> Result<(Frequency, NaiveTime), ApiError> {
    let parsed = Frequency::parse(frequency).ok_or_else(|| {
        ApiError::BadRequest("frequency must be hourly, daily, weekdays or weekly".to_string())
    })?;
    let time = NaiveTime::parse_from_str(time_of_day, "%H:%M")
        .map_err(|_| ApiError::BadRequest("time_of_day must be HH:MM in UTC".to_string()))?;
    match (parsed, weekday) {
        (Frequency::Weekly, None) => Err(ApiError::BadRequest(
            "weekly tasks need a weekday".to_string(),
        )),
        (Frequency::Weekly, Some(day)) if !(1..=7).contains(&day) => Err(ApiError::BadRequest(
            "weekday must be 1 (Monday) to 7 (Sunday)".to_string(),
        )),
        _ => Ok((parsed, time)),
    }
}

async fn ensure_in_workspace(
    pool: &PgPool,
    table: &str,
    id: Uuid,
    workspace_id: Uuid,
) -> Result<(), ApiError> {
    let sql = format!("SELECT EXISTS (SELECT 1 FROM {table} WHERE id = $1 AND workspace_id = $2)");
    let found: bool = sqlx::query_scalar(&sql)
        .bind(id)
        .bind(workspace_id)
        .fetch_one(pool)
        .await?;
    if found {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "{table} does not belong to this workspace"
        )))
    }
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ScheduledTaskView>>, ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    let sql = format!(
        "SELECT {COLUMNS} FROM scheduled_tasks WHERE workspace_id = $1 ORDER BY created_at DESC"
    );
    Ok(Json(
        sqlx::query_as::<_, ScheduledTaskView>(&sql)
            .bind(id)
            .fetch_all(&state.pool)
            .await?,
    ))
}

async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateScheduledTask>,
) -> Result<(StatusCode, Json<ScheduledTaskView>), ApiError> {
    require_role(&state.pool, id, user.id, Role::Member).await?;
    validate::length("name", &req.name, 1, 100)?;
    validate::length("prompt", &req.prompt, 1, 100_000)?;
    let (frequency, time) = check_schedule(&req.frequency, &req.time_of_day, req.weekday)?;
    if let Some(env) = req.environment_id {
        ensure_in_workspace(&state.pool, "environments", env, id).await?;
    }
    if let Some(project) = req.project_id {
        ensure_in_workspace(&state.pool, "projects", project, id).await?;
    }
    let weekday = if frequency == Frequency::Weekly {
        req.weekday
    } else {
        None
    };
    let next = schedule::next_run(frequency, time, weekday.map(|d| d as u8), Utc::now());

    let mut tx = state.pool.begin().await?;
    let task_id: Uuid = sqlx::query_scalar(
        "INSERT INTO scheduled_tasks (workspace_id, name, prompt, frequency, time_of_day, weekday, environment_id,
                                      project_id, next_run_at, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(id)
    .bind(&req.name)
    .bind(&req.prompt)
    .bind(&req.frequency)
    .bind(&req.time_of_day)
    .bind(weekday)
    .bind(req.environment_id)
    .bind(req.project_id)
    .bind(next)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        id,
        user.id,
        "scheduled_task.created",
        "scheduled_task",
        task_id,
        json!({ "name": req.name }),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(load(&state.pool, task_id).await?)))
}

async fn get_one(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ScheduledTaskView>, ApiError> {
    let task = load(&state.pool, id).await?;
    require_role(&state.pool, task.workspace_id, user.id, Role::Member).await?;
    Ok(Json(task))
}

/// Changing the schedule recomputes the next run from now. Re-enabling does the same.
async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateScheduledTask>,
) -> Result<Json<ScheduledTaskView>, ApiError> {
    let task = load(&state.pool, id).await?;
    ensure_creator_or_admin(&state.pool, task.workspace_id, task.created_by, user.id).await?;
    if let Some(name) = &req.name {
        validate::length("name", name, 1, 100)?;
    }
    if let Some(prompt) = &req.prompt {
        validate::length("prompt", prompt, 1, 100_000)?;
    }
    let frequency = req.frequency.clone().unwrap_or(task.frequency.clone());
    let time_of_day = req.time_of_day.clone().unwrap_or(task.time_of_day.clone());
    let weekday = req.weekday.or(task.weekday);
    let (parsed, time) = check_schedule(&frequency, &time_of_day, weekday)?;
    let weekday = if parsed == Frequency::Weekly {
        weekday
    } else {
        None
    };
    let (set_env, environment_id) = match req.environment_id {
        None => (false, None),
        Some(None) => (true, None),
        Some(Some(env)) => {
            ensure_in_workspace(&state.pool, "environments", env, task.workspace_id).await?;
            (true, Some(env))
        }
    };
    let (set_project, project_id) = match req.project_id {
        None => (false, None),
        Some(None) => (true, None),
        Some(Some(project)) => {
            ensure_in_workspace(&state.pool, "projects", project, task.workspace_id).await?;
            (true, Some(project))
        }
    };
    let enabled = req.enabled.unwrap_or(task.enabled);
    let reschedule = req.frequency.is_some()
        || req.time_of_day.is_some()
        || req.weekday.is_some()
        || (enabled && !task.enabled);
    let next = schedule::next_run(parsed, time, weekday.map(|d| d as u8), Utc::now());

    sqlx::query(
        "UPDATE scheduled_tasks
         SET name = COALESCE($2, name),
             prompt = COALESCE($3, prompt),
             frequency = $4,
             time_of_day = $5,
             weekday = $6,
             enabled = $7,
             environment_id = CASE WHEN $8 THEN $9 ELSE environment_id END,
             project_id = CASE WHEN $10 THEN $11 ELSE project_id END,
             next_run_at = CASE WHEN $12 THEN $13 ELSE next_run_at END,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.name.as_deref())
    .bind(req.prompt.as_deref())
    .bind(&frequency)
    .bind(&time_of_day)
    .bind(weekday)
    .bind(enabled)
    .bind(set_env)
    .bind(environment_id)
    .bind(set_project)
    .bind(project_id)
    .bind(reschedule)
    .bind(next)
    .execute(&state.pool)
    .await?;
    Ok(Json(load(&state.pool, id).await?))
}

async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let task = load(&state.pool, id).await?;
    ensure_creator_or_admin(&state.pool, task.workspace_id, task.created_by, user.id).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM scheduled_tasks WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        task.workspace_id,
        user.id,
        "scheduled_task.deleted",
        "scheduled_task",
        id,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Starts a run now. The schedule itself is unchanged.
async fn run_now(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let task = load(&state.pool, id).await?;
    ensure_creator_or_admin(&state.pool, task.workspace_id, task.created_by, user.id).await?;
    let due = DueTask {
        id,
        workspace_id: task.workspace_id,
        name: task.name.clone(),
        prompt: task.prompt.clone(),
        frequency: task.frequency.clone(),
        time_of_day: task.time_of_day.clone(),
        weekday: task.weekday,
        environment_id: task.environment_id,
        project_id: task.project_id,
        created_by: Some(user.id),
    };
    let mut tx = state.pool.begin().await?;
    let thread_id = schedule::start_run(&mut tx, task.workspace_id, &due).await?;
    tx.commit().await?;
    dispatch::spawn_drain(state.clone(), thread_id);
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "thread_id": thread_id })),
    ))
}

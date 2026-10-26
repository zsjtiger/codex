//! Scheduled tasks: computing when a task is next due, and starting its runs.

use std::time::Duration;

use chrono::{DateTime, Datelike, NaiveTime, Timelike, Utc, Weekday};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::state::AppState;

const TICK: Duration = Duration::from_secs(30);
const MAX_TASKS_PER_TICK: i64 = 50;

/// How often a task runs. Times are UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frequency {
    Hourly,
    Daily,
    Weekdays,
    Weekly,
}

impl Frequency {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hourly" => Some(Self::Hourly),
            "daily" => Some(Self::Daily),
            "weekdays" => Some(Self::Weekdays),
            "weekly" => Some(Self::Weekly),
            _ => None,
        }
    }
}

/// The first run time strictly after `after`.
pub fn next_run(
    frequency: Frequency,
    time: NaiveTime,
    weekday: Option<u8>,
    after: DateTime<Utc>,
) -> DateTime<Utc> {
    match frequency {
        Frequency::Hourly => {
            let this_hour = after
                .with_minute(time.minute())
                .and_then(|t| t.with_second(0))
                .and_then(|t| t.with_nanosecond(0))
                .unwrap_or(after);
            if this_hour > after {
                this_hour
            } else {
                this_hour + chrono::TimeDelta::hours(1)
            }
        }
        Frequency::Daily | Frequency::Weekdays | Frequency::Weekly => {
            let wanted = weekday.and_then(|d| Weekday::try_from(d.saturating_sub(1)).ok());
            let mut candidate = after.date_naive().and_time(time).and_utc();
            if candidate <= after {
                candidate += chrono::TimeDelta::days(1);
            }
            // Step forward day by day until the day matches the frequency. Seven steps
            // always reach a match because every weekday recurs within a week.
            for _ in 0..7 {
                let matches = match frequency {
                    Frequency::Daily => true,
                    Frequency::Weekdays => {
                        !matches!(candidate.weekday(), Weekday::Sat | Weekday::Sun)
                    }
                    Frequency::Weekly => Some(candidate.weekday()) == wanted,
                    Frequency::Hourly => unreachable!(),
                };
                if matches {
                    break;
                }
                candidate += chrono::TimeDelta::days(1);
            }
            candidate
        }
    }
}

/// Starts a run of a task inside `tx`: creates its thread and first turn. Returns the thread id.
pub async fn start_run(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: Uuid,
    task: &DueTask,
) -> Result<Uuid, sqlx::Error> {
    let environment_id =
        match task.environment_id {
            Some(id) => Some(id),
            None => sqlx::query_scalar(
                "SELECT id FROM environments WHERE workspace_id = $1 ORDER BY created_at LIMIT 1",
            )
            .bind(workspace_id)
            .fetch_optional(&mut **tx)
            .await?,
        };
    let thread_id: Uuid = sqlx::query_scalar(
        "INSERT INTO threads (workspace_id, environment_id, project_id, created_by, title)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(workspace_id)
    .bind(environment_id)
    .bind(task.project_id)
    .bind(task.created_by)
    .bind(&task.name)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query("INSERT INTO turns (thread_id, seq, input, created_by) VALUES ($1, 1, $2, $3)")
        .bind(thread_id)
        .bind(&task.prompt)
        .bind(task.created_by)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "UPDATE scheduled_tasks SET last_run_at = now(), last_thread_id = $2 WHERE id = $1",
    )
    .bind(task.id)
    .bind(thread_id)
    .execute(&mut **tx)
    .await?;
    Ok(thread_id)
}

#[derive(Debug, sqlx::FromRow)]
pub struct DueTask {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub prompt: String,
    pub frequency: String,
    pub time_of_day: String,
    pub weekday: Option<i16>,
    pub environment_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub created_by: Option<Uuid>,
}

/// Starts every due task. Rows are locked with SKIP LOCKED so two gateways never start the same run.
pub async fn run_due(state: &AppState) -> Result<usize, sqlx::Error> {
    let mut tx = state.pool.begin().await?;
    let due: Vec<DueTask> = sqlx::query_as(
        "SELECT id, workspace_id, name, prompt, frequency, time_of_day, weekday, environment_id, project_id, created_by
         FROM scheduled_tasks
         WHERE enabled AND next_run_at <= now()
         ORDER BY next_run_at
         LIMIT $1
         FOR UPDATE SKIP LOCKED",
    )
    .bind(MAX_TASKS_PER_TICK)
    .fetch_all(&mut *tx)
    .await?;

    let mut started = Vec::with_capacity(due.len());
    for task in &due {
        let frequency =
            Frequency::parse(&task.frequency).expect("database enforces frequency values");
        let time = NaiveTime::parse_from_str(&task.time_of_day, "%H:%M")
            .expect("database enforces time format");
        let next = next_run(frequency, time, task.weekday.map(|d| d as u8), Utc::now());
        sqlx::query(
            "UPDATE scheduled_tasks SET next_run_at = $2, updated_at = now() WHERE id = $1",
        )
        .bind(task.id)
        .bind(next)
        .execute(&mut *tx)
        .await?;
        started.push(start_run(&mut tx, task.workspace_id, task).await?);
    }
    tx.commit().await?;

    for thread_id in &started {
        crate::runtime::dispatch::spawn_drain(state.clone(), *thread_id);
    }
    Ok(started.len())
}

/// Runs the scheduler for the life of the process.
pub fn spawn_scheduler(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            match run_due(&state).await {
                Ok(0) => {}
                Ok(count) => tracing::info!(count, "started scheduled runs"),
                Err(err) => tracing::error!(%err, "scheduler tick failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn daily_runs_today_or_tomorrow() {
        let eight = NaiveTime::from_hms_opt(8, 0, 0).unwrap();
        assert_eq!(
            next_run(Frequency::Daily, eight, None, at(2026, 10, 26, 7, 0)),
            at(2026, 10, 26, 8, 0)
        );
        assert_eq!(
            next_run(Frequency::Daily, eight, None, at(2026, 10, 26, 8, 0)),
            at(2026, 10, 27, 8, 0)
        );
    }

    #[test]
    fn weekdays_skip_the_weekend() {
        let eight = NaiveTime::from_hms_opt(8, 0, 0).unwrap();
        // 2026-10-23 is a Friday; after Friday 08:00 the next run is Monday 26th.
        assert_eq!(
            next_run(Frequency::Weekdays, eight, None, at(2026, 10, 23, 9, 0)),
            at(2026, 10, 26, 8, 0)
        );
    }

    #[test]
    fn weekly_runs_on_the_given_iso_weekday() {
        let four_pm = NaiveTime::from_hms_opt(16, 0, 0).unwrap();
        // Friday is ISO weekday 5.
        assert_eq!(
            next_run(Frequency::Weekly, four_pm, Some(5), at(2026, 10, 26, 9, 0)),
            at(2026, 10, 30, 16, 0)
        );
    }

    #[test]
    fn hourly_runs_at_the_given_minute() {
        let quarter_past = NaiveTime::from_hms_opt(0, 15, 0).unwrap();
        assert_eq!(
            next_run(
                Frequency::Hourly,
                quarter_past,
                None,
                at(2026, 10, 26, 9, 5)
            ),
            at(2026, 10, 26, 9, 15)
        );
        assert_eq!(
            next_run(
                Frequency::Hourly,
                quarter_past,
                None,
                at(2026, 10, 26, 9, 15)
            ),
            at(2026, 10, 26, 10, 15)
        );
    }
}

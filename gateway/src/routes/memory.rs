//! The signed-in user's saved memory: notes that are added to their new turns when memory is on.

use std::{fs, path::Path as FsPath, process::Stdio};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{auth::CurrentUser, error::ApiError, state::AppState, validate};

/// Most memory notes kept per user.
const MAX_NOTES: i64 = 200;
/// Largest import accepted, in bytes (a zip export or pasted text).
const IMPORT_LIMIT_BYTES: usize = 10 * 1024 * 1024;
/// Most text read out of an imported zip.
const MAX_ZIP_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// Most text files read out of an imported zip.
const MAX_ZIP_FILES: usize = 50;

#[derive(Serialize)]
pub struct ImportResult {
    pub added: usize,
    /// Notes already saved, so they were not added again.
    pub duplicates: usize,
    /// Notes left out because the memory is full.
    pub skipped: usize,
}

/// Adds notes from pasted text, a Markdown or text file, or a zip of them (such as an export).
/// Bullet points become one note each; without bullets, each paragraph becomes a note.
async fn import_notes(
    State(state): State<AppState>,
    user: CurrentUser,
    body: Bytes,
) -> Result<Json<ImportResult>, ApiError> {
    // Each file is read on its own, so a bullet list in one file does not hide paragraphs in another.
    let texts = if body.starts_with(b"PK\x03\x04") {
        read_zip(&body).await?
    } else {
        vec![String::from_utf8(body.to_vec()).map_err(|_| ApiError::BadRequest("the file must be UTF-8 text or a zip".to_string()))?]
    };
    let candidates: Vec<String> = texts.iter().flat_map(|text| parse_notes(text)).collect();
    if candidates.is_empty() {
        return Err(ApiError::BadRequest("no notes found in the import".to_string()));
    }

    let existing: Vec<String> = sqlx::query_scalar("SELECT content FROM user_memories WHERE user_id = $1")
        .bind(user.id)
        .fetch_all(&state.pool)
        .await?;
    let mut seen: std::collections::HashSet<String> = existing.iter().map(|c| c.to_lowercase()).collect();
    let mut room = (MAX_NOTES as usize).saturating_sub(existing.len());
    let (mut added, mut duplicates, mut skipped) = (0, 0, 0);
    let mut tx = state.pool.begin().await?;
    for note in candidates {
        if !seen.insert(note.to_lowercase()) {
            duplicates += 1;
            continue;
        }
        if room == 0 {
            skipped += 1;
            continue;
        }
        sqlx::query("INSERT INTO user_memories (user_id, content) VALUES ($1, $2)")
            .bind(user.id)
            .bind(&note)
            .execute(&mut *tx)
            .await?;
        room -= 1;
        added += 1;
    }
    tx.commit().await?;
    Ok(Json(ImportResult { added, duplicates, skipped }))
}

/// The text of each Markdown and text file in a zip.
async fn read_zip(bytes: &[u8]) -> Result<Vec<String>, ApiError> {
    let dir = std::env::temp_dir().join("codex-memory-import").join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).map_err(|_| ApiError::Internal)?;
    let archive = dir.join("import.zip");
    let result = async {
        fs::write(&archive, bytes).map_err(|_| ApiError::Internal)?;
        let output = tokio::process::Command::new("unzip")
            .arg("-q")
            .arg("-o")
            .arg(&archive)
            .arg("-d")
            .arg(dir.join("files"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await
            .map_err(|_| ApiError::Internal)?;
        if !output.status.success() {
            return Err(ApiError::BadRequest("the zip file could not be read".to_string()));
        }
        let mut paths = Vec::new();
        collect_text_files(&dir.join("files"), &mut paths, 0);
        paths.sort();
        let mut texts = Vec::new();
        let mut total = 0;
        for path in paths.iter().take(MAX_ZIP_FILES) {
            if total >= MAX_ZIP_TEXT_BYTES {
                break;
            }
            if let Ok(content) = fs::read_to_string(path) {
                total += content.len();
                texts.push(content);
            }
        }
        Ok(texts)
    }
    .await;
    let _ = fs::remove_dir_all(&dir);
    result
}

fn collect_text_files(dir: &FsPath, out: &mut Vec<std::path::PathBuf>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_text_files(&path, out, depth + 1);
        } else if matches!(path.extension().and_then(|e| e.to_str()), Some("md" | "markdown" | "txt")) {
            out.push(path);
        }
    }
}

/// Notes from text: bullet points first, otherwise paragraphs. Headings are skipped.
pub fn parse_notes(text: &str) -> Vec<String> {
    let mut bullets = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let item = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("• "))
            .or_else(|| {
                let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
                (digits > 0).then(|| trimmed[digits..].strip_prefix(". ")).flatten()
            });
        if let Some(item) = item {
            bullets.push(item.to_string());
        }
    }
    let raw = if !bullets.is_empty() {
        bullets
    } else {
        let mut paragraphs = Vec::new();
        let mut current = String::new();
        for line in text.lines().chain(std::iter::once("")) {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                if !current.is_empty() {
                    paragraphs.push(std::mem::take(&mut current));
                }
            } else {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(trimmed);
            }
        }
        paragraphs
    };
    raw.into_iter()
        .map(|note| note.trim().trim_matches('*').trim().to_string())
        .filter(|note| !note.is_empty())
        .map(|note| note.chars().take(2000).collect::<String>())
        .collect()
}

#[derive(Serialize, FromRow)]
pub struct MemoryNote {
    pub id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct MemoryView {
    pub enabled: bool,
    pub notes: Vec<MemoryNote>,
}

#[derive(Deserialize)]
pub struct NoteInput {
    content: String,
}

#[derive(Deserialize)]
pub struct EnabledInput {
    enabled: bool,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/memory", get(get_memory).post(add_note))
        .route("/me/memory/enabled", put(set_enabled))
        .route(
            "/me/memory/import",
            post(import_notes).layer(DefaultBodyLimit::max(IMPORT_LIMIT_BYTES)),
        )
        .route("/me/memory/{id}", axum::routing::patch(edit_note).delete(delete_note))
}

async fn get_memory(State(state): State<AppState>, user: CurrentUser) -> Result<Json<MemoryView>, ApiError> {
    let enabled = memory_enabled(&state, user.id).await?;
    let notes = sqlx::query_as::<_, MemoryNote>(
        "SELECT id, content, created_at, updated_at FROM user_memories WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(MemoryView { enabled, notes }))
}

async fn add_note(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(req): Json<NoteInput>,
) -> Result<(StatusCode, Json<MemoryNote>), ApiError> {
    let content = clean(&req.content)?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_memories WHERE user_id = $1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;
    if count >= MAX_NOTES {
        return Err(ApiError::BadRequest(format!("memory is limited to {MAX_NOTES} notes")));
    }
    let note = sqlx::query_as::<_, MemoryNote>(
        "INSERT INTO user_memories (user_id, content) VALUES ($1, $2)
         RETURNING id, content, created_at, updated_at",
    )
    .bind(user.id)
    .bind(content)
    .fetch_one(&state.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(note)))
}

async fn edit_note(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Json(req): Json<NoteInput>,
) -> Result<Json<MemoryNote>, ApiError> {
    let content = clean(&req.content)?;
    let note = sqlx::query_as::<_, MemoryNote>(
        "UPDATE user_memories SET content = $3, updated_at = now()
         WHERE id = $1 AND user_id = $2
         RETURNING id, content, created_at, updated_at",
    )
    .bind(id)
    .bind(user.id)
    .bind(content)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(note))
}

async fn delete_note(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    let result = sqlx::query("DELETE FROM user_memories WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn set_enabled(State(state): State<AppState>, user: CurrentUser, Json(req): Json<EnabledInput>) -> Result<StatusCode, ApiError> {
    sqlx::query(
        "INSERT INTO user_settings (user_id, memory_enabled) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET memory_enabled = EXCLUDED.memory_enabled, updated_at = now()",
    )
    .bind(user.id)
    .bind(req.enabled)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Whether the user's saved memory is used in their chats. On by default.
pub async fn memory_enabled(state: &AppState, user_id: Uuid) -> Result<bool, ApiError> {
    let value: Option<bool> = sqlx::query_scalar("SELECT memory_enabled FROM user_settings WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await?;
    Ok(value.unwrap_or(true))
}

fn clean(content: &str) -> Result<String, ApiError> {
    let trimmed = content.trim();
    validate::length("memory", trimmed, 1, 2000)?;
    Ok(trimmed.to_string())
}

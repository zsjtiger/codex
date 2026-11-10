//! Files the agent writes for the user. A local turn is told to save deliverables in its own
//! output folder; when the turn ends the gateway stores those files as attachments of the turn.
//! Only that folder is read, and only files with a known document type and a size limit.

use std::{
    fs,
    path::{Path, PathBuf},
};

use axum::body::Bytes;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::state::AppState;

/// Most files kept from one turn.
const MAX_OUTPUTS: usize = 10;
/// Largest output file kept, in bytes.
const MAX_OUTPUT_BYTES: u64 = 20 * 1024 * 1024;

/// File types the gateway keeps, with the content type they are served as.
const KINDS: &[(&str, &str)] = &[
    ("pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
    ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    ("pdf", "application/pdf"),
    ("md", "text/markdown"),
    ("csv", "text/csv"),
    ("txt", "text/plain"),
    ("png", "image/png"),
];

/// The folder a turn's output files are written to.
pub fn output_dir(turn_id: Uuid) -> PathBuf {
    std::env::temp_dir().join("codex-outputs").join(turn_id.to_string())
}

/// Tells the agent where to save files for the user. Sent with every local turn.
pub fn prompt_note(dir: &Path) -> String {
    format!(
        "If you create a file the user should receive (for example pptx, docx, xlsx, pdf, md, csv, txt or png), \
         save it in the folder {} and name it in your reply. Do not save anything else there.",
        dir.display()
    )
}

/// Stores the files found in `dir` as attachments of the turn and removes the folder.
/// Problems with single files are logged and skipped so the turn itself still completes.
pub async fn collect(state: &AppState, workspace_id: Uuid, thread_id: Uuid, turn_id: Uuid, dir: &Path) -> Result<(), sqlx::Error> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    let mut ids = Vec::new();
    for entry in entries.flatten().take(MAX_OUTPUTS * 4) {
        if ids.len() >= MAX_OUTPUTS {
            break;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let Some((_, content_type)) = KINDS.iter().find(|(known, _)| *known == ext) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() || meta.len() > MAX_OUTPUT_BYTES || name.contains(['"', '\\', '/']) {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        let attachment_id = store(state, workspace_id, thread_id, &name, content_type, bytes).await;
        if let Some(id) = attachment_id {
            ids.push(id);
        }
    }
    if !ids.is_empty() {
        sqlx::query("UPDATE turns SET output_attachment_ids = $2 WHERE id = $1")
            .bind(turn_id)
            .bind(&ids)
            .execute(&state.pool)
            .await?;
    }
    let _ = fs::remove_dir_all(dir);
    Ok(())
}

async fn store(
    state: &AppState,
    workspace_id: Uuid,
    thread_id: Uuid,
    name: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> Option<Uuid> {
    let id = Uuid::new_v4();
    let key = format!("workspaces/{workspace_id}/attachments/{id}");
    let sha = hex::encode(Sha256::digest(&bytes));
    let size = bytes.len() as i64;
    if let Err(err) = state.storage.put(&key, Bytes::from(bytes)).await {
        tracing::warn!(%err, "could not store output file");
        return None;
    }
    let inserted = sqlx::query(
        "INSERT INTO attachments (id, workspace_id, thread_id, uploaded_by, filename, content_type, size_bytes, sha256, storage_key, uploaded)
         VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8, true)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(thread_id)
    .bind(name)
    .bind(content_type)
    .bind(size)
    .bind(&sha)
    .bind(&key)
    .execute(&state.pool)
    .await;
    match inserted {
        Ok(_) => Some(id),
        Err(err) => {
            tracing::warn!(%err, "could not record output file");
            None
        }
    }
}

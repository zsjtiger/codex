//! Reads the text out of a project file so chats can use it and search can find it.
//! PDFs go through `pdftotext` and Word files through `unzip`, both run on a temporary copy.

use std::{path::PathBuf, time::Duration};

use tokio::process::Command;
use uuid::Uuid;

/// Most text kept for one file, in bytes.
pub const MAX_EXTRACTED_BYTES: usize = 2 * 1024 * 1024;

const TIMEOUT: Duration = Duration::from_secs(60);

/// The text of a file, or `None` when the type is not one this module reads.
pub async fn extract(filename: &str, content_type: &str, bytes: &[u8]) -> Option<String> {
    let name = filename.to_ascii_lowercase();
    let text = if content_type == "application/pdf" || name.ends_with(".pdf") {
        run_on_copy(bytes, "pdf", |path| {
            let mut command = Command::new("pdftotext");
            command.arg("-enc").arg("UTF-8").arg(path).arg("-");
            command
        })
        .await?
    } else if content_type.contains("presentationml") || name.ends_with(".pptx") {
        // Slide text lives in DrawingML paragraphs (`a:p`) inside each slide part.
        let xml = run_on_copy(bytes, "pptx", |path| {
            let mut command = Command::new("unzip");
            command.arg("-p").arg(path).arg("ppt/slides/*.xml");
            command
        })
        .await?;
        strip_xml(&xml)
    } else if content_type.contains("spreadsheetml") || name.ends_with(".xlsx") {
        // Shared strings hold the text of every text cell, one `si` item each.
        let xml = run_on_copy(bytes, "xlsx", |path| {
            let mut command = Command::new("unzip");
            command.arg("-p").arg(path).arg("xl/sharedStrings.xml");
            command
        })
        .await?;
        strip_xml(&xml)
    } else if content_type.contains("wordprocessingml") || name.ends_with(".docx") {
        let xml = run_on_copy(bytes, "docx", |path| {
            let mut command = Command::new("unzip");
            command.arg("-p").arg(path).arg("word/document.xml");
            command
        })
        .await?;
        strip_xml(&xml)
    } else if content_type.starts_with("text/") || content_type == "application/json" {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        return None;
    };
    Some(truncate(text))
}

/// Writes the bytes to a fresh folder, runs the command on the copy, and removes the folder.
async fn run_on_copy(bytes: &[u8], ext: &str, build: impl FnOnce(&PathBuf) -> Command) -> Option<String> {
    let dir = std::env::temp_dir().join("codex-extract").join(Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir).ok()?;
    let input = dir.join(format!("input.{ext}"));
    let result = async {
        std::fs::write(&input, bytes).ok()?;
        let mut command = build(&input);
        let output = tokio::time::timeout(TIMEOUT, command.output()).await.ok()?.ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }
    .await;
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Office XML (Word, PowerPoint, Excel) with paragraphs on their own lines and the tags removed.
fn strip_xml(xml: &str) -> String {
    let with_breaks = xml
        .replace("</w:p>", "\n")
        .replace("</a:p>", "\n")
        .replace("</si>", "\n")
        .replace("<w:tab/>", "\t");
    let mut out = String::with_capacity(with_breaks.len());
    let mut in_tag = false;
    for ch in with_breaks.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn truncate(mut text: String) -> String {
    if text.len() > MAX_EXTRACTED_BYTES {
        let mut end = MAX_EXTRACTED_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// The part of `text` around the first match of `query`, ignoring case. `None` when there is no match.
pub fn snippet(text: &str, query: &str) -> Option<String> {
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    if needle.is_empty() {
        return None;
    }
    let hay: Vec<char> = text.chars().collect();
    let folded: Vec<char> = hay.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let position = folded.windows(needle.len()).position(|window| window == needle.as_slice())?;
    let start = position.saturating_sub(60);
    let end = (position + needle.len() + 60).min(hay.len());
    let excerpt: String = hay[start..end].iter().collect();
    Some(excerpt.replace('\n', " ").trim().to_string())
}

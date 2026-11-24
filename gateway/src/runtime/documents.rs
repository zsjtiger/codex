//! Documents a chat holds, and the documents the model makes or changes.
//!
//! Each turn in a chat gets the chat's documents in front of the message, so the model can see
//! them, and the way to write documents. To change one, the model replies with the document's
//! complete new text in a block that starts with four backticks and `document-edit id=<document
//! id>`. To save text as a new document, it replies with a block that starts with four backticks and
//! `document-create title="<title>"`, holding the text to save. When the turn completes, the gateway
//! applies these blocks to the chat. Blocks that name a document of another chat, or that do not
//! parse, change nothing.

use sqlx::PgPool;
use uuid::Uuid;

/// Documents shown to the model for one turn, in characters. Documents past this are left out.
const CONTEXT_BUDGET: usize = 200_000;

/// The instructions for documents, and the chat's documents. Every turn gets the instructions, so the
/// model can tell when a request is for a document.
pub(crate) async fn document_context(pool: &PgPool, thread_id: Uuid) -> Result<Option<String>, sqlx::Error> {
    let docs: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, title, content FROM artifacts
         WHERE thread_id = $1 AND kind = 'document' ORDER BY created_at, id",
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await?;
    let mut text = String::from(
        "Documents. When the request is to save text as a document (for example \"save the text above as a \
         doc\"), reply with a block that starts with four backticks followed by `document-create title=\"<title>\"` \
         and ends with four backticks, holding the text to save, copied exactly. To save the previous reply as \
         it is, leave the block empty. Say briefly that it is saved. Do not use that block for anything else.\n",
    );
    if docs.is_empty() {
        return Ok(Some(text));
    }
    text.push_str(
        "\nDocuments in this chat. You may change one. To do so, reply with its complete new text in a block \
         that starts with four backticks followed by `document-edit id=<id>` and ends with four backticks. \
         Put nothing else in the block, and say briefly in your reply what you changed. Change only the \
         documents listed here.\n",
    );
    let mut used = text.len();
    for (id, title, content) in docs {
        let entry = format!("\n### {title} (id={id})\n{content}\n");
        if used + entry.len() > CONTEXT_BUDGET {
            text.push_str("\n(Further documents of this chat are not shown.)\n");
            break;
        }
        used += entry.len();
        text.push_str(&entry);
    }
    Ok(Some(text))
}

/// One edit a reply asks for: the document's id and its complete new text.
#[derive(Debug, PartialEq)]
pub(crate) struct Edit {
    pub id: Uuid,
    pub content: String,
}

/// The `document-edit` blocks in a reply, in order. A block opens with a run of at least three
/// backticks and closes with the same run alone on a line, so a document may contain code blocks.
pub(crate) fn parse_edits(reply: &str) -> Vec<Edit> {
    let mut edits = Vec::new();
    let mut lines = reply.lines();
    while let Some(line) = lines.next() {
        let Some((fence, info)) = opening_fence(line) else {
            continue;
        };
        let Some(id) = info
            .strip_prefix("document-edit")
            .and_then(|rest| rest.trim().strip_prefix("id="))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|raw| Uuid::parse_str(raw).ok())
        else {
            continue;
        };
        let mut body = Vec::new();
        let mut closed = false;
        for inner in lines.by_ref() {
            if inner.trim() == fence {
                closed = true;
                break;
            }
            body.push(inner);
        }
        if closed {
            edits.push(Edit { id, content: body.join("\n") });
        }
    }
    edits
}

/// A block that saves text as a new document of the chat.
#[derive(Debug, PartialEq)]
pub(crate) struct Create {
    pub title: Option<String>,
    pub content: String,
}

/// The `document-create` blocks in a reply, in order. The title is optional.
pub(crate) fn parse_creates(reply: &str) -> Vec<Create> {
    let mut creates = Vec::new();
    let mut lines = reply.lines();
    while let Some(line) = lines.next() {
        let Some((fence, info)) = opening_fence(line) else {
            continue;
        };
        let Some(rest) = info.strip_prefix("document-create") else {
            continue;
        };
        let title = rest.trim().strip_prefix("title=").and_then(|raw| {
            let raw = raw.trim();
            let value = match raw.strip_prefix('"') {
                Some(quoted) => quoted.split('"').next()?.to_string(),
                None => raw.split_whitespace().next()?.to_string(),
            };
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        });
        let mut body = Vec::new();
        let mut closed = false;
        for inner in lines.by_ref() {
            if inner.trim() == fence {
                closed = true;
                break;
            }
            body.push(inner);
        }
        if closed {
            creates.push(Create { title, content: body.join("\n") });
        }
    }
    creates
}

/// The reply of the chat's last completed turn before this one, as stored.
async fn previous_reply(pool: &PgPool, turn_id: Uuid) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT prev.reply FROM turns cur
         JOIN turns prev ON prev.thread_id = cur.thread_id AND prev.seq < cur.seq
         WHERE cur.id = $1 AND prev.status = 'completed' AND prev.reply IS NOT NULL
         ORDER BY prev.seq DESC LIMIT 1",
    )
    .bind(turn_id)
    .fetch_optional(pool)
    .await
    .map(Option::flatten)
}

/// Saves each `document-create` block of a completed reply as a document of the chat, made by the
/// person who sent the turn. A block with no text takes the chat's previous reply. A block with too
/// much text is skipped.
pub(crate) async fn apply_creates(
    pool: &PgPool,
    turn_id: Uuid,
    workspace_id: Uuid,
    thread_id: Uuid,
    made_by: Option<Uuid>,
    reply: &str,
) -> Result<usize, sqlx::Error> {
    const MAX_CONTENT: usize = 500_000;
    let mut saved = 0;
    for create in parse_creates(reply) {
        let mut content = create.content.trim_end().to_string();
        if content.trim().is_empty() {
            content = match previous_reply(pool, turn_id).await? {
                Some(previous) => previous.trim_end().to_string(),
                None => continue,
            };
        }
        if content.trim().is_empty() || content.len() > MAX_CONTENT {
            continue;
        }
        let title = match create.title {
            Some(title) => title.chars().take(200).collect(),
            None => super::dispatch::document_title(&content),
        };
        sqlx::query(
            "INSERT INTO artifacts (workspace_id, thread_id, title, kind, content, created_by)
             VALUES ($1, $2, $3, 'document', $4, $5)",
        )
        .bind(workspace_id)
        .bind(thread_id)
        .bind(title)
        .bind(content)
        .bind(made_by)
        .execute(pool)
        .await?;
        saved += 1;
    }
    Ok(saved)
}

/// The fence and the info string of a line that opens a fenced block, if it does.
fn opening_fence(line: &str) -> Option<(String, &str)> {
    let trimmed = line.trim_start();
    let ticks = trimmed.chars().take_while(|c| *c == '`').count();
    if ticks < 3 {
        return None;
    }
    Some(("`".repeat(ticks), trimmed[ticks..].trim()))
}

/// Applies the edits a completed turn's reply asks for to documents of the same chat. Each change
/// keeps the replaced text as a version, made by the person who sent the turn.
pub(crate) async fn apply_document_edits(
    pool: &PgPool,
    thread_id: Uuid,
    made_by: Option<Uuid>,
    reply: &str,
) -> Result<usize, sqlx::Error> {
    let mut applied = 0;
    for edit in parse_edits(reply) {
        let current: Option<(String, String)> = sqlx::query_as(
            "SELECT title, content FROM artifacts WHERE id = $1 AND thread_id = $2 AND kind = 'document'",
        )
        .bind(edit.id)
        .bind(thread_id)
        .fetch_optional(pool)
        .await?;
        let Some((title, content)) = current else {
            tracing::warn!(document = %edit.id, "a reply asked to change a document that is not in this chat");
            continue;
        };
        if content == edit.content {
            continue;
        }
        let mut tx = pool.begin().await?;
        sqlx::query("INSERT INTO artifact_versions (artifact_id, title, content, replaced_by) VALUES ($1, $2, $3, $4)")
            .bind(edit.id)
            .bind(&title)
            .bind(&content)
            .bind(made_by)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE artifacts SET content = $2, updated_at = now() WHERE id = $1")
            .bind(edit.id)
            .bind(&edit.content)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        applied += 1;
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "7d3f2a10-1b2c-4d5e-8f90-a1b2c3d4e5f6";

    #[test]
    fn a_block_gives_the_document_and_its_new_text() {
        let reply = format!("Done.\n\n````document-edit id={ID}\n# 新标题\n\n```rust\nfn main() {{}}\n```\n````\n");
        let edits = parse_edits(&reply);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].id.to_string(), ID);
        assert_eq!(edits[0].content, "# 新标题\n\n```rust\nfn main() {}\n```");
    }

    #[test]
    fn a_create_block_gives_its_title_and_text() {
        let reply = "Saved.\n\n````document-create title=\"结题总结\"\n# 结题\n\n正文\n````\n";
        assert_eq!(
            parse_creates(reply),
            vec![Create { title: Some("结题总结".to_string()), content: "# 结题\n\n正文".to_string() }]
        );
        let untitled = parse_creates("````document-create\nonly text\n````");
        assert_eq!(untitled, vec![Create { title: None, content: "only text".to_string() }]);
        // An edit block is not a create block.
        assert!(parse_creates(&format!("````document-edit id={ID}\nx\n````")).is_empty());
    }

    #[test]
    fn blocks_that_do_not_parse_change_nothing() {
        assert!(parse_edits("```document-edit id=not-a-uuid\nx\n```").is_empty());
        assert!(parse_edits("```rust\nlet x = 1;\n```").is_empty());
        // Never closed: the rest of the reply is not taken as a document.
        assert!(parse_edits(&format!("````document-edit id={ID}\nno end")).is_empty());
    }
}

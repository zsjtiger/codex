//! Documents a chat holds, and the edits the model makes to them.
//!
//! Each turn in a chat gets the chat's documents in front of the message, so the model can see
//! them. To change one, the model replies with the document's complete new text in a block that
//! starts with four backticks and `document-edit id=<document id>`. When the turn completes, the
//! gateway applies the block to that document of the same chat, keeping the text it replaces as a
//! version. Blocks that name a document of another chat, or that do not parse, change nothing.

use sqlx::PgPool;
use uuid::Uuid;

/// Documents shown to the model for one turn, in characters. Documents past this are left out.
const CONTEXT_BUDGET: usize = 200_000;

/// The documents of a chat, with the instructions for changing them. `None` when the chat has none.
pub(crate) async fn document_context(pool: &PgPool, thread_id: Uuid) -> Result<Option<String>, sqlx::Error> {
    let docs: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, title, content FROM artifacts
         WHERE thread_id = $1 AND kind = 'document' ORDER BY created_at, id",
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await?;
    if docs.is_empty() {
        return Ok(None);
    }
    let mut text = String::from(
        "Documents in this chat. You may change one. To do so, reply with its complete new text in a block \
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
    fn blocks_that_do_not_parse_change_nothing() {
        assert!(parse_edits("```document-edit id=not-a-uuid\nx\n```").is_empty());
        assert!(parse_edits("```rust\nlet x = 1;\n```").is_empty());
        // Never closed: the rest of the reply is not taken as a document.
        assert!(parse_edits(&format!("````document-edit id={ID}\nno end")).is_empty());
    }
}

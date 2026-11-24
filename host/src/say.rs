//! `POST /api/groups/{id}/say`: one call for what the person typed.
//! Stores a text message, then queues a task when someone should act:
//! in a DM, its bro; in a group, the member named by a leading `@名字`, and
//! then every member gets one turn in order (see [`crate::turns`]).
//! `/messages` and `/tasks` stay as they are.

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use robroz_contract::{route_mention, Message, MessageKind, Said, Say, Task, TaskState};
use rusqlite::{params, Connection};

use crate::{bros, db_error, error_response, group_exists, lock_db, new_id, now_rfc3339, AppState};

/// Who should act on `body` in `group_id`, and on what prompt, and whether
/// this is a group (where every member then gets a turn, see [`crate::turns`]).
fn assignee(
    conn: &Connection,
    group_id: &str,
    body: &str,
) -> rusqlite::Result<(Option<(String, String)>, bool)> {
    if let Some(bot) = bros::dm_owner(conn, group_id)? {
        return Ok((Some((bot, body.to_string())), false));
    }
    let mut stmt = conn.prepare(
        "SELECT b.id, b.name FROM memberships m JOIN bots b ON b.id = m.bot_id WHERE m.group_id = ?1",
    )?;
    let members: Vec<(String, String)> = stmt
        .query_map([group_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let routed = route_mention(
        body,
        members
            .iter()
            .map(|(id, name)| (id.as_str(), name.as_str())),
    );
    Ok((routed, true))
}

pub(crate) async fn say(
    State(state): State<AppState>,
    AxumPath(group_id): AxumPath<String>,
    Json(input): Json<Say>,
) -> Response {
    let text = input.body.trim().to_string();
    if text.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "请填写内容");
    }
    let mut conn = lock_db(&state);
    let result = (|| {
        let tx = conn.transaction()?;
        if !group_exists(&tx, &group_id)? {
            return Ok(None);
        }
        let now = now_rfc3339();
        let message = Message {
            id: new_id(),
            kind: MessageKind::Text,
            body: text.clone(),
            created_at: now.clone(),
        };
        tx.execute(
            "INSERT INTO messages (id, group_id, kind, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![message.id, group_id, message.kind.as_str(), message.body, now],
        )?;
        let (routed, in_group) = assignee(&tx, &group_id, &text)?;
        let task = match routed {
            None => None,
            Some((bot, prompt)) => {
                let task = Task {
                    id: new_id(),
                    assignee_bot_id: bot,
                    prompt,
                    state: TaskState::Queued,
                };
                tx.execute(
                    "INSERT INTO tasks (id, group_id, assignee_bot_id, prompt, state, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                    params![task.id, group_id, task.assignee_bot_id, task.prompt, task.state.as_str(), now],
                )?;
                Some(task)
            }
        };
        if in_group {
            let mentioned = task
                .as_ref()
                .map(|t| (t.assignee_bot_id.as_str(), t.id.as_str()));
            crate::turns::open_round(
                &tx,
                &group_id,
                &message.id,
                &text,
                mentioned,
                crate::turns::PERSON_DEPTH,
                &now,
            )?;
        }
        tx.commit()?;
        Ok(Some(Said { message, task }))
    })();
    drop(conn);
    match result {
        Ok(None) => error_response(StatusCode::NOT_FOUND, "群不存在"),
        Ok(Some(said)) => (StatusCode::CREATED, Json(said)).into_response(),
        Err(err) => db_error(err),
    }
}

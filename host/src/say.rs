//! `POST /api/groups/{id}/say`: one call for what the person typed.
//! Stores a text message, then queues a task when someone should act:
//! in a DM, its bro; in a group, the member named by a leading `@名字`, and
//! then every member gets one turn in order (see [`crate::turns`]). A group
//! message that names nobody goes to one picked bro instead
//! ([`crate::responder`]). When
//! that bro (DM or `@名字`) still has an open task there, the message joins
//! that task instead (see [`crate::follow_ups`]).
//! `/messages` and `/tasks` stay as they are.

use crate::extract::Json;
use crate::extract::Path as AxumPath;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use robroz_contract::{route_mention, Message, MessageKind, Said, Say, Task, TaskState};
use rusqlite::{params, Connection};

use crate::responder::{self, Decision, Plan};
use crate::{bros, db_error, error_response, group_exists, lock_db, new_id, now_rfc3339, AppState};

/// The members of a group, in join order: `(bot id, name)`.
fn members(conn: &Connection, group_id: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT b.id, b.name FROM memberships m JOIN bots b ON b.id = m.bot_id
         WHERE m.group_id = ?1 ORDER BY m.rowid",
    )?;
    let rows = stmt.query_map([group_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// `(who acts and on what prompt, in a group, picked by the single responder)`.
type Assigned = (Option<(String, String)>, bool, bool);

/// Who should act on `body` in `group_id`, and on what prompt, whether
/// this is a group (where every member then gets a turn, see
/// [`crate::turns`]), and whether the bro was picked by [`crate::responder`]
/// (`single`: then it is the only turn). `picked` only applies when `body`
/// names nobody; a bro that has left the group since is skipped.
fn assignee(
    conn: &Connection,
    group_id: &str,
    body: &str,
    picked: Option<&Decision>,
) -> rusqlite::Result<Assigned> {
    if let Some(bot) = bros::dm_bot(conn, group_id)? {
        return Ok((Some((bot, body.to_string())), false, false));
    }
    let members = members(conn, group_id)?;
    let routed = route_mention(
        body,
        members
            .iter()
            .map(|(id, name)| (id.as_str(), name.as_str())),
    );
    if routed.is_none() {
        if let Some(decision) = picked {
            if members.iter().any(|(id, _)| *id == decision.bot_id) {
                return Ok((
                    Some((decision.bot_id.clone(), body.to_string())),
                    true,
                    true,
                ));
            }
        }
    }
    Ok((routed, true, false))
}

/// Pick the answering bro for a group message that names nobody, before the
/// message is stored. The model call (if any) runs off the database lock.
async fn pick_responder(
    state: &AppState,
    group_id: &str,
    text: &str,
) -> rusqlite::Result<Option<Decision>> {
    let (plan, selector) = {
        let conn = lock_db(state);
        let named = route_mention(
            text,
            members(&conn, group_id)?
                .iter()
                .map(|(id, name)| (id.as_str(), name.as_str())),
        )
        .is_some();
        let plan = responder::plan(&conn, state.single_responder, group_id, named)?;
        let selector = match plan {
            Plan::Ask(_) => {
                responder::selector_for(&conn, state.selector.as_ref(), state.model.as_ref())
            }
            _ => None,
        };
        (plan, selector)
    };
    let decision = match plan {
        Plan::Off => return Ok(None),
        Plan::Decided(decision) => decision,
        Plan::Ask(ctx) => {
            let owned = text.to_string();
            let spare = ctx.clone();
            tokio::task::spawn_blocking(move || {
                responder::choose(&ctx, &owned, selector.as_ref(), responder::SELECT_TIMEOUT)
            })
            .await
            .unwrap_or_else(|_| responder::choose(&spare, text, None, responder::SELECT_TIMEOUT))
        }
    };
    decision.log(group_id);
    Ok(Some(decision))
}

pub(crate) async fn say(
    State(state): State<AppState>,
    axum::Extension(user): axum::Extension<crate::users::CurrentUser>,
    AxumPath(group_id): AxumPath<String>,
    Json(input): Json<Say>,
) -> Response {
    let text = input.body.trim().to_string();
    if text.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            robroz_contract::host_text::NEED_TEXT,
        );
    }
    let picked = match pick_responder(&state, &group_id, &text).await {
        Ok(picked) => picked,
        Err(err) => return db_error(err),
    };
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
            author_user_id: Some(user.0.clone()),
            author_name: crate::users::load(&tx, &user.0)?.map(|(name, _)| name),
        };
        tx.execute(
            "INSERT INTO messages (id, group_id, kind, body, created_at, author_user_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                message.id,
                group_id,
                message.kind.as_str(),
                message.body,
                now,
                user.0
            ],
        )?;
        let (routed, in_group, single) = assignee(&tx, &group_id, &text, picked.as_ref())?;
        // A follow-up: the bro's still-open task here (the DM's bro, or the
        // `@名字` bro in a group).
        let open = match &routed {
            Some((bot, _)) => crate::follow_ups::open_task(&tx, &group_id, bot)?,
            None => None,
        };
        let mut appended_to_task_id = None;
        let task = match routed {
            None => None,
            Some((bot, prompt)) if open.is_some() => {
                let (id, task_prompt, state) = open.expect("checked");
                crate::follow_ups::attach(&tx, &id, &message.id, &prompt, &now)?;
                appended_to_task_id = Some(id.clone());
                Some(Task {
                    id,
                    assignee_bot_id: bot,
                    prompt: task_prompt,
                    state: TaskState::parse(&state).unwrap_or(TaskState::Queued),
                })
            }
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
            match mentioned {
                Some(started) if single => crate::turns::open_single_round(
                    &tx,
                    &group_id,
                    &message.id,
                    &text,
                    started,
                    crate::turns::PERSON_DEPTH,
                    &now,
                )?,
                _ => crate::turns::open_round(
                    &tx,
                    &group_id,
                    &message.id,
                    &text,
                    mentioned,
                    crate::turns::PERSON_DEPTH,
                    &now,
                )?,
            }
        }
        tx.commit()?;
        Ok(Some(Said {
            message,
            task,
            appended_to_task_id,
        }))
    })();
    drop(conn);
    match result {
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            robroz_contract::host_text::GROUP_NOT_FOUND,
        ),
        Ok(Some(said)) => (StatusCode::CREATED, Json(said)).into_response(),
        Err(err) => db_error(err),
    }
}

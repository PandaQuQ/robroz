# Concepts and terms

Names, task states and cards in robroz. Exact API fields are in [api.md](api.md). This page is about the ideas; if it disagrees with api.md or the code, those win.

The desktop client UI is English by default. Switch to 中文 with the Language / 语言 row on the Settings (Model) page. `ROBROZ_LANG=en|zh` overrides the saved choice for one run and is not written back. Quoted UI text below is what the English UI shows; the Chinese is in parentheses where it helps. See [configuration.md](configuration.md) for the saved `lang` file.

## Glossary

| Term | What it is | Where it is used |
| --- | --- | --- |
| bro | An agent, which is a bot. Adding one is Add a bro (「引入一个 bro」). | UI only. The conversation list is titled Chats (「聊天」); the new button is Add a bro (「引入一个 bro」) (`crates/robroz-ui/src/conversations.rs`, `TITLE` and `NEW_BRO`); an empty list shows No bros yet. Add your first one. (「还没有 bro，引入第一个」). Click the name at the top of a DM to rename. A bro profile page is not built yet; it is planned in phase 2 (“manage a bot as a contact”), see [roadmap-phase2.md](roadmap-phase2.md). |
| Default name | The name filled in on create: the first is `bro`, then `bro 2`, `bro 3`, the first one not already taken. You can rename it at any time. | Only a default at create time; no extra field. |
| `botId` / `botName` | Frozen field names in the contract. | Code, the API and the database keep these names; they are not renamed to bro. |
| Group | A conversation. It can have people and more than one bro. | Code and the API use “group”. The conversation list is titled Chats (「聊天」); some other UI strings still say 群. |
| DM (「私聊」) | A group with only you and one bro, not a different kind of object. | The UI tags it DM (「私聊」). In code it is a group. Messages, tasks, events, the live stream and unread all work on it as they do on any group. |
| Group directory | One folder per group, `ROBROZ_VOLUMES/{groupId}`, mounted in the container at `/workspace`. The folder stays after the container is removed. | A delivery card's `path` is relative to this folder. |
| Control directory | One folder per task, `ROBROZ_CONTROL/{taskId}`. Only the host writes it; it is mounted read-only at `/control` in the container. | Holds `prompt.txt`, `reply.txt` and the takeover marker `takeover`. See [architecture.md](architecture.md). |
| Routine | A scheduled task. When it is due, the host creates an ordinary task. | The UI button is Routines (「例行任务」). The API is `/api/schedules`. |
| Briefing | A delivery card after a successful routine, with `briefing` set to true. Not a new card type. | The client shows a corner badge Briefing (「简报」) (`robroz_card::briefing_badge`). |
| Allow rules (「放行规则」) | Short sentences you write so auto-review can approve a risky action without showing you a card. | The settings page title is Allow rules (「放行规则」). See [api.md](api.md) and [design/auto-review.md](design/auto-review.md). |

## Bro names

A bot's name is global: the same `botId` has the same name in every group. `@name` in `/say` matches that global name. Passing `name` in `POST /api/groups/{id}/members` for a bot that already exists does not rename it; use `PATCH /api/bros/{id}`.

## Group turns

When a person sends a group `/say`, every bro in the group gets a turn. The bro named by a leading `@name` or `＠name` (full-width) goes first, and that task is returned in `Said.task`. The rest go one at a time in join order and may stay silent (no message, no card, no unread; `lastActivityAt` does not change). An un-mentioned turn stays hidden until that bro speaks. If no one is named, `Said.task` is `null` and every member still gets a turn.

If the named bro already has an open task in the group (`queued`, `running` or `blocked`), the message is a follow-up: it attaches to that task instead of starting a new one. `Said.task` is the existing task, and `Said.appendedToTaskId` is set. The same rule applies in a DM, without `@`.

When a bro posts a clean reply in a group (not silence, not a failed result), the other bros get a turn to join in. The speaker is not asked again, and neither are bros still waiting in the same round. A leading `@name` in that reply moves that bro to the front. This stops after two such rounds (`MAX_BRO_DEPTH = 2`). Silence, a failed result, a DM, `POST /api/groups/{id}/tasks`, and scheduled tasks do not open a new round.

A first-task walk-through is in [install.md](install.md#first-task). The code is `host/src/turns.rs` (`open_round`, `open_bro_round`).

## Task states

`state` has only four values, and they are frozen; a fifth will not be added: `queued`, `running`, `blocked`, `done`.

Failure is not a state. It is expressed in two ways:

- **A dead run:** the container exited non-zero, wrote no result, or disappeared (the host or Docker restarted, or the container was deleted). The task ends as `done`, `failure` is 「没写出结果」 (the client shows Finished with no result), and there is no delivery card. It is not queued again, a container is not started again, and `resume` returns 409 「这次不能恢复」 (the client shows This one can't be resumed). There is no fifth state.
- **Couldn't finish:** the task is `done` but carries a `failure`. See [permissions.md, “Couldn't finish”](permissions.md#couldnt-finish-failure). The client shows Finished with no result (「没写出结果」) as it is; any other reason is shown as Couldn't finish: {reason} (「没做成：原因」). It is not shown as Couldn't finish: Finished with no result (「没做成：没写出结果」).

## Status card fields

A task has exactly one status card. Its `id` equals `taskId`. The client updates that card in place by id: `createdAt` stays put, `updatedAt` moves.

| Field | Meaning |
| --- | --- |
| `containerUp` | `true` only while the task is `running` and the container is still up. `false` when queued, done, blocked, or when a run has already died. |
| `label` | The API field is still the Chinese line from `robroz_card::status_line(state, containerUp)`: 「排队」, 「进行」 when the container is up, 「没写出结果」 when a `running` card has no container (old dead-run data), 「卡住」, 「完成」. The client shows Queued, Running, Finished with no result, Stuck or Done. The client may show a different line for a failed task or for a task waiting on an approval card; see below. |
| `failure` | Optional. Why a `done` task did not finish. |
| `waitingFor` | Optional. Why a `blocked` task is waiting: `approval` (answer an approval card) or `takeover` (take over in the DM). |
| `progress` | Optional. The API field is still Chinese. What the bro is doing while `running` with a live container. When the task is `blocked` waiting for an MCP tool approval, this is 「在用 {server}.{tool}」 (the client shows Using {server}.{tool}). When the approval card was moved to the owner's DM, this line is 「已发到主人私聊」 (the client shows Sent to owner's DM). |

Colours and actions come from `robroz-card` (`appearance`, `palette`); the client does not pick its own palette. `appearance` by itself is: queued, grey; running with a live container, ink (dark); a dead run (`running` with no container), grey, same as queued; done, green; blocked, amber, with the action Take over in DM (「去私聊接管」).

The client then applies two overrides (`client/src/model.rs`):

- A `done` task with a `failure` is grey, with no action. The line is `failure_line`, never the green Done (「完成」).
- A task blocked with `waitingFor: "approval"` is grey, with no takeover button. The line is Needs your approval (「等你批准」). Amber and Take over in DM (「去私聊接管」) stay only for a real takeover (`waitingFor: "takeover"`).

### progress

- The host reports key moments itself: Getting started (「开始干活」) when the run begins, or Thinking about a reply (「在想怎么回」) when a model is attached; Running in the sandbox (「在沙箱里跑」) after the sandbox is up; Using <tool name> (「在用 <工具名>」) when a tool is called (MCP tools use the display name `{server}.{tool}`).
- The bro can report too: a line `robroz-progress:text` in the prompt or in the model reply. The runner writes the last progress line to the group folder `tasks/{taskId}/progress`, and the host reads it while the task is running. A progress line in the model reply is applied when the sandbox comes up; it wins over Running in the sandbox (「在沙箱里跑」). Progress lines do not go into `result.txt`.
- The text is normalized to one line: newlines and consecutive whitespace become a single space, ends are trimmed, at most 60 characters (longer text is cut to 59 characters plus 「…」). This is not an error.
- If the text is unchanged, `updatedAt` is not moved and no extra event is pushed.
- The line is cleared when the task finishes, is blocked for a takeover, or the container is gone, and a new run starts from scratch. Exception: a `blocked` task waiting for an MCP tool approval keeps Using {server}.{tool} (「在用 {server}.{tool}」); the line is cleared after the card is answered. If that card was moved to the owner's DM, the line is Sent to owner's DM (「已发到主人私聊」) instead.
- The host may also write No reply from the model or tools for N min (「模型和工具还没回，已等 N 分钟」) or No new progress for N min (「N 分钟没有新进度」). The stall hint is only a hint. The No reply hint shows at half of `ROBROZ_STEP_TIMEOUT_SECS`. A step that runs past `ROBROZ_STEP_TIMEOUT_SECS` is cancelled: the container is stopped and the task ends as Finished with no result (「没写出结果」), with no delivery and no rerun. Time waiting on an approval card does not count. The variables are in [configuration.md](configuration.md).
- Code: `host/src/progress.rs`, `host/src/watchdog.rs`, `host/src/timeouts.rs`.

## Cards (events)

Each item in the event stream is an `Event`, distinguished by `type`. Full fields are in [api.md, event types](api.md#event-types).

| `type` | What it is | Who sends it |
| --- | --- | --- |
| `message` | A message. `kind` is `text`, `image` or `file`. Stored in the database only, not as a file. | A person (`/say`, `/messages`) |
| `status` | The task's status card. One per task. | The host |
| `delivery` | A result card: a summary plus a path relative to the group folder, today `tasks/{taskId}/result.txt`. A briefing is the same type with `briefing: true`. | The host, when the task writes a result |
| `choice` | A choice card: pick from 2 to 4 fixed options, single or multi. `primary` is the bro's recommendation (ignored when `multi`). | A bro, through the API |
| `approval` | An approval card: ask a person before deleting, sending, spending, or calling a tool. The host also issues one when a task wants to write outside (with `taskId`). | A bro or the host |
| `confirm` | A confirm card: a bro proposed a memory or a skill, and a person decides. | The host, after a successful task |

Approvals are checked against your Allow rules first (auto-review), including cards a bro issues itself with `POST /api/groups/{id}/approvals`. If a rule allows the action, no card is shown; the action goes ahead, and `GET /api/approvals/{id}` can still read it (`decision` is `"approved"`; approve or deny later returns 409). If the card is blocked or there are no rules, it goes to the owner's DM, with `expiresAt` set. The group the bro posted in lists the same card id as a notice, with no buttons, and the client shows Sent to owner's DM (「已发到主人私聊」). Unread counts only in the DM; the group copy still updates `lastActivityAt`. A card that expires has `reason` Approval expired (「审批已过期」). Confirm cards and a takeover do not expire. Allow-rule changes (`rule_add` / `rule_delete`) use an approval card on the settings page; they are not sent to any chat.

Rules:

- When a card changes, it appears again with the same `id`. The client replaces the existing row in place.
- The client skips an event whose `type` it does not recognise, so an older client does not break when a new type is added.
- There is no promised order between different event types; do not depend on one.

## Read and unread

Each group has one read position (a single owner for now). Unread counts only what a bro sent that you need to see, after that position:

- Delivery cards, choice cards, and confirm cards in that chat.
- Approval cards that were actually issued (a card still in auto-review, or one that a rule already allowed, is not counted). A bro-issued card that was moved to the owner's DM is unread only in the DM; the group listing is a notice and does not count, before or after it is answered.
- Tasks that became `blocked`, with `updatedAt` after the read position, and that are not hidden.

These do not count: your own messages; status changes through queued, running and done; un-mentioned group turns that are still hidden (including a silent reply); Allow-rule cards on the settings page (`rule_add` / `rule_delete`), which are not in any chat.

`lastActivityAt` is the newest change among messages, visible tasks, deliveries, choices and approvals. A bro-issued card moved to the owner's DM also counts as activity in the original group (the notice is listed there), but it is not unread there. A silent turn does not change `lastActivityAt`. Details are in [api.md, unread](api.md#unread).

# Auto-review

Status: **Draft**. This is a design proposal, not an accepted contract. Existing
behaviour below is taken from `main` at `22a0279`. If this document disagrees
with the code or with [api.md](../api.md), [permissions.md](../permissions.md),
or [concepts.md](../concepts.md), the code wins.

Auto-review is a host-side gate in front of risky actions. It layers on top of
the existing three-tier permission policy. It does not replace that policy, and
it does not add a fifth task `state`.

## Rules

1. Every risky action is reviewed before it runs: shell, MCP write calls, pull
   request merges, sending messages, and deleting. Review returns only **allow**
   or **block**. There is no third outcome.
2. The owner writes allow rules as one sentence on a settings page. A matching
   rule allows the action directly, with no card. A rule only counts once the
   owner has confirmed it on a card. Bros cannot add or edit rules themselves.
3. When an action is blocked, the approval card goes only to the owner's direct
   message with that bro. Group messages, other bros' words, web pages, and tool
   output never count as owner authorization.
4. A denied or expired card is final: the action does not run, the bro does not
   retry, reword, or work around the decision, and the host does not offer
   another path to the same action in that task.
5. Auto-review is additive to the three-tier permissions (in-container work is
   automatic; read-only internet is metered by quota; external writes and
   captcha go through the owner's DM and then `blocked`). A blocked action uses
   the existing `blocked`（卡住）state. There is no fifth `state`.

## Relationship with the three-tier permissions

The three tiers live in `robroz_policy::tier(SideEffect)` and are unchanged.
Auto-review runs *after* that classification, only for actions that are about to
happen, and only where the table says so.

Check order:

1. Classify the side effect and apply the three-tier rule, including the
   outbound quota and GET-only restriction.
2. If the tier already forbids the action (over quota, or a non-GET outbound
   call), the existing outcome stands: the task becomes `done` with `failure`,
   not `blocked`. Auto-review is not consulted.
3. If the action is in auto-review's scope and no confirmed allow rule matches,
   the host blocks it and asks the owner. If a confirmed rule matches, the host
   allows it and continues, writing the same takeover or continue markers that a
   manual approve would write today.

| Action class | Three-tier | Auto-review |
| --- | --- | --- |
| In-container workspace read and write (`WorkspaceRead` / `WorkspaceWrite`) | `Allow`. No notification. Success writes `tasks/{taskId}/result.txt`. | Not reviewed. Stays automatic. |
| Read-only internet: outbound GET, `web_search` / `web_fetch`, MCP tools marked `readOnlyHint: true` (`Outbound`) | `Metered`. GET only, counted per task up to `ROBROZ_GET_LIMIT` (default 8). Over quota or non-GET: `done` with `failure`, no delivery card, not `blocked`. | Not reviewed. The quota already limits the action. |
| External write, including `robroz-action:external` and MCP tools that are not read-only (`ExternalWrite`) | `Takeover`. Task becomes `blocked`, `waitingFor` is `approval`. The host already issues a task or `call` approval card. | Reviewed before the write runs. A matching allow rule allows and continues (no card). Otherwise the approval card goes to the owner's DM; the group status card only shows 「等你批准」. |
| Captcha (`robroz-action:captcha`, `Captcha`) | `Takeover`. Task becomes `blocked`, `waitingFor` is `takeover`. No approval card: the owner must complete the captcha in the DM, then `POST /api/tasks/{id}/resume`. | A sentence rule cannot auto-allow a captcha (someone has to fill it). The takeover flow is unchanged. |
| Host-side shell, pull request merge, sending a message, deleting, and spending | Not a `SideEffect` when the host or bro initiates them outside the container prompt. Bros can already ask with `POST /api/groups/{id}/approvals` (`action` is `delete`, `send`, or `spend`); answering that card does not make the host perform the action. There is no first-class host shell or merge tool on `main` today. | Reviewed before any such action *runs*. A matching rule allows; otherwise an approval card goes to the owner's DM. When host shell or merge tools are added, they use this gate. Existing `send` / `delete` / `spend` / `call` verbs are reused where they already fit. |

A matching allow rule for an `ExternalWrite` must do what today's
`POST /api/approvals/{id}/approve` already does: append the takeover hash or
write the MCP continue marker, then return the task to `queued` so pump can
continue. It does not invent a new continue path.

## What already exists on `main`

Verified on `22a0279`. Unmerged work is marked planned.

- **Task states** (frozen, [concepts.md](../concepts.md)): `queued`（排队）,
  `running`（在跑）, `blocked`（卡住，要人）, `done`（结束）. Failure is not a
  state. A `done` task that did not succeed carries `failure`. A run that did
  not write a result stays `running` with `containerUp` false and the label
  「没写出结果」 (`resume` returns 409「这次不能恢复」).
- **`waitingFor`** (only when `blocked`): `approval` (answer the approval card)
  or `takeover` (go to the DM, then `resume`).
- **`POST /api/tasks/{id}/resume`**: only accepts `blocked` tasks that are
  waiting for takeover. Success appends `intent_hash` to the takeover file,
  clears the block, and returns the task to `queued`. 409「请先回答审批卡」when
  the task is blocked on an MCP or external-write approval card. Auto-review
  does not change this endpoint.
- **Approval cards** (host): `POST /api/groups/{id}/approvals` for bro-issued
  `delete` / `send` / `spend` cards; host-issued task cards (`action` `send`,
  with `taskId`) and MCP write cards (`action` `call`, with `mcpServer` /
  `mcpTool`). Answered once with `POST /api/approvals/{id}/approve` or `/deny`.
  Decisions are `approved` or `denied`. Approve of a task or MCP card requeues
  the task; deny ends it as `done` with `failure`「你拒绝了：…」. A second
  answer is 409「已经答过了」.
- **[#58](https://github.com/PandaQuQ/robroz/pull/58) (merged)**: when
  `blocked` and `waitingFor` is `approval`, the status card is grey 「等你批准」,
  with no button. The conversation list preview is 「{botName}：等你批准」.
- **[#68](https://github.com/PandaQuQ/robroz/pull/68) (merged)**: confirm cards
  for memory and skill proposals, answered with the same approve / deny
  endpoints, in the owner's conversation.
- **MCP call approval UI**: the host already emits `call` cards. On `main` the
  client renders them with the generic approval card (「批准」「拒绝」). The
  dedicated MCP call approval card in
  [#103](https://github.com/PandaQuQ/robroz/pull/103) is **not on `main`** and
  is planned. The MCP settings page and stdio command approval
  (`POST /api/mcp/servers/{id}/approve-command`, button 「批准运行」) already
  exist; that is a different card, on the settings page, not this gate.
- **Today's gap**: approval events are stored and streamed in the *task's
  group*, so the interactive card currently appears in the group thread next to
  the status card. Auto-review moves the interactive card to the owner's DM.
  Cards do not yet expire.

## State flow

Reuse the frozen states and the existing endpoints. No new `state`, no new
`waitingFor` value, no change to `POST /api/tasks/{id}/resume`.

1. A risky action is about to run. The host reviews it (allow rules, then a
   card if needed). Review returns allow or block.
2. **Allow** (a confirmed rule matched, or the owner approved the card): the
   action runs. For an in-flight task this is the same continue path as today
   (markers, then `queued`, then pump).
3. **Block** (no matching rule): the task, if any, becomes `blocked` with
   `waitingFor` set to `approval`. In the *group*, the status card shows
   「等你批准」 and has no button. The approval card is created in the owner's
   DM with that bro (`POST /api/bros/{id}/dm` already opens or returns that
   conversation). Group messages cannot approve it.
4. **Owner approves** (`POST /api/approvals/{id}/approve`): same as today's
   approve path. The card records `decision: approved`, `answeredAt`, and
   optional `answeredBy`.
5. **Owner denies** (`POST /api/approvals/{id}/deny`): same as today's deny
   path. The task becomes `done` with `failure`「你拒绝了：…」. The action does
   not run. The bro must not retry, reword, or work around it in this task.
6. **Card expires** (proposed; cards have no expiry on `main`): the host
   settles the unanswered card as denied. The task becomes `done` with
   `failure` (proposed copy: 「审批已过期」). Answering after expiry is 409
   (proposed: 「已经过期了」). This is also final: no retry, no workaround.
7. Captcha stays on `waitingFor: takeover` and `POST /api/tasks/{id}/resume`.
   Auto-review does not issue a card and does not auto-allow it.

A bro-issued `delete` / `send` / `spend` card that does not drive a task still
follows rules 3–6: the card goes to the owner's DM, and a deny or expiry is
final for that request. Answering still does not make the host perform the
action (unchanged).

## Client

- **Group thread**: show only the existing status treatment, 「等你批准」. Do
  not show the interactive approval card in the group. Do not treat any group
  message as an answer.
- **Owner DM**: the approval card itself, including approve and deny. Until
  #103 lands, `call` cards use the generic approval card already on `main`.
  After #103, the dedicated MCP call card is used in the DM. Planned: a group
  card that is waiting must not grow a second copy of the buttons.
- **Confirm cards (#68)**: reuse for confirming a new allow rule. The confirm
  card is also posted only in the owner's DM.
- **Settings — allow rules** (planned): a new page, next to the existing left-nav
  pages (routines, memory, skills, web search, MCP). List rules, add a
  one-sentence rule, and delete a rule. Adding a rule does not activate it:
  the page proposes the rule, a confirm card appears in the owner's DM, and
  the rule counts only after the owner approves that card. Bros have no
  settings UI for this and cannot confirm a rule.

## New API (additive only)

No existing fields change. No existing endpoint changes its request, response,
or status mapping. The endpoints below are **proposed**, for the backend
(超稳后端工程师) to confirm. Naming follows `/api/settings/search`,
`/api/settings/model`, `GET` / `POST` / `DELETE` for collections, and the
existing `POST /api/approvals/{id}/approve` and `/deny`.

### Proposed: allow rules

`ReviewRule` (proposed): `id`, `text` (one sentence, trimmed, non-empty, one
line), `status` (`pending` until confirmed, `kept` once confirmed, same words
as memories and skills), `createdAt`, `updatedAt`, and `confirmId` when a
confirm card is still unanswered.

| Method | Path | Role |
| --- | --- | --- |
| `GET` | `/api/settings/review-rules` | List confirmed and pending rules. Proposed response: `{"rules":[ReviewRule]}`. |
| `POST` | `/api/settings/review-rules` | Propose a rule. Body: `{"text":"…"}`. Proposed: 201 the `pending` rule, and a confirm card in the owner's DM. The rule is not active until that card is approved. Deny leaves it inactive (`rejected`, not listed as `kept`). |
| `DELETE` | `/api/settings/review-rules/{id}` | Remove a rule. Proposed: deleting a `pending` rule also closes its confirm card as `denied`. Deleting a `kept` rule takes effect immediately. Bros must not be able to add, edit, or delete rules; see open questions for the shared-token limit. |

There is no edit endpoint. To change a rule, the owner deletes it and proposes
a new one (which needs its own confirm card).

Rule confirmation reuses `POST /api/approvals/{confirmId}/approve` and `/deny`.
How the confirm card is typed is proposed, not decided: either a `confirm`
event with a new `subject` (needs a coordinated client, because unknown
`ConfirmSubject` values fail to parse today), or a new event `type` (old
clients already skip unknown types). Prefer the form that keeps old event JSON
valid and does not change existing fields. Backend to confirm.

### Existing: approval decision

Keep `POST /api/approvals/{id}/approve` and `/deny` as the only way to answer
an approval or confirm card. Do not add a parallel decision API. Request body,
`answeredBy`, 404「审批卡不存在」, and 409「已经答过了」 stay as they are.

### Proposed: expiry

Approval cards have no `expiresAt` on `main`. Proposed additive optional field
`expiresAt` (RFC 3339, same style as `createdAt` / `answeredAt`) on approval
events, omitted on cards that do not expire so old JSON stays valid. When the
timestamp is reached and `decision` is still `null`, the host settles the card
as denied (rule 6 above). A late approve or deny returns 409. The default TTL
is not set here; see open questions.

`expiresAt` is a new optional field. It is not a change to any existing field.

## Authorization

The only inputs that can allow a risky action are:

- a confirmed allow rule that matches this action, or
- the owner tapping approve on the card in their DM.

These never count, even if they look like an instruction to allow:

- group messages, including `/say` and @-mentions
- words from another bro
- a web page, search result, or any tool output
- a rule that is still `pending`, or that a bro proposed but the owner has not
  confirmed

The host uses a single shared token and cannot tell which person called an
API (`answeredBy` is client-reported and not verified). That does not weaken
the rule: a call that is not the owner's confirm on the card does not activate
a rule and does not approve an action.

## Open questions

For the backend and PM to confirm.

1. Default TTL for an approval card, and whether rule-confirm cards expire
   too (memory and skill confirm cards do not expire today).
2. Exact `failure` copy for an expired task card. This draft proposes
   「审批已过期」.
3. How a one-sentence rule is matched (the host interprets the sentence at
   review time, versus a structured pattern). This draft assumes the reviewer
   interprets the sentence and returns only allow or block. The algorithm is
   not specified here.
4. The host token is shared, so a raw `DELETE /api/settings/review-rules/{id}`
   cannot be proven to be the owner. Is the settings page plus “bros must not
   call this” enough, or should deleting a `kept` rule also require a confirm
   card?
5. Confirm-card shape for a new rule (new `confirm` `subject` versus a new
   event `type`).
6. Whether `expiresAt` applies to every action approval, including today's
   task and MCP cards, or only to cards issued after this design lands.
7. New `ApprovalAction` values for host shell or merge, when those tools
   exist. Extending that enum needs a coordinated client, as `call` did.
   Until then, reuse `send` / `delete` / `spend` / `call` where they already
   fit.

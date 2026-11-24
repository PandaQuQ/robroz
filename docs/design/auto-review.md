# Auto-review

Status: **Accepted**. Existing behaviour below is taken from `main` at
`0aed709`. If a description of current code disagrees with the tree or with
[api.md](../api.md), [permissions.md](../permissions.md), or
[concepts.md](../concepts.md), the code wins. The decisions in this document
are the accepted design for the review layer.

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

## Decisions

1. **Expiry.** Approval cards expire after 24 hours by default. The TTL is
   configurable. Expiry applies to approval cards, including task approval
   cards, MCP call cards, and rule add/delete cards. An expired card is settled
   as `decision: denied` with `reason` set to the exact string 「审批已过期」
   (`APPROVAL_EXPIRED_REASON` in the contract; planned, added by the backend).
   `answeredBy` is left unset: the owner never clicked. That is final, and no
   new decision or task state is added. Takeover cards (「去私聊接管」) never
   expire. Memory and skill confirm cards are unchanged and still do not
   expire. The `failure` copy on an expired task card is also 「审批已过期」.
   A later approve or deny returns 409「已经答过了」, the same path as a card
   that was already answered elsewhere: the client locks the buttons, refetches
   the card, and shows the expired presentation. There is no separate 409 copy
   or client branch for expiry.
2. **Rule matching.** The review model is given the pending action and the
   original text of every confirmed rule. It returns `allow` or `block` plus a
   one-sentence reason. There is no regex or wildcard matching. If the review
   itself errors or times out, it is retried once; if it still fails, the
   action is blocked. Review never default-allows.
3. **Adding and deleting a rule.** `POST` and `DELETE /api/settings/review-rules`
   do not change the rule list. Each call only creates a confirm card and
   returns **202** `{"approvalId":"..."}`. The rule is added or removed
   only after the owner confirms the card with the existing
   `POST /api/approvals/{id}/approve`. Deny leaves the list unchanged.
   Unconfirmed rules are not returned by `GET`. A bro who calls these endpoints
   only creates a card. Auth is the existing API token.
4. **Confirm card.** Rule add and delete reuse the existing approval-card
   event and the [#68](https://github.com/PandaQuQ/robroz/pull/68) confirm-card
   layout. The title is 「添加放行规则」 or 「删除规则」. The body is the rule
   sentence, verbatim. A grey line states whether this is an add or a delete.
   Delete uses a red button labelled 「删除规则」. Answered with the existing
   `/approve` and `/deny` endpoints.
5. **What expires.** Only approval cards, as in decision 1. Takeover is not an
   approval card and does not expire.
6. **`ApprovalAction`.** Two new values are added, and no existing value
   changes: `rule_add` and `rule_delete`. See [Rollout](#rollout) for the
   serde names and the required ship order. A client that does not recognise a
   value must render a generic card, once `#[serde(other)]` is in place.

Captcha is unchanged: it still goes through takeover and
`POST /api/tasks/{id}/resume`. A sentence rule cannot auto-allow it.

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
3. If the action is in auto-review's scope, the review model sees the pending
   action and all confirmed rule texts. A match allows and continues, writing
   the same takeover or continue markers that a manual approve would write
   today. Otherwise the host blocks and asks the owner.

| Action class | Three-tier | Auto-review |
| --- | --- | --- |
| In-container workspace read and write (`WorkspaceRead` / `WorkspaceWrite`) | `Allow`. No notification. Success writes `tasks/{taskId}/result.txt`. | Not reviewed. Stays automatic. |
| Read-only internet: outbound GET, `web_search` / `web_fetch`, MCP tools marked `readOnlyHint: true` (`Outbound`) | `Metered`. GET only, counted per task up to `ROBROZ_GET_LIMIT` (default 8). Over quota or non-GET: `done` with `failure`, no delivery card, not `blocked`. | Not reviewed. The quota already limits the action. |
| External write, including `robroz-action:external` and MCP tools that are not read-only (`ExternalWrite`) | `Takeover`. Task becomes `blocked`, `waitingFor` is `approval`. The host already issues a task or `call` approval card. | Reviewed before the write runs. A matching allow rule allows and continues (no card). Otherwise the approval card goes to the owner's DM; the group status card shows 「等你批准」 and 「已发到主人私聊」, with no buttons. |
| Captcha (`robroz-action:captcha`, `Captcha`) | `Takeover`. Task becomes `blocked`, `waitingFor` is `takeover`. No approval card: the owner must complete the captcha in the DM, then `POST /api/tasks/{id}/resume`. | A sentence rule cannot auto-allow a captcha. The takeover flow is unchanged, and the card does not expire. |
| Host-side shell, pull request merge, sending a message, deleting, and spending | Not a `SideEffect` when the host or bro initiates them outside the container prompt. Bros can already ask with `POST /api/groups/{id}/approvals` (`action` is `delete`, `send`, or `spend`); answering that card does not make the host perform the action. There is no first-class host shell or merge tool on `main` today. | Reviewed before any such action *runs*. A matching rule allows; otherwise an approval card goes to the owner's DM. When host shell or merge tools are added, they use this gate. Existing `send` / `delete` / `spend` / `call` verbs are reused where they already fit. |

## What already exists on `main`

Verified on `91efdde`.

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
  endpoints.
- **[#103](https://github.com/PandaQuQ/robroz/pull/103) (merged)**: the client
  renders MCP `call` approval cards with a dedicated layout (the 「调用」 tag is
  not repeated; the host summary already starts with 「调用」). Other approval
  kinds keep the generic card (「批准」「拒绝」). The MCP settings page and
  stdio command approval (`POST /api/mcp/servers/{id}/approve-command`, button
  「批准运行」) are a different card, on the settings page, not this gate.
- **Today's gap**: approval events are stored and streamed in the *task's
  group*, so the interactive card currently appears in the group thread next to
  the status card. Auto-review moves the interactive card to the owner's DM.
  Cards do not yet expire, and approval events have no `reason` field.

## State flow

Reuse the frozen states and the existing endpoints. No new `state`, no new
`waitingFor` value, no change to `POST /api/tasks/{id}/resume`.

1. A risky action is about to run. The host reviews it (confirmed rules, then a
   card if needed). Review returns allow or block, plus a one-sentence reason.
2. **Allow** (the model matched a confirmed rule, or the owner approved the
   card): the action runs. For an in-flight task this is the same continue path
   as today (markers, then `queued`, then pump).
3. **Block** (no match, or review failed after the retry): the task, if any,
   becomes `blocked` with `waitingFor` set to `approval`. In the *group*, the
   status card shows 「等你批准」 and 「已发到主人私聊」, and has no button. The
   approval card is created in the owner's DM with that bro
   (`POST /api/bros/{id}/dm` already opens or returns that conversation). The
   card reuses the [#103](https://github.com/PandaQuQ/robroz/pull/103) approval
   card, plus a grey line for `reason` (why it was blocked). Group messages
   cannot approve it.
4. **Owner approves** (`POST /api/approvals/{id}/approve`): same as today's
   approve path. The card records `decision: approved`, `answeredAt`, and
   optional `answeredBy`.
5. **Owner denies** (`POST /api/approvals/{id}/deny`): same as today's deny
   path. The task becomes `done` with `failure`「你拒绝了：…」. The action does
   not run. The bro must not retry, reword, or work around it in this task.
6. **Card expires** (24 hours by default, configurable): the host settles the
   unanswered approval card as `decision: denied` and
   `reason: APPROVAL_EXPIRED_REASON` (「审批已过期」), with `answeredBy`
   unset. If the card was tied to a task, the task becomes `done` with
   `failure`「审批已过期」. A later approve or deny returns the existing
   409「已经答过了」. The client follows the same already-answered path:
   lock the buttons, refetch the card, then show the expired presentation.
   This is also final: no retry, no workaround, no 「重新发起」.
7. Captcha stays on `waitingFor: takeover` and `POST /api/tasks/{id}/resume`.
   Auto-review does not issue a card, does not auto-allow it, and does not
   expire it.

A bro-issued `delete` / `send` / `spend` card that does not drive a task still
follows rules 3–6: the card goes to the owner's DM, and a deny or expiry is
final for that request. Answering still does not make the host perform the
action (unchanged).

## Client

### Allow-rules settings page

A new left-nav entry, 「放行规则」, directly under 「MCP」.

- The top of the page has one input and an 「添加」 button. Clicking it does
  not add the rule. It creates a confirm card in the owner's DM; the owner
  must confirm before the rule appears in the list.
- Each listed rule is one row: the original sentence, a grey line under it
  「添加于 {日期}」 (from `createdAt`), and a 「删除」 text button on the
  right. There is no toggle and no edit. To change a rule, delete it and add
  it again, each with its own confirm card.
- Deleting also creates a confirm card. The confirm card's action button is
  red and labelled 「删除规则」. The rule stays in the list until that card is
  approved.
- Empty state: 「还没有放行规则。有风险的操作都会先问你。」

### Cards in chat

- **Group thread**: show only 「等你批准」 and 「已发到主人私聊」. No approve
  or deny buttons.
- **Owner DM, blocked action**: reuse the [#103](https://github.com/PandaQuQ/robroz/pull/103)
  approval card, plus a grey line for `reason` (why it was blocked).
- **Owner DM, add or delete a rule**: reuse the [#68](https://github.com/PandaQuQ/robroz/pull/68)
  confirm-card layout on an approval event whose `action` is `rule_add` or
  `rule_delete`. Title 「添加放行规则」 or 「删除规则」; body is the rule
  sentence, verbatim; grey line states add or delete.
- **Owner DM, expired card**: the stored decision is `denied`, but the card
  must not say 「我已拒绝」 or 「已拒绝」, and must not use the red denied
  dot — the owner never clicked. Top-right: a grey dot and 「已过期」.
  Footnote: 「没人批准，已过期」 and the time as two spans, with no 「·」.
  Buttons are locked. There is no 「重新发起」. The client detects this
  case with `reason == APPROVAL_EXPIRED_REASON` and must not hard-code the
  string.

## New API (additive only)

No existing fields change. No existing endpoint changes its request, response,
or status mapping. Auth is the existing API token, the same as the rest of
`/api/settings/*`.

### Allow rules

`ReviewRule`: `id`, `text` (the original sentence, trimmed, non-empty, one
line), `createdAt` (used to render 「添加于 {日期}」). Only confirmed rules
have this shape in the list. Unconfirmed proposals are not listed.

| Method | Path | Role |
| --- | --- | --- |
| `GET` | `/api/settings/review-rules` | List confirmed rules only. Response: `{"rules":[ReviewRule]}`. |
| `POST` | `/api/settings/review-rules` | Propose a rule. Body: `{"text":"…"}`. Returns **202** `{"approvalId":"..."}`. Creates a `rule_add` card in the owner's DM. The rule is not listed and is not used in review until that card is approved. |
| `DELETE` | `/api/settings/review-rules/{id}` | Propose deleting a confirmed rule. Returns **202** and the pending approval id, same body as `POST`. Creates a `rule_delete` card in the owner's DM. The rule stays listed until that card is approved. 404 if the id is not a confirmed rule. |

A bro who calls `POST` or `DELETE` gets the same 202 and a card; the list still
does not change until the owner confirms. There is no edit endpoint.

### Approval decision

Keep `POST /api/approvals/{id}/approve` and `/deny` as the only way to answer
an approval or confirm card. Do not add a parallel decision API. Request body,
`answeredBy`, 404「审批卡不存在」, and 409「已经答过了」 stay as they are.
After expiry, a later answer returns that same 409「已经答过了」. The client
uses the existing already-answered path (lock buttons, refetch, then show the
expired presentation). There is no new 409 copy and no separate client branch.

### Additive fields

- `expiresAt` (RFC 3339, same style as `createdAt` / `answeredAt`): optional
  on approval events, omitted on cards that do not expire so old JSON stays
  valid. Set on task approval cards, MCP call cards, and rule add/delete
  cards. Absent on takeover, and on memory and skill confirm cards.
- `reason` (string, optional): on a blocked action's approval card, the
  one-sentence reason from the review model, shown as grey text on the DM
  card. When the card expired, `reason` is exactly 「审批已过期」. That
  string is a planned contract constant, `APPROVAL_EXPIRED_REASON`, added
  by the backend. The host writes the constant; the client compares
  `reason == APPROVAL_EXPIRED_REASON` and must not hand-write the string.
  Omitted when there is no reason (old JSON stays valid).

Both are new optional fields. They are not changes to any existing field.

### `ApprovalAction`

Two new values, additive only, on `crates/robroz-contract/src/lib.rs`
`ApprovalAction` (today: `delete`, `send`, `spend`, `call`, with
`#[serde(rename_all = "lowercase")]`, and no `#[serde(other)]`).

The JSON names must be `rule_add` and `rule_delete`. Because of
`rename_all = "lowercase"`, a variant named `RuleAdd` would serialise as
`ruleadd`. The new variants need explicit `#[serde(rename = "rule_add")]` and
`#[serde(rename = "rule_delete")]`.

## Implementation status

On `main` after the auto-review PR (`feat/auto-review`):

- Done: `rule_add` / `rule_delete` in the contract, with `ReviewRule`,
  `ReviewRules`, `CreateReviewRule`, `ReviewRuleCreated` and
  `REVIEW_RULE_MAX`. `GET` / `POST` / `DELETE /api/settings/review-rules`
  (202 plus a confirm card; approve applies, deny or expiry discards).
- Done: expiry of task, MCP `call`, bro-issued and rule cards
  (`ROBROZ_APPROVAL_TTL_SECS`, default 24 hours), settled as `denied` +
  `APPROVAL_EXPIRED_REASON`, task `done` + 「审批已过期」, later answers 409.
- Done: review of task external-write cards and MCP `call` cards against the
  confirmed rules (`host/src/review/`), retry once, then block; allow
  continues with no card; block shows the card with `reason`.
- Done: rule cards raised from the settings page are shown on the settings
  page only, not in any chat or unread count, and work with no bro. They use
  `botId` `SETTINGS_APPROVAL_BOT_ID` (`"settings"`) and `botName`
  `SETTINGS_APPROVAL_BOT_NAME` (「设置」), and the client reads them with the
  new `GET /api/approvals/{id}`. A card under review returns 404 there and
  on `approve` / `deny`.
- Done: task and MCP `call` approval cards go to the owner's DM with the
  task's bro (created if that bro has none; the task's group if the bro is
  unknown). The group's status card keeps `waitingFor: approval` and shows
  `progress` 「已发到主人私聊」 (`APPROVAL_SENT_TO_OWNER_DM_NOTICE`). A task
  that runs in a DM keeps its card there. Bro-issued cards stay where they
  were posted.
- Not yet: review of
  bro-issued `delete` / `send` / `spend` cards (they only expire). A bro
  proposing a rule in conversation, with the card in that bro's DM with the
  owner: there is no bro path for proposing rules yet, so it is not built. The client's expired and rule-card layouts
  ([#120](https://github.com/PandaQuQ/robroz/pull/120)).

## Rollout

Verified on `main` at `0aed709` (the `ApprovalAction` enum is unchanged since `91efdde`), `crates/robroz-contract/src/lib.rs` (the
`ApprovalAction` enum). There is no `#[serde(other)]` today, so an unknown
`action` value fails the whole event parse; the client does not fall back to a
generic card.

The review-layer PR must add `#[serde(other)] Unknown` (or equivalent) to
`ApprovalAction` and ship that in at least one release *before* any host emits
`rule_add` or `rule_delete`. Already-shipped clients will still fail to parse
those events until they are updated. Emitting the new values in the same
release as the catch-all is not enough for clients that are already out.

Until that catch-all has shipped, rule add/delete cards must not be emitted.
Host shell and merge, when added, keep using existing verbs (`call`, `send`,
`delete`, `spend`) until any further action values are accepted and shipped
the same way.

## Authorization

The only inputs that can allow a risky action are:

- a confirmed allow rule that the review model matched, or
- the owner tapping approve on the card in their DM.

These never count, even if they look like an instruction to allow:

- group messages, including `/say` and @-mentions
- words from another bro
- a web page, search result, or any tool output
- a rule that has not been confirmed, including one a bro proposed

The host uses a single shared token and cannot tell which person called an
API (`answeredBy` is client-reported and not verified). That does not weaken
the rule: `POST` and `DELETE` on review rules only create a card, and only the
owner's confirm on that card adds or removes a rule or allows an action.

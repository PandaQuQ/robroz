# Permissions, approvals and takeover

What a bro may do, when it stops to ask a person, and how a person takes over. The tiers live in `crates/robroz-policy`; they are applied in `host/src/pump.rs`, `host/src/approvals.rs`, `host/src/tools.rs` and `host/src/mcp_tools.rs`. API details are in [api.md](api.md).

The desktop client UI is English by default. Switch to 中文 with the Language / 语言 row on the Settings (Model) page. `ROBROZ_LANG=en|zh` overrides the saved choice for one run and is not written back. Quoted UI text below is what the English UI shows; the Chinese is in parentheses where it helps. See [configuration.md](configuration.md) for the saved `lang` file.

## Permission tiers

`robroz_policy::tier(SideEffect)` puts each side effect into one of three tiers:

| Tier | Side effects | What happens |
| --- | --- | --- |
| `Allow` | Read and write the group folder (`WorkspaceRead` / `WorkspaceWrite`) | Nobody is notified. On success the host writes `tasks/{taskId}/result.txt` and the task finishes. |
| `Metered` | Outbound requests (`Outbound`), including `web_search` / `web_fetch`, and MCP tools marked `readOnlyHint: true` | GET only, counted per task, cap `ROBROZ_GET_LIMIT` (default 8). A read-only MCP tool counts as one outbound GET each time it is called, runs immediately, and does not raise an approval card. Using the full limit is still a success; going over it, or using a method other than GET, removes the container, ends the task as `done` with a `failure`, and does not issue a delivery card. The task is not moved to `blocked`. |
| `Takeover` | Writing outside (`ExternalWrite`, including MCP tools not marked `readOnlyHint: true`) and captchas (`Captcha`) | The host does not fill in credentials. The task becomes `blocked`. An external write in the prompt also raises a task approval card; a write MCP tool raises an approval card with `action` `call` (see below). A captcha still needs a person to take over in the DM and then `resume`. |

How the meter works: outbound counts are stored in the group folder at `tasks/{taskId}/outbound.txt` and tallied per task by `robroz_policy::meter`. The (N+1)th GET, where N is the limit, ends the task, and `failure` is 「出网超额：已用 N+1 次，上限 N」 (assembled in `robroz-policy`; with the default cap that is 「出网超额：已用 9 次，上限 8」). A non-GET is 「出网只允许 GET：用了 POST」 when the method was `POST`. Those full policy sentences are not in `host_text` and are not mapped yet; the client shows Couldn't finish: plus the stored Chinese text. Method matching is exact: `GET` is allowed, `get` is not.

## Blocking lines

`robroz-action:external` and `robroz-action:captcha` lines in the prompt are blocking lines. The task stops when it reaches one. Whether it gets stuck depends only on whether the prompt contains such a line, not on what the model replied.

Where it stopped is computed by the host, not trusted from the container:

1. When the runner stops, it writes `tasks/{taskId}/intent` in the group folder. That file only means “this run is stuck”; the host never reads its contents.
2. After the container exits, the host takes the prompt stored in the database and the hashes in `ROBROZ_CONTROL/{taskId}/takeover`, and finds the first blocking line whose sha256 is not already in that takeover file (`robroz_policy::blocking_line`; the runner uses the same function to decide where to stop).
3. The computed kind, exact line and hash are stored on the task as `intent_kind` / `intent_line` / `intent_hash`, then the `intent` file in the group folder is deleted (it is also cleared again before the next container start).
4. If an `intent` file is present but no such line can be computed (the container claimed it was stuck, but there is no blocking line that has not already been taken over): treat it as a dead run: `done`, `failure` is Finished with no result (「没写出结果」), not `blocked`.

The takeover marker `takeover`:

- It lives in the control directory, one hash per line, written only by the host. The container only sees the file for its own task, mounted read-only at `/control`.
- The runner only reads `/control/takeover`. It skips blocking lines whose hash is listed there, and those skipped lines are not written into `result.txt`. Any other blocking line still stops the run; taking over again appends another hash.
- A takeover file anywhere under `/workspace` (including one a bro writes itself) is ignored.
- If the same line text appears more than once in the prompt, one takeover allows every copy. The stored prompt is not rewritten.
- Opening an older database adds the three `intent_*` columns automatically; existing rows are left as they are.

## Resume

`POST /api/tasks/{id}/resume` only accepts a task in `blocked`. On success the host appends the stored `intent_hash` to `takeover`, clears the stuck record, and returns the task to `queued` with `containerUp` false. Resume does not read the group folder and does not start a container; the pump later starts a new container on the same group folder. The container is still `--network none` and does not mount `docker.sock`. A task blocked on an MCP tool cannot be resumed this way: the call returns 409 Answer the approval card first (「请先回答审批卡」). A task that is not `blocked` returns 409 This one can't be resumed (「这次不能恢复」). Status codes and error strings are in [api.md, resume](api.md#post-apitasksidresume).

## Approval cards

Approvals are checked against your Allow rules (「放行规则」) first, including cards a bro issues itself. If a rule allows the action, no card is shown; the action goes ahead (a task card is settled as approved, so the task returns to `queued`), and `GET /api/approvals/{id}` can still read it (`decision` is `"approved"`; approve or deny later returns 403 with `reason` `decided`). If the card is blocked or there are no rules, it is shown with `expiresAt` set. A card that expires is denied with `reason` Approval expired (「审批已过期」); a task it was gating ends as `done` with that same `failure`. Confirm cards and a takeover do not expire.

### Who may answer, and where the card goes (v0.2)

Every bro has an owner (the user who created it; bros from a v0.1 database belong to the bootstrap admin). **Only the bro's owner may approve or deny its cards.** An admin is not an exception, except for Allow-rule confirm cards on the settings page, which any admin answers. Each approval card carries `ownerUserId` and `ownerName` so the client can show who decides.

- A card raised in a **group** stays in that group, with its buttons. The owner's DM with that bro gets an `approval_reminder` line: no buttons, it names the group and the bro, counts as unread in the DM, and changes to approved / denied / expired when the card is settled. The v0.1 behaviour of moving the card to the DM and showing Sent to owner's DM (「已发到主人私聊」) in the group is gone.
- A card raised in the bro's **DM** stays in the DM, with its buttons, and there is no reminder.
- Anyone else who tries to answer gets 403 `{"code":"approval_forbidden","reason":"not_owner",…}`. A card that was already answered gets 403 with `reason` `decided`, and one that expired gets 403 with `reason` `expired`. These were 409 before v0.2. A 403 is never a token problem; a bad token is still 401. Details are in [api.md, approve / deny](api.md#post-apiapprovalsidapprove-and-deny).
- The same owner rule covers renaming a bro, opening its DM and adding it to another group (403 `not_bro_owner`, with `ownerUserId` / `ownerName`; the client shows "Only {owner} can change this bro.").

### Cards a bro issues

`POST /api/groups/{id}/approvals`, with `action` `delete`, `send` or `spend`. The card is only a question; answering it does not make the host do the action. Bro-issued cards go through auto-review like a task card. If the card is blocked or there are no rules, it stays where the bro posted it (`host/src/approvals.rs:bro_card_group`), and a group card gets a reminder in the owner's DM (see above). Fields and errors are in [api.md](api.md#post-apigroupsidapprovals).

### Cards the host issues for a task

When a task reaches `robroz-action:external`, the host issues an approval card for it; the person no longer has to take over in the DM first:

- `action` is `send`, `summary` is Writes to an outside service: {what it will do} (「要对外写入：要做的事」), `botId` is the task's assignee, and the optional field `taskId` is set (a card a bro issues itself does not have `taskId`).
- The task is `blocked` with `waitingFor` set to `approval`. The card stays in the task's group (or DM), and a group card gets a reminder in the owner's DM (a DM is created if needed), see above.
- Approve: the host records the takeover (appends the hash) and the task is queued again so it can finish the run.
- Deny: the task ends as `done` without doing that step, there is no delivery card, and the status card has `failure` You denied: … (「你拒绝了：…」).
- If the task was already taken over with `/resume`, answering the card only records the decision and does not change the task again.
- A captcha (`robroz-action:captcha`) does not raise an approval card, because someone has to type the response. It stays “blocked, take over in the DM”.

### MCP tools

When the model wants to call an MCP tool that is not marked `readOnlyHint: true`, the host raises an approval card first (`host/src/mcp_tools.rs`, `host/src/approvals.rs:gate_mcp_task`) and does not make the call:

- `action` is `call`, `summary` is still 「调用 {server}.{tool}」 plus a short parameter summary (the client shows Call {server}.{tool}), and the optional fields `mcpServer`, `mcpTool` and `taskId` are set.
- The task is `blocked` with `waitingFor` set to `approval`, and the progress line is Using {server}.{tool} (「在用 {server}.{tool}」). The card is placed the same way as a task approval card (in the task's group, with a reminder in the owner's DM).
- Approve: the task returns to `queued` and the pump continues: it makes this call and hands the result back to the model. The call also counts as one outbound GET and shares `ROBROZ_GET_LIMIT` with read-only tools.
- Deny: the call is not made, the task is `done`, the delivery card's `summary` is Didn't run (「未执行」), and `failure` is You denied: {server}.{tool} (「你拒绝了：{server}.{tool}」) on both the status card and the delivery card.
- Answering a second time is 403 with `reason` `decided`. Do not use `POST /api/tasks/{id}/resume` in place of answering the card: that returns 409 Answer the approval card first (「请先回答审批卡」).
- A read-only MCP tool does not raise a card. It is called immediately and counts as one outbound GET.
- A call error (can't connect, timeout, or the server returned an error) does not fail the task: the model is given Call failed: {reason} (「调用失败：{原因}」) and continues. Secrets in the reason are redacted.

Details are in [search-and-mcp.md](search-and-mcp.md#模型怎么调用).

### answeredBy

Who may answer is decided by the bro's owner (above). `answeredBy` is the device or person name the client reports when it answers; the host does not check it. Approval, choice and confirm cards all have this field. It is always present on the event; if it was omitted, the value is `null`. The rules are in [api.md, answeredBy](api.md#answeredby); how the client shows it is in [client.md](https://github.com/PandaQuQ/robroz-client/blob/main/docs/client.md#who-answered).

## waitingFor

Present only while the task is `blocked`:

- `approval`: there is an unanswered task approval card that matches the current blocking line (`robroz-action:external`, or a `call` card for a write MCP tool). Answering that card is enough.
- `takeover`: there is no such approval card (for example a captcha). A person takes over in the DM and then calls `resume`. A write MCP tool cannot be released with `resume`.

`waitingFor` only changes how this status card is shown; it does not add another card.

## Couldn't finish (failure)

When a task is `done` but the work was not finished, the status card has `failure`. If there is also a delivery card, that card carries the same value. A normal result omits the key. Starting the task again clears it.

| Situation | `failure` | Delivery card |
| --- | --- | --- |
| The model call failed | No reply from the model: reason (「模型没回：原因」); the same sentence is written into the result | Yes |
| A task approval card was denied | You denied: … (「你拒绝了：…」) | No |
| An MCP tool approval card was denied | You denied: {server}.{tool} (「你拒绝了：{server}.{tool}」) | Yes, `summary` is Didn't run (「未执行」); the delivery card has the same `failure` |
| Over the outbound limit | 「出网超额：已用 N 次，上限 M」 (not mapped yet; the client shows Couldn't finish: plus this text) | No |
| Outbound used a method other than GET | 「出网只允许 GET：用了 POST」, with the method actually used (not mapped yet; the client shows Couldn't finish: plus this text) | No |
| More than 8 rounds of tool calls | Stopped after 8 rounds of tool calls (「工具调用超过 8 轮」); the same sentence is written into the result | Yes |
| A dead run (container exited non-zero, wrote no result, or disappeared) | Finished with no result (「没写出结果」) | No |
| An approval card expired | Approval expired (「审批已过期」) | No |

When the client sees `failure`, it shows a grey Couldn't finish: reason (「没做成：原因」), not the green Done, and there is no button. The only exception is a dead run: when `failure` is 「没写出结果」, the client shows Finished with no result as it is (`robroz_card::failure_line`). From v0.2 a client renders this from `failureCode` and `failureDetail` (the reason without its prefix) instead of the Chinese `failure`; see [api.md, failure](api.md#failure).

## The model's tool calls

When the host calls the model it can register tools (`host/src/tools.rs`). Those tools use the same three tiers:

- A Takeover MCP tool raises an approval card and is not executed until it is approved. The host's own tools today are Metered (`web_search` / `web_fetch`); any other Takeover tool is not executed.
- A task may request at most 8 model rounds. If the 8th reply is still a tool call, the task ends as `done` with `failure` Stopped after 8 rounds of tool calls (「工具调用超过 8 轮」). An 8th reply that is plain text is accepted.
- A single tool result is at most 16,000 characters. Longer text is cut and `…(truncated)` is appended, still within that 16,000-character cap.
- If no tools are registered, the request does not include a `tools` field, and the behaviour is the same as a call with no tools.

How MCP tools are registered, named and approved is in [search-and-mcp.md](search-and-mcp.md#模型怎么调用).

## MCP stdio command approval

A stdio server's `command` and `args` must first be approved with `POST /api/mcp/servers/{id}/approve-command` before `/test`, `/tools` and registration in the model loop will start that process. Until then those endpoints return 409 `{"error":"command_not_approved","command","args"}`, and the model loop skips that server. HTTP servers do not need this approval. Changing the command or arguments clears the approval, and it must be granted again. See [search-and-mcp.md](search-and-mcp.md#mcp) and [api.md](api.md).

# Multi-agent design: Router, Thread subscriptions, and Watchdog

Status: **Accepted**. Group turns (PR A) shipped in
[#129](https://github.com/PandaQuQ/robroz/pull/129) (`41edf6f`). Follow-up
attachment (PR B, group and DM) and bros waking bros (with a depth cap)
are built; see Implementation status. The internal Router (PR C) is
accepted design and is **not built** on `main`. See [Implementation status](#implementation-status).
Author: PandaQuQ

Existing behaviour below is taken from `main` at `67dde00` (#130) plus [#132](https://github.com/PandaQuQ/robroz/pull/132). This document
does not itself change code or the frozen contract. Where a description of
current code disagrees with [`docs/api.md`](../api.md),
[`docs/concepts.md`](../concepts.md), or `crates/robroz-contract`, those win.
The decisions below are the accepted design.

The three ideas below are adapted from the DeepSeek Harness team's writeup
*Exploring Super-agent Design: Towards a More Autonomous Agent System*
(2026-11-01) and from the behaviour described in the MIT-licensed prototype
`dsh-superagent-prototype`. The published prototype is a research plugin, not an
official DeepSeek product, and is not identical to the writeup. This document
describes that behaviour in our own words, maps it onto robroz as it exists on
`main`, and does not copy prototype code. Sources and licence notes are at the
end.

robroz is a single-machine, multi-bro IM workspace, not a single Super-agent
facing the user. The mappings below keep that product shape: a person still
addresses a named bro with `@name`; every bro in the room gets one turn on
a group message (PR A, shipped in #129); and the four task states stay
frozen.

## 1. Router and Worker

### How the prototype does it

The writeup splits "talking to people and coordinating work" from "doing the
work" into two tiers. A resident Router receives human interaction at every
entry point, keeps global order and memory, and dispatches tasks. It does not
execute work itself, and it need not even have a shell. Workers are separate
sessions with isolated context, a full tool set, and one task each. They run
concurrently. They do not talk to people or stream their reasoning to a
channel; they report status to the Router, and only the Router replies.

The published prototype uses different names for the same split. A Main Session
(`orchestrator`, preset `dsh-superagent-router`) evaluates incoming events,
accepts work as Tickets, and replies. An Executor Session (`worker`, preset
`dsh-superagent-worker`) runs one Ticket in its own directory. The Main Session
has routing and ticket tools, and a smaller tool set; a guard also refuses long
waits, so a blocked routing step cannot delay new work. Workers report
progress and outcomes through a report tool. They do not get tools for asking
or presenting to a person, or for starting other sessions. When a Worker
finishes, the host relays the report to the Main Session, which faces the
human.

### How robroz does it today

Verified on `main` at `67dde00` plus #132. There is no Router session and no Worker pool.

- A person creates work with `POST /api/groups/{id}/say` (`host/src/say.rs`).
  In a DM, every message becomes a new task for that bro (unchanged). In a
  group, every member gets one turn (`host/src/turns.rs`). At most one bro
  speaks first: the one named by a leading `@name` or `＠name` that matches
  a member (`robroz_contract::route_mention` in
  `crates/robroz-contract/src/lib.rs`). That mentioned bro uses today's fast
  path: the mention is stripped from the prompt, the status card is visible
  at once, and the task is returned in `Said.task`. The rest go one at a
  time in join order (`memberships.rowid`). If no one is named, `Said.task`
  is `null` and every member takes a turn in join order. A person's `/say`
  opens a round at `PERSON_DEPTH` 0 (`turns.depth`). A clean bro reply on
  a turn opens a round for the other bros at depth + 1, up to
  `MAX_BRO_DEPTH` 2 (`open_bro_round` in `host/src/turns.rs`).
  `/messages`, `/tasks` and schedules never open a round. Approvals are
  unchanged.
  `POST /api/groups/{id}/tasks` assigns a bot directly, with no dispatch
  step.
- Turns in one round run one after another: the next turn's task is created
  only after every earlier turn's task in that round has left `queued` /
  `running` (it is `done` or `blocked`). Two rounds from two messages do
  not wait for each other. If a bro has left the group when their turn
  comes, the turn is skipped. Later bros see earlier bros' delivered
  results (failure-free deliveries only): each quote is clipped to 2000
  characters, and lines whose trim-start begins with `robroz-` are
  stripped, so a quote cannot trigger an external write, progress, or
  silence on the reader's behalf.
- A non-mentioned bro's task starts hidden (`tasks.turn_hidden = 1`): no
  status card, no unread, and it does not move `lastActivityAt`. It is
  revealed only if the bro speaks. Silence is the whole reply being
  `robroz-silent` or empty (after dropping empty lines and
  `robroz-progress:` lines). For a hidden turn, a failed model call or no
  model configured also counts as silence: the host logs it (`eprintln`)
  and does not show a card. A silent turn ends `done`, stays hidden, posts
  no message, shows no card, adds no unread, and leaves `lastActivityAt`
  unchanged. A pending MCP approval, or `ROBROZ_MODEL=echo`, reveals the
  task and it continues as a normal task. On reveal, `created_at` and
  `updated_at` are set to the reveal time. The `@`-mentioned bro is never
  hidden, so a model error on that path still surfaces as it does today.
- The same assignee both "speaks" and "works". The host pump
  (`host/src/pump.rs`) claims the task, calls the model and tools as that bot,
  starts a container, and later writes the status card (`botId` / `botName` of
  the assignee) and the delivery card into the same group. Several bros in one
  group can each have a running task at once, and each posts its own cards.
  There is no layer that collects worker reports and replies with a single
  voice.
- The pump is a single OS thread (`host/src/main.rs`). One pass starts every
  `queued` task, then inspects every live `running` task, then sleeps one
  second. Each task's model-and-tool loop runs on its own thread
  (`host/src/steps.rs`), at most one in-flight step per task; the pump does
  not wait. The database lock is not held across Docker or the model call,
  so HTTP keeps serving.

### How to change it

Group turns (PR A) shipped in #129 (`41edf6f`). The internal Router (PR C)
is still accepted design and is not built on `main`. The order and done
standards are in [Implementation status](#implementation-status).

Keep the existing `@name` and DM paths as a deterministic fast path: the person
already named the assignee, so the host does not ask a model to re-route that
message.

Router and Worker are only an internal dispatch layer *inside one bro*. They
are not room members, they do not appear in the member list, and they do not
speak in the group. A Worker reports to the bro that dispatched it; that bro
speaks. No agent speaks for another.

Group messages keep today's `@` routing, plus turn-taking (PR A, shipped):

- A group message gives every bro in the room one turn. Each bro decides to
  speak or stay silent. Nothing is answered on anyone else's behalf.
- At most one bro speaks first: the one named by a leading `@name`. That
  bro uses the existing fast path (card visible at once, returned in
  `Said.task`). The rest take their turn one at a time in join order, and
  may stay silent. Later bros see earlier bros' replies (each quoted up to
  2000 characters, with `robroz-` control lines stripped).
- Silence (`robroz-silent` or empty; for a hidden turn, also a model error
  or no model, logged only) produces nothing at all: no message, no card,
  no unread, and `lastActivityAt` is unchanged. A non-mentioned bro's task
  starts hidden and appears only if the bro speaks.
- A person's `/say` starts a round at depth 0. A bro's clean reply on a
  turn starts a round for the other bros one level deeper, up to a cap.
  Direct messages and approvals are unchanged.
- Do not introduce a group-level Router that picks one assignee and posts the
  only user-visible reply.

A Worker is the existing execution path for one task: model, tools, and
sandbox, already isolated per `taskId` under `ROBROZ_CONTROL/{taskId}` and the
group volume. Workers report completion to their bro (today, via the host in
`collect_task`). They must not open their own user-facing channel. Do not give
the internal Router a sandbox or write tools.

### Crates

- `robroz-host`: group turns in `turns.rs` (called from `say.rs` and the
  pump), follow-up attachment in `say.rs`, any per-bro Router loop, and
  how `pump.rs` records reports.
- `robroz-policy`: optional, if turn-taking or dispatch rules should be
  unit-tested next to the existing permission rules.
- `robroz-model`: only if a bro's internal Router uses a model call of its
  own.
- `robroz-runner`, `robroz-mcp`, `robroz-client`, `robroz-ui`, `robroz-card`:
  unchanged if cards and execution stay as they are.

### Contract impact

The frozen task states `queued` / `running` / `blocked` / `done` do not need a
fifth value for this split. `botId` / `botName` keep their current meaning. No
new required fields.

Giving every bro one turn on an unaddressed group message, and attaching
follow-ups instead of always inserting a new task, changes when new task ids
appear. That is an accepted behaviour change of `POST /api/groups/{id}/say`
(see [Decisions](#decisions)). New optional fields are allowed; renaming or
reinterpreting existing ones is not. Delivery events have no `botId` today
(`Event::Delivery` in `crates/robroz-contract/src/lib.rs`); status, choice,
and approval cards do. Replies in the group are posted as the bro that did
the work.

**No contract change** if a first cut only adds an internal Router inside one
bro that still creates the same `queued` task and the same cards after an
explicit `@name` or a DM message, and does not yet implement turn-taking.

## 2. Thread subscription

### How the prototype does it

The writeup treats a GitHub pull request, an IM topic, or an email chain as a
Thread: a time-ordered stream of events. After the Router creates a Worker for
the first event, that Worker subscribes to the Thread. Later events on a
subscribed Thread go straight to the Worker in code, without another Router
decision. The Router still sees everything, but it only judges events that
have no subscriber. Subscription is many-to-many: one Worker may subscribe to
several Threads, and one Thread may have several Workers. A new event fans
out to every subscriber, so Workers that share an environment can see each
other's actions without sending peer-to-peer messages. A Worker that finds an
event out of scope can unsubscribe or hand it back to the Router. Workers
still report conclusions to the Router.

The prototype implements this as a Subscription between a Ticket and a
Thread, held by the Thread. A matching event fans out to every live
subscribed Ticket and does not go through Eval when it is a person's message
that already belongs to that Ticket. Context-only subscriptions deliver
events but do not grant authority and do not resume an ended Ticket. An
Executor can follow a related Thread for context, or drop a Thread it finds
unrelated, except the Thread the Ticket started on and one a Source required
it to follow (for example its pull request).

### How robroz does it today

There is no Thread object, no subscription table, and no fan-out to an
in-flight task.

- The unit of conversation is the group (a DM is a group with one bro). The
  event stream (`GET /api/groups/{id}/events` and `/stream`) delivers every
  card in that group to the client. That is a client subscription, not a
  worker subscription.
- (Before PR B.) Follow-up messages do not attach to a running task. In a DM, every new
  `/say` inserts a new `queued` task, even if the same bro still has a
  `queued`, `running`, or `blocked` task in that group (`host/src/say.rs`).
  In a group, a new `@name` likewise creates a new task. The older task is
  not told. The pump will start the new task as soon as it is claimed, so two
  tasks for the same bro can run in the same group volume at once.
- Approvals, choices, and `resume` (`POST /api/tasks/{id}/resume`) are
  per-task and already targeted, but they are not modelled as Thread
  subscriptions. Scheduled tasks (`host/src/schedule.rs`, `tick` in the pump
  loop) create ordinary new tasks; they do not subscribe anything to later
  events.
- robroz has no GitHub, mail, or webhook intake on `main`
  ([`docs/roadmap-phase2.md`](../roadmap-phase2.md)).

### How to change it

This section is accepted design. Follow-up attachment in a group is PR B
and is built for groups and DMs (`host/src/follow_ups.rs`); external
Threads are not. See [Implementation status](#implementation-status).

Introduce Thread as a host-internal correlation, not as a new user-facing
session.

- The first Thread key is `(groupId, taskId)` for a still-open task
  (`queued`, `running`, or `blocked`). A later key can be a choice or
  approval card, a schedule, or an external source once those exist.
- When a bro already has a still-open task in that group, a follow-up — a
  later `/say` in a DM, or an `@name` of that same bro — attaches to that
  task. The host does not insert a new task. Only genuinely new work creates
  a task.
- A follow-up attaches only to *that bro's* still-open task, not to another
  bro's. Two bros do not both act on one follow-up. Workers do not message
  each other.
- If the follow-up is genuinely new work, outside the current task, the
  existing `@name` or DM path may open a new task.
- A `done` task is not still-open and is not resumed by a follow-up (`resume`
  today only accepts `blocked`; see [`docs/permissions.md`](../permissions.md)).
- Keep this intake on the host. Do not broadcast every group event into every
  worker's model context.

External Threads (pull requests, mail) wait until robroz has those sources.
The same Subscription shape should be able to represent them later.

### Crates

- `robroz-host`: a subscription store, changes to `say.rs` and the pump so a
  running task can receive an input, and any later source adapters.
- `robroz-contract`: only if the `/say` response or a new event type is
  needed (see below).
- `robroz-client` / `robroz-ui`: only if the UI later shows that a message
  was attached to an existing task. Not required for a host-only first cut.
- `robroz-runner`: unchanged, unless an input needs a documented file in the
  control directory.

### Contract impact

Attaching a follow-up to an existing task is accepted (see
[Decisions](#decisions)). `POST /api/groups/{id}/say` today, when it routes,
always inserts a new task and returns it in `Said.task`. After this change,
a follow-up may return the open task instead of a new one. Adding an optional
field (for example `appendedToTaskId`) is allowed. Omitting or redefining
`task`, when a client currently expects a new id, is not: keep returning the
attached task in `Said.task` if that is the work the message belongs to, and
do not change the field's meaning. No new required fields.

A follow-up for a `blocked` or `running` task attaches to that task and does
not create a sibling. It is not a `resume`. `resume` still only accepts
`blocked`, and only after the person answers the card.

**No contract change** if subscriptions are stored and used only inside the
host, and `/say` continues to create a new task as it does now. That would
not yet solve the "every DM line is a new task" problem, so it is a
book-keeping step, not the feature.

Task states stay at the four frozen values. A follow-up does not add a fifth
state. A `done` task is not resumed just because a Thread event arrived.

## 3. Async tools and Watchdog

### How the prototype does it

The writeup replaces a blocking `run()` in the ReAct loop with asynchronous
`start()` and `stop()`. `start()` returns at once; when the background work
finishes, the result arrives as a new event. `stop()` cancels work that has
run too long. The session can keep answering while a long command runs.

Separately, the writeup puts an independent Watchdog session outside the main
system, driven on a timer. Process liveness is not enough: the Watchdog also
asks a model whether each Worker is making real progress and whether it has
left the goal. On a fault it intervenes; on a transient failure it rechecks
until recovery. The writeup also lets maintainers ask the Watchdog why
something failed, or have it edit prompts and trigger rules live.

The published prototype is narrower than that writeup, and should be
described separately:

- Long shell commands in a Worker are moved to background jobs after five
  seconds, so the Worker can read new messages on the next step instead of
  waiting. After a turn, background jobs have a thirty-minute cap; jobs past
  that cap are killed and the Ticket is reported as waiting. Subagents may
  continue. The Main Session uses the same five-second shell and job-wait
  limits, and a guard refuses `sleep` and long waits.
- Failure handling is built into the host: retries with backoff for passing
  provider or network errors, a Retry that starts a new Assignment in the
  same session and directory, and a Restart path that recovers interrupted
  Assignments after the host comes back.
- The documented "hourly check" lives in another DeepSeek Harness profile so
  it can keep running when the agent service is down. It inspects process
  status and can restart the service. That is infrastructure liveness, not
  the writeup's model-judged progress Watchdog. The published docs do not
  describe a separate Watchdog session that edits prompts live.

### How robroz does it today

- Host tools are still synchronous on the step thread. `Tool::call` in
  `host/src/tools.rs` returns a `String` and the model loop (`complete` /
  `complete_loop`) waits for it. One task may call the model at most eight
  times (`MAX_TOOL_ROUNDS`). Each model HTTP call waits up to 120 seconds
  (`crates/robroz-model/src/lib.rs`). A single tool result is capped at
  16,000 characters. MCP tools and web search run inside this loop, before
  the sandbox starts.
- Each task's model-and-tool loop runs on its own thread
  (`host/src/steps.rs`), at most one in-flight step per task. The pump does
  not wait. Claiming a task sets `container_up = 1` before `docker run`, so
  an in-flight model step is not treated as a dead run. HTTP is not blocked
  (the database lock is dropped first; see the comment in `host/src/main.rs`
  and `host/src/pump.rs`).
- The sandbox itself is already detached: `docker run -d` in
  `host/src/sandbox.rs`. Later pump passes inspect the container and collect
  the result. There is no wall-clock timeout that cancels a running
  container or an in-flight step.
- A Watchdog runs in the same pump loop (`host/src/watchdog.rs`). For a
  live task (`running` and `containerUp` true) it may write a hint into the
  existing `progress` field: 「模型和工具还没回，已等 N 分钟」 after
  `ROBROZ_STEP_TIMEOUT_SECS` (default 600), or 「N 分钟没有新进度」 after
  `ROBROZ_STALL_SECS` (default 900). It only logs and hints; it does not
  change state, cancel the step, or stop the container.
- A container that exits non-zero, writes no result file, or disappears
  mid-run is a dead run. It ends as `done` + `failure` (`robroz_card::NO_RESULT`
  「没写出结果」). There is no fifth state, the task is not re-queued, no
  new container is started, and `resume` returns 409 ("这次不能恢复"). The
  write is a compare-and-set, only while the task is still `running` (and
  for leftover cleanup, `containerUp` is also false). The card shows the
  grey line 「没写出结果」, not 「没做成：没写出结果」
  (`robroz_card::failure_line`). The host implements this in
  `host/src/pump.rs` as `Outcome::Dead` and in `host/src/watchdog.rs` for
  leftover `running` + `containerUp` false tasks from an older host. A
  `docker run` that fails to start is the exception: the task is put back
  to `queued`.
- `tick` only creates scheduled tasks. It skips a fire if the previous run is
  still `queued`, or `running` with `containerUp` true.
- `blocked` already waits for a person (approval or takeover). That is not a
  stall; the Watchdog must not re-dispatch it.

### How to change it

Part of this has landed on `main` in #118 (`41c8e7f`): per-task step
threads, Watchdog hints on `progress`, and the dead-task transition
(`done` + `failure`, card 「没写出结果」). What has not landed: cancelling
a timed-out step or stopping the container; a start/cancel/result shape
for long tools; a host timeout that ends a still-running model call as
`done` + `failure`.

A host-side timeout that *cancels* the step can still land separately. The
dead-task transition below has already shipped with the card-copy change.

**Async tools (and a non-blocking pump):**

- Stop waiting on the pump thread for a long tool or a long model loop.
  Run each task's model loop off the pump thread (one in-flight loop per
  task is enough). The pump continues to start other containers and collect
  exits.
- Give long tools a start/cancel/result shape inside the host. `start`
  returns at once; the result is pushed into the same task's model loop as
  the next turn when the work finishes; `cancel` stops work that has
  exceeded a limit. Until that lands, at least bound every tool and model
  call with a timeout so the pump cannot stall indefinitely.
- Do not stream tool traces into the group. Progress already has a one-line
  field for "在用 {tool}".

**Watchdog:**

- Add a periodic loop next to `tick` and `pump`, still inside `robroz-host`,
  not a user-facing bro and not a tool-using agent that may edit prompts or
  policy.
- Detect three conditions on running work: a timeout (a container, model
  loop, or tool loop that has been up too long); a container that is gone;
  and stuck progress (a live container whose `progress` has not changed for
  a configured interval).
- On a finding, write a log and a hint on the status card. The Watchdog does
  not itself change task state, except for the dead-task rule below.
- A host timeout that already surfaces as today's `done` + `failure` (for
  example 「模型没回：…」, already used when the model call fails) stays a
  host/pump path. It is not a Watchdog state change, does not change
  dead-run semantics by itself, and can ship first.
- Auto-requeue (putting a dead or stalled run back to `queued` and starting
  again) is out of scope. A Worker may already have written to the group
  volume; `pump.rs` already refuses to rerun a missing container for that
  reason. A rerun is the user's choice. The Watchdog must not resume a
  `blocked` task either.
- A dead task (`running` and `containerUp` false, no result file) ends as
  `done` + `failure`. The card shows 「没写出结果」. This overturns the
  node 4 rule above, and is accepted (see [Decisions](#decisions)). There
  is no fifth state.
  - The state transition and the `robroz-card` grey branch and copy
    (「没写出结果」) must ship in the same PR, never alone. Today's client
    shows `done` + `failure` as 「没做成：原因」; this case must not be
    displayed that way.
  - The write must be a compare-and-set, applied only while the task is
    still `running`, `containerUp` is false, and no result file exists, so
    a late result and the Watchdog cannot both write.

### Crates

- `robroz-host`: pump concurrency, tool start/cancel, the Watchdog loop,
  timeouts.
- `robroz-model`: per-call timeout already exists; may need cancellation or
  a non-blocking client if the loop moves off-thread.
- `robroz-mcp`: if MCP tool calls become start/cancel instead of a blocking
  `call`.
- `robroz-policy`: optional timeout and stall rules, so they can be tested
  without HTTP or Docker.
- `robroz-contract` and `robroz-card`: for the accepted dead-task change,
  `robroz-card` (today the grey `Running` + `container_up == false` branch
  and the 「没写出结果」 copy) must change in the same PR as the host, never
  alone, so the card still shows 「没写出结果」 after the task is `done` +
  `failure`. A new host timeout can use the existing grey 「没做成：原因」
  path (`client/src/model.rs`, [`docs/client.md`](../client.md),
  [`docs/permissions.md`](../permissions.md)) without a new card type and
  without waiting for that dead-task PR. Optional fields may be added;
  no new required field.
- `robroz-runner`: unchanged unless the runner must watch a cancel flag.

### Contract impact

Do not add a fifth task state. `failure` on a `done` status and delivery card
is already the documented way to say the work did not succeed.

Ending a dead run (`running`, `containerUp` false, no result) as `done` +
`failure`, with the card still showing 「没写出结果」, is accepted (see
[Decisions](#decisions)). That overturns the node 4 rule cited above and
reinterprets a documented state combination. The host change and the
`robroz-card` grey branch and 「没写出结果」 copy must land in the same PR,
never separately, and the update must be a compare-and-set as described
above. Docs that state the current rule
([`docs/concepts.md`](../concepts.md), [`docs/api.md`](../api.md),
[`docs/permissions.md`](../permissions.md),
[`docs/client.md`](../client.md), [`docs/architecture.md`](../architecture.md))
must be updated in that same PR.

No new required field, and no new meaning of `progress`, `containerUp`, or
`waitingFor`. Optional fields may be added (for example a stall hint that is
only present when set) without changing old JSON.

Auto-requeue of a dead or stalled run would also be a contract change
(`resume` is refused today, and dead runs are not re-queued). It is not
proposed. A rerun is the user's choice.

**No contract change** for: moving the model loop off the pump thread
(shipped in #118); adding a timeout that still surfaces as today's
`done` + `failure` 「模型没回：…」 on a call that already fails that
way (after PR C; #118 only hints); and a Watchdog that only logs and
adds a card hint (shipped in #118, with the dead-task change).

## Next phase, and what we will not do

This table is the recommended implementation order after this document
merges. It is not a schedule.

| Item | In the next phase? | Why |
| --- | --- | --- |
| Turn-taking on a group message: the bro named by a leading `@name` first (existing fast path), the rest one at a time in join order | Done (#129, `41edf6f`) | Every person's group `/say` opens a round (`host/src/turns.rs`). A silent turn posts nothing, shows no card, adds no unread, and leaves `lastActivityAt` unchanged. See [Implementation status](#implementation-status). |
| Follow-ups attach to that bro's own still-open task; only genuinely new work creates a task | Done (PR B, #130) | A DM `/say`, or a group `@name`, to a bro with a visible `queued` / `running` / `blocked` task there joins it (`host/src/follow_ups.rs`); the task's next step sees it. |
| A bro's message also wakes the other bros, with a round cap on recorded `depth` | Done (#132) | As in Grok Bot, so bros can reply to each other. At the cap the round wraps up, and bros speak only when necessary. Today only a person's `/say` opens a round (`PERSON_DEPTH` 0). |
| Router and Worker as an internal dispatch layer inside one bro | Not yet (PR C, after A and B) | Workers are not room members; they report to that bro; that bro speaks. Today's `@name` and DM paths stay as the fast path. At most three steps; fallback to one Worker; `progress` 「2/3 步」. |
| Non-blocking model and tool loops, so one long call cannot stall the pump | Done (#118) | Each task's loop runs on its own thread (`host/src/steps.rs`); the pump does not wait. Tools themselves are still synchronous on that thread. |
| New timeout on a model, tool, or live container, surfaced as today's `done` + `failure` (「模型没回：…」) without changing dead-run semantics | After PR C | A host/pump path, not a Watchdog state change. #118 only hints; it does not cancel the step or stop the container. Not a contract change (see section 3). |
| Host Watchdog that detects timeout, a gone container, and stuck progress, then writes a log and a card hint | Done (#118) | Hints go in the existing `progress` field. Defaults: `ROBROZ_STEP_TIMEOUT_SECS` 600, `ROBROZ_STALL_SECS` 900. Does not change state except for the dead-task rule, and does not cancel the step. |
| Ending a dead `running` task as `done` + `failure`, with the card showing 「没写出结果」 | Done (#118) | Compare-and-set while still `running`. `Outcome::Dead` and leftover cleanup write `robroz_card::NO_RESULT`. The card uses `failure_line`, not 「没做成：没写出结果」. No auto-requeue. |
| A group-level Router that picks one assignee for an unaddressed message and posts the only reply | No | Superseded by turn-taking. No agent speaks for another. |
| Auto-requeue of a dead or stalled run | No | The Worker may already have written to the group volume. This is why `pump` does not re-queue a missing container today. A rerun is the user's choice. |
| A fifth task state | No | Frozen in `AGENTS.md` and `robroz-contract`. Use `done` + `failure`. |
| Storage and compute split (cloud-stateless loop plus an on-device daemon, as in the writeup's section 5) | No | robroz is a single-machine host plus local Docker. That split does not match the current product and is not needed to land Router, Threads, or a Watchdog. |
| Speculative execution (start work before anyone asked, deliver only if it helps, as in the writeup's section 3.4) | No | It fights the existing permission model and the goal of spending as little human attention as possible. Silence remains the default when a bro chooses not to speak. |
| A Watchdog that is itself a full-tool agent and may edit prompts or policy live | No | Too much authority for a recovery loop. Keep the Watchdog as host code with a fixed policy. |
| Peer-to-peer messages between Workers | No | Workers report to their bro. They do not message each other. |
| Copying prototype code, prompts, or long passages | No | Clean-room: behaviour from the public writeup and docs only. The prototype is MIT-licensed, which is compatible with reading it, but robroz remains AGPL-3.0-only and does not take that code. |
| External Threads (GitHub, mail, webhooks) and two-level event filters for those sources | Not in this pass | There is no such intake on `main`. The Subscription shape should leave room for them, and the work itself belongs with those sources. |
| Inline & Spill / chain of recall (writeup section 3.3) | Not in this pass | Useful later for long-running context; it is not one of the three ideas this document covers. |

## Decisions

These replace the open questions in the draft. They follow Grok Bot's group
behaviour: every bro in the room sees the message and gets one turn; a bro
that has nothing to add stays silent; whoever does the work speaks for
themselves. Group turns (PR A) shipped in #129 (`41edf6f`). Follow-up
attachment (PR B) and the internal Router (PR C) are accepted design, not
built on `main`. The PR order and done standards are in
[Implementation status](#implementation-status).

1. **Group turns and follow-ups.** A group message gives every bro in the
   room one turn. Each bro decides to speak or stay silent. Silence
   (`robroz-silent` or empty; for a hidden turn, also a model error or no
   model, logged only) produces nothing at all: no message, no card, no
   unread, and `lastActivityAt` is unchanged. At most one bro speaks first:
   the one named by a leading `@name`, on the existing fast path; the rest
   take their turn one at a time in join order. A non-mentioned bro's task
   starts hidden and appears only if the bro speaks. A person's `/say`
   starts a round; a bro's clean reply starts a deeper round for the other
   bros, up to a depth cap. Direct messages and approvals are unchanged by the
   group-turns PR. A follow-up in the group attaches to that bro's own
   still-open task (`queued`, `running`, or `blocked`); the host does not
   create a new task. Only genuinely new work creates a task. The follow-up
   rule is built for a group `@name` and for DMs (PR B). A bro's reply
   also wakes the other bros, capped on the recorded `depth` (built after
   PR B).
2. **Who speaks, and where Router sits.** In a group, whoever does the work
   replies themselves. No agent speaks for another. Router and Worker are
   only an internal dispatch layer inside one bro: their background workers
   are not room members, they report to that bro, and that bro speaks. Group
   messages use the existing `@` routing plus the turn-taking rule above.
   `botId` / `botName` keep their current meaning. No new required fields.
3. **Dead tasks.** A dead task (`running` and `containerUp` false) ends as
   `done` + `failure`. The card shows 「没写出结果」. There is no
   auto-requeue; a rerun is the user's choice. There is no fifth task state.

Hint budgets shipped in #118: `ROBROZ_STEP_TIMEOUT_SECS` defaults to 600,
`ROBROZ_STALL_SECS` to 900. Budgets that *cancel* a timed-out step are not
chosen here; pick those in the later timeout PR. A follow-up on a
`blocked` task attaches to that task and does not create a sibling; it is
not a `resume`.

## Implementation status

Verified on `main` at `67dde00` plus #132 (`host/src/turns.rs`, `host/src/follow_ups.rs`, `host/src/say.rs`,
`host/src/pump.rs`, `host/src/unread.rs`).

- Done ([#129](https://github.com/PandaQuQ/robroz/pull/129), merged as
  `41edf6f`): group turns. Every person's group `/say` opens a round with
  one turn per member. At most one bro speaks first: the one named by a
  leading `@name` or `＠name` (`robroz_contract::route_mention`). That
  mentioned bro uses today's fast path (mention stripped from the prompt,
  status card visible at once, task returned in `Said.task`). The rest go
  one at a time in join order (`memberships.rowid`). If no one is named,
  `Said.task` is `null` and every member takes a turn in join order. Turns
  in one round run sequentially: the next task is created only after every
  earlier turn's task is `done` or `blocked`. Later prompts quote earlier
  bros' failure-free deliveries (each clipped to 2000 characters, with
  lines whose trim-start begins with `robroz-` removed). A non-mentioned
  turn starts hidden (`tasks.turn_hidden = 1`: no status card, no unread,
  `lastActivityAt` unchanged) and is revealed only if the bro speaks. The
  whole reply `robroz-silent` or empty (after dropping empty lines and
  `robroz-progress:` lines) is silence. For a hidden turn, a failed model
  call or no model configured also counts as silence, logged only, with
  no card. A silent turn ends `done`, stays hidden, posts no message,
  shows no card, adds no unread, and leaves `lastActivityAt` unchanged.
  A pending MCP approval, or `ROBROZ_MODEL=echo`, reveals the task (and
  sets `created_at` / `updated_at` to the reveal time) and it continues
  as a normal task. The `@`-mentioned bro is never hidden, so a model
  error on that path still surfaces as it does today. A person's `/say`
  opens a round at `PERSON_DEPTH` 0, and each turn records `depth`.
  `/messages`, `/tasks` and schedules never open a round (bro-opened
  rounds are described below). Direct messages, the `@` direct path, and approvals
  are unchanged. No contract change: `Said.task` is still the mentioned
  bro's task or `null`.

  **Done when (met in #129):** in a group with two bros, one message with
  no `@` gives both bros a turn, and at least one of them can choose to
  stay silent (posts nothing).

- Done (PR B, [#130](https://github.com/PandaQuQ/robroz/pull/130)): follow-ups (`host/src/follow_ups.rs`, called from
  `say.rs` and the pump). A follow-up is a person's DM `/say`, or a group
  `/say` with a leading `@name`, when that bro has a still-open task there: the
  newest one in `queued`, `running`, or `blocked` that is not a hidden turn
  task. The host stores it in `follow_ups` and creates no task and no card.
  `Said.task` is the joined task (current state, original prompt) and the
  new optional `Said.appendedToTaskId` is its id. The task's next model
  step sees it: a `queued` task gets `追问：` plus the text appended to its
  prompt before it starts; a `running` task finishes the current step and
  delivers as usual, then, if it wrote a result, goes back to `queued` on
  the same task id with its last reply (control lines dropped) and the
  follow-up in the prompt. One status card, one delivery per step. A
  `blocked` task stays blocked (a follow-up is not a `resume`); after the
  card is answered, the next step sees it (after an approved MCP call, the
  step after that one). If the step fails, is a dead run, or the card is
  denied, the task ends as today and the follow-up goes unanswered; there
  is no requeue. A `done` task is not still-open, so an `@name` to it
  starts a new task. In the round the follow-up opens, the named bro's
  turn is the joined task (no extra hidden turn task); the other bros take
  their turns after it leaves `queued` / `running`, as for any `@name`.
  Not follow-ups: group messages with no `@`, and an `@` to another bro.
  A turn also waits while its bro has any other task in the group still
  `queued` or `running` (its own task, or another round's turn), so one bro
  never runs twice on the group volume at once; the bros after it keep
  waiting behind it.

  **Done when (met in #130):** a follow-up in the group attaches to that
  bro's open task and does not start a new one.

- Done ([#132](https://github.com/PandaQuQ/robroz/pull/132), after PR B): bros wake bros (`open_bro_round` in
  `host/src/turns.rs`, called from the pump right after a delivery). When
  a task that is a turn in a group round delivers a clean reply (no
  `failure`, no `needsSetup`, not `robroz-silent` or empty), the host
  opens a round for the other bros at the parent round's `depth` + 1. The
  turn's `speaker` column records who opened it.
  - The speaker gets no turn in its own round. A bro whose turn in the
    parent round has not started yet is left out too; that turn already
    quotes the reply, so it would otherwise get two turns for one reply.
  - A leading `@name` of another member moves that bro to the front, as a
    hidden turn. It is not a fast-path task and never a follow-up;
    follow-ups stay person-only.
  - Turn mechanics are those of PR A and PR B: sequential, quoting earlier
    replies (2000 characters, `robroz-` lines stripped), hidden until the
    bro speaks, silence rules, and waiting while the bro is busy. The
    prompt names the speaking bro and asks for `robroz-silent` unless the
    bro is asked directly, must correct something, or has something
    essential to add.
  - Cap: `MAX_BRO_DEPTH` is 2. A reply in a depth-2 round is delivered and
    opens nothing, so the conversation closes out.
  - Tasks with no turn (`POST /api/groups/{id}/tasks`, schedules), failed
    or needs-setup deliveries, silence, and DMs open nothing. No contract
    change.

  **Done when (met in #132):** a bro's reply wakes the other bros one level deeper, the
  speaker is never re-woken, and nothing opens past `MAX_BRO_DEPTH`.

- Not yet (PR C, after A and B): the per-bro Router and Workers.
  Router and Worker stay an internal dispatch layer *inside one bro*.
  Group behaviour from PR A and PR B is unchanged.

  1. The bro's model splits the task into Workers, at most three steps
     per task.
  2. Each step is a sub-step inside the task: one status card, and one
     merged delivery at the end. The contract is unchanged.
  3. The Router uses the bro's own configured model. If the model errors
     or cannot split the work, fall back to one Worker, behaving exactly
     as today.
  4. `progress` shows a line of the form 「2/3 步」.
  5. If any step fails, the whole card ends as `done` + `failure`
     「没写出结果」, with no requeue.

  **Done when:** a task can be split into at most three sub-steps with
  one status card and one delivery; a failed or impossible split falls
  back to one Worker; a failed step ends the task as 「没写出结果」 and
  does not requeue.

- Not yet (after PR C): a timeout that *cancels* a still-running step or
  stops the container (#118 only hints).

## Sources and attribution

The ideas in this document are adapted from the following public sources.
Descriptions above are paraphrases of documented behaviour, not reproductions
of source text or code.

- DeepSeek Harness team, *Exploring Super-agent Design: Towards a More
  Autonomous Agent System* (original Chinese title 《Super-agent
  设计探索：迈向更自主的 Agent 系统》), 2026-11-01. English translation:
  <https://chinamodelapi.com/super-agent-design-deepseek>. Chinese text:
  <https://vigenz.com/posts/deepseek-superagent-zh-full/>.
- Prototype repository: <https://github.com/deepseek-ai/dsh-superagent-prototype>,
  documents `docs/000` through `docs/008`. The repository `LICENSE` is the
  MIT License, copyright 2026 DeepSeek Harness Team. GitHub's `licenseInfo`
  may show as empty; the `LICENSE` file is authoritative. The README states
  that the prototype is for reference, is not an official DeepSeek product,
  and is not identical to the writeup.

robroz behaviour cited above was checked against the tree at `67dde00` plus #132 on
`main`, in particular `host/src/say.rs`, `host/src/turns.rs`,
`host/src/pump.rs`, `host/src/unread.rs`,
`host/src/main.rs`, `host/src/tools.rs`, `host/src/steps.rs`,
`host/src/watchdog.rs`, `host/src/sandbox.rs`,
`host/src/progress.rs`, `host/src/schedule.rs`,
`crates/robroz-contract/src/lib.rs`, `crates/robroz-model/src/lib.rs`,
`crates/robroz-card/src/lib.rs`,
[`docs/architecture.md`](../architecture.md),
[`docs/concepts.md`](../concepts.md),
[`docs/permissions.md`](../permissions.md),
[`docs/client.md`](../client.md), and
[`docs/api.md`](../api.md).

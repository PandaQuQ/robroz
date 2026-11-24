# Multi-agent design: Router, Thread subscriptions, and Watchdog

Status: **Accepted**  
Author: PandaQuQ

Existing behaviour below is taken from `main` at `d431948`. This document
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
addresses a named bro with `@name`; a group message with no `@` gives every
bro in the room one turn; and the four task states stay frozen.

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

Verified on `main` (`d431948`). There is no Router session and no Worker pool.

- A person creates work by naming a bro. `POST /api/groups/{id}/say`
  (`host/src/say.rs`) stores the message, then maybe inserts one `queued` task.
  In a DM, every message becomes a new task for that bro. In a group, a task is
  created only when the message starts with `@name` or `＠name` that matches a
  member (`robroz_contract::route_mention` in `crates/robroz-contract/src/lib.rs`).
  An unaddressed group message is stored and nothing is dispatched. One message
  routes to at most one bro. `POST /api/groups/{id}/tasks` assigns a bot
  directly, with no dispatch step.
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

Keep the existing `@name` and DM paths as a deterministic fast path: the person
already named the assignee, so the host does not ask a model to re-route that
message.

Router and Worker are only an internal dispatch layer *inside one bro*. They
are not room members, they do not appear in the member list, and they do not
speak in the group. A Worker reports to the bro that dispatched it; that bro
speaks. No agent speaks for another.

Group messages keep today's `@` routing, plus turn-taking:

- A group message with no `@` gives every bro in the room one turn. Each bro
  decides to speak or stay silent. Nothing is answered on anyone else's
  behalf.
- If the message `@`-mentions a bro, that bro speaks first. Other bros may
  still take their turn afterwards and may stay silent.
- Do not introduce a group-level Router that picks one assignee and posts the
  only user-visible reply.

A Worker is the existing execution path for one task: model, tools, and
sandbox, already isolated per `taskId` under `ROBROZ_CONTROL/{taskId}` and the
group volume. Workers report completion to their bro (today, via the host in
`collect_task`). They must not open their own user-facing channel. Do not give
the internal Router a sandbox or write tools.

### Crates

- `robroz-host`: turn-taking and follow-up attachment in `say.rs`, any
  per-bro Router loop, and how `pump.rs` records reports.
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
- Follow-up messages do not attach to a running task. In a DM, every new
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

**No contract change** for: moving the model loop off the pump thread;
adding a timeout that still surfaces as today's `done` + `failure`
「模型没回：…」 on a call that already fails that way (this can be done
first, after this document lands); and a Watchdog that only logs and adds a
card hint, until the dead-task PR lands.

## Next phase, and what we will not do

This table is the recommended implementation order after this document
merges. It is not a schedule.

| Item | In the next phase? | Why |
| --- | --- | --- |
| Router and Worker as an internal dispatch layer inside one bro | Yes | Workers are not room members; they report to that bro; that bro speaks. Today's `@name` and DM paths already name an assignee and stay as the fast path. |
| Turn-taking on a group message with no `@`; the `@`-mentioned bro speaks first | Yes | Accepted. Each bro gets one turn and may stay silent. This changes `/say` behaviour (see [Decisions](#decisions)). |
| Follow-ups attach to that bro's own still-open task; only genuinely new work creates a task | Yes | Accepted. A DM or a second `@name` today starts a parallel task in the same group volume, with no link to the work already running. |
| Non-blocking model and tool loops, so one long call cannot stall the pump | Done (#118) | Each task's loop runs on its own thread (`host/src/steps.rs`); the pump does not wait. Tools themselves are still synchronous on that thread. |
| New timeout on a model, tool, or live container, surfaced as today's `done` + `failure` (「模型没回：…」) without changing dead-run semantics | Yes, and first | A host/pump path, not a Watchdog state change. #118 only hints; it does not cancel the step or stop the container. Not a contract change (see section 3). |
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
themselves.

1. **Group turns and follow-ups.** A group message with no `@` gives every
   bro in the room one turn. Each bro decides to speak or stay silent. If the
   message `@`-mentions a bro, that bro speaks first. A follow-up attaches to
   that bro's own still-open task (`queued`, `running`, or `blocked`); the
   host does not create a new task. Only genuinely new work creates a task.
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

robroz behaviour cited above was checked against the tree at `d431948` on
`main`, in particular `host/src/say.rs`, `host/src/pump.rs`,
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

# Multi-agent design: Router, Thread subscriptions, and Watchdog

Status: Draft  
Author: PandaQuQ

This is a design proposal only. It does not change code, the frozen contract, or
any schedule. Nothing here is a commitment to a phase or a date. Where this
document disagrees with [`docs/api.md`](../api.md),
[`docs/concepts.md`](../concepts.md), or `crates/robroz-contract`, those win.

The three ideas below are adapted from the DeepSeek Harness team's writeup
*Exploring Super-agent Design: Towards a More Autonomous Agent System*
(2026-11-01) and from the behaviour described in the MIT-licensed prototype
`dsh-superagent-prototype`. The published prototype is a research plugin, not an
official DeepSeek product, and is not identical to the writeup. This proposal
describes that behaviour in our own words, maps it onto robroz as it exists on
`main`, and does not copy prototype code. Sources and licence notes are at the
end.

robroz is a single-machine, multi-bro IM workspace, not a single Super-agent
facing the user. The mappings below keep that product shape: a person still
addresses a named bro with `@name`, and existing task states stay frozen.

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

Verified on `main` (`1acbae1`). There is no Router session and no Worker pool.

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
  second. `start_task` runs the model-and-tool loop to completion
  (`write_reply` → `tools::complete` / `complete_resume`) before `docker run -d`.
  That loop blocks the pump thread, so another queued task waits. The database
  lock is not held across Docker or the model call, so HTTP keeps serving.

### How to change it

Keep the existing `@name` and DM paths as a deterministic fast path: the person
already named the assignee, so the host should not ask a model to re-route
that message.

Add a Router only where dispatch is missing or contested:

- Unaddressed group messages, or messages that name a designated dispatcher
  bro, go to a Router. The Router decides whether work is needed and which
  bot should do it, then the host inserts a normal `queued` task for that
  assignee. The first version can be host-native rules (no model). A later
  version may call a lightweight model that is allowed only to return dispatch
  decisions, not to run workspace tools or start a sandbox.
- A Worker is the existing execution path for one task: model, tools, and
  sandbox, already isolated per `taskId` under `ROBROZ_CONTROL/{taskId}` and
  the group volume.
- Workers report completion to the host, which already happens in
  `collect_task`. For unaddressed work, only the Router (or the host on its
  behalf) should post the user-visible reply. A delivery for a task the person
  sent with `@name` may still appear as that bro, which matches today's IM
  model. Workers must not each answer the same unaddressed message.
- Do not give the Router a sandbox or write tools. Do not let Workers open
  their own user-facing channel.

### Crates

- `robroz-host`: dispatch in `say.rs`, any Router loop, and how
  `pump.rs` records reports.
- `robroz-policy`: optional, if dispatch rules should be unit-tested next to
  the existing permission rules.
- `robroz-model`: only if the Router uses a model call of its own.
- `robroz-runner`, `robroz-mcp`, `robroz-client`, `robroz-ui`, `robroz-card`:
  unchanged if cards and execution stay as they are.

### Contract impact

The frozen task states `queued` / `running` / `blocked` / `done` do not need a
fifth value for this split. `botId` / `botName` stay as they are.

**Needs Panda's approval before merge if implemented:**

- Changing `/say` so an unaddressed group message can create a task. Today it
  never does. That is a behaviour change of an existing endpoint, even if the
  JSON shape of `Said` is unchanged.
- Changing who is allowed to post a delivery or a conversational reply for
  work the Router dispatched (for example, posting it without the worker
  bot's identity, or suppressing a worker's own reply). Delivery events have
  no `botId` today (`Event::Delivery` in `crates/robroz-contract/src/lib.rs`);
  status, choice, and approval cards do. Any change to those existing fields'
  meaning is a contract change. New optional fields are allowed; renaming or
  reinterpreting existing ones is not, unless Panda agrees.

**No contract change** if the first cut only adds an internal dispatcher that
still creates the same `queued` task and the same cards after an explicit
`@name` or a DM message, and leaves unaddressed group messages alone.

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

- The first Thread key is `(groupId, taskId)` for an open task (`queued`,
  `running`, or `blocked`). A later key can be a choice or approval card, a
  schedule, or an external source once those exist.
- When the host creates a task, it subscribes that task to its Thread. A
  later `/say` in a DM, or an `@name` of a bro that already has an open task
  in that group, fans out to the subscribed task as an input instead of
  always inserting a new task. If several tasks subscribe to the same Thread,
  every subscriber receives the event. Workers do not message each other.
- If the follow-up is outside the current task, the worker (or the host)
  unsubscribes or hands the message back so the Router or the existing
  `@name` path can open a new task.
- A context-only subscription (for example another task in the same group)
  may receive the event without being allowed to act or to resume a `done`
  task.
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

**Needs Panda's approval before merge if implemented:**

- `POST /api/groups/{id}/say` today, when it routes, always inserts a new
  task and returns it in `Said.task`. Attaching a follow-up to an existing
  task changes when new task ids appear, and may return the open task instead
  of a new one. That is a behaviour change of an existing endpoint. Adding an
  optional field (for example `appendedToTaskId`) is allowed; omitting or
  redefining `task` when a client currently expects a new id is not, unless
  Panda agrees.
- Automatically feeding a new DM message into a `blocked` or `running` task,
  instead of queueing a sibling task, changes concurrency and recovery
  behaviour that clients can already observe on the status card.

**No contract change** if subscriptions are stored and used only inside the
host, and `/say` continues to create a new task as it does now. That would
not yet solve the "every DM line is a new task" problem, so it is a
book-keeping step, not the feature.

Task states stay at the four frozen values. A follow-up does not add a fifth
state. A `done` task should not resume just because a Thread event arrived,
unless a later, approved rule says otherwise (`resume` today only accepts
`blocked`; see [`docs/permissions.md`](../permissions.md)).

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

- Host tools are synchronous. `Tool::call` in `host/src/tools.rs` returns a
  `String` and the model loop (`complete` / `complete_loop`) waits for it.
  One task may call the model at most eight times (`MAX_TOOL_ROUNDS`). Each
  model HTTP call waits up to 120 seconds (`crates/robroz-model/src/lib.rs`).
  A single tool result is capped at 16,000 characters. MCP tools and web
  search run inside this loop on the pump thread, before the sandbox starts.
- The sandbox itself is already detached: `docker run -d` in
  `host/src/sandbox.rs`. Later pump passes inspect the container and collect
  the result. There is no wall-clock timeout on a running container, and no
  deadline on `start_task` as a whole.
- Because the model-and-tool loop runs on the one pump thread, a long call
  delays starting or collecting every other task. HTTP is not blocked
  (the database lock is dropped first; see the comment in `host/src/main.rs`
  and `host/src/pump.rs`).
- Progress text exists (`host/src/progress.rs`, optional `progress` on the
  status card) but nothing judges whether it is moving. A container that
  disappears mid-run stays `running` with `containerUp` false, is not
  re-queued, and `resume` returns 409 ("这次不能恢复"). That behaviour is
  documented in [`docs/concepts.md`](../concepts.md) and implemented in
  `collect_task`. A `docker run` that fails to start is the exception: the
  task is put back to `queued`.
- `tick` only creates scheduled tasks. It skips a fire if the previous run is
  still `queued`, or `running` with `containerUp` true. It does not look for
  stalled workers.
- `blocked` already waits for a person (approval or takeover). That is not a
  stall; the Watchdog must not re-dispatch it.

### How to change it

Treat this as two host changes that can land in either order.

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
- Check running work for: a container that has been up too long; a model or
  tool loop that has been up too long; a live container whose `progress` has
  not changed for a configured interval; and the existing "container is gone"
  case.
- On a timeout or a confirmed stall, stop the container and the model loop.
  The default, contract-safe outcome is to end the task as `done` with a
  `failure` string (the same channel used today for model failure and the
  eight-round cap), and not to invent a fifth state.
- Re-dispatch (putting a dead or stalled run back to `queued` and starting
  again) is a separate decision. It is useful for transient Docker or
  provider failures, and dangerous for a worker that already wrote into the
  group volume. It must not be the silent default, and it must not resume a
  `blocked` task.

### Crates

- `robroz-host`: pump concurrency, tool start/cancel, the Watchdog loop,
  timeouts.
- `robroz-model`: per-call timeout already exists; may need cancellation or
  a non-blocking client if the loop moves off-thread.
- `robroz-mcp`: if MCP tool calls become start/cancel instead of a blocking
  `call`.
- `robroz-policy`: optional timeout and stall rules, so they can be tested
  without HTTP or Docker.
- `robroz-contract` and `robroz-card`: only if a new optional field or a new
  `failure` wording must be shown. The existing grey "没做成：原因" path can
  display a timeout without a new card type.
- `robroz-runner`: unchanged unless the runner must watch a cancel flag.

### Contract impact

Do not add a fifth task state. `failure` on a `done` status and delivery card
is already the documented way to say the work did not succeed.

**Needs Panda's approval before merge if implemented:**

- Ending a dead run (`running`, `containerUp` false, no result) as
  `done` + `failure` instead of leaving it `running`. That reinterprets a
  documented state combination.
- Automatically re-queueing a dead or stalled run. Today those runs are not
  re-queued, and `resume` is refused. Changing that changes existing state
  machine behaviour.
- Any new required field, or any new meaning of `progress`, `containerUp`,
  or `waitingFor`. Optional fields may be added (for example a stall reason
  that is only present when set) without changing old JSON.

**No contract change** for: moving the model loop off the pump thread;
adding a timeout that still surfaces as today's `done` + `failure` "模型没回：…"
on a call that already fails that way; and a Watchdog that only logs until
the state-machine questions above are decided.

## Next phase, and what we will not do

This table is a recommendation for the next design and implementation pass
after this document is reviewed. It is not a schedule.

| Item | In the next phase? | Why |
| --- | --- | --- |
| Host-native Router for cases with no `@name`, plus "Workers report, they do not all answer an unaddressed message" | Yes | This is the gap. Today's `@name` and DM paths already name an assignee and should stay as the fast path. |
| Thread subscriptions so a follow-up can attach to an open task instead of always opening a sibling task | Yes | A DM or a second `@name` today starts a parallel task in the same group volume, with no link to the work already running. |
| Non-blocking model and tool loops, so one long call cannot stall the pump | Yes | Containers are already detached; the model-and-tool loop on the single pump thread is what serialises work. |
| Host Watchdog for timeouts, missing containers, and stuck progress | Yes | There is no wall-clock limit and no stall check. Liveness of the container is not the same as progress. |
| Ending or re-dispatching a dead `running` task | Only with Panda's approval | This changes documented state-machine behaviour. The Watchdog can land as logging, or as `done` + `failure` for new timeout cases, once that change is approved. |
| Changing `/say` so follow-ups do not always create a new task | Only with Panda's approval | Existing endpoint behaviour. Prefer an additive field if one is needed. |
| A fifth task state | No | Frozen in `AGENTS.md` and `robroz-contract`. Use `done` + `failure`, or keep `running` / `blocked` as they are. |
| Storage and compute split (cloud-stateless loop plus an on-device daemon, as in the writeup's section 5) | No | robroz is a single-machine host plus local Docker. That split does not match the current product and is not needed to land Router, Threads, or a Watchdog. |
| Speculative execution (start work before anyone asked, deliver only if it helps, as in the writeup's section 3.4) | No | It fights the existing permission model, the rule that unaddressed group messages do nothing, and the goal of spending as little human attention as possible. Silence is already the default. |
| A Watchdog that is itself a full-tool agent and may edit prompts or policy live | No | Too much authority for a recovery loop. Keep the Watchdog as host code with a fixed policy. |
| Peer-to-peer messages between Workers | No | The prototype already avoids this: shared Threads fan out events to subscribers. |
| Copying prototype code, prompts, or long passages | No | Clean-room: behaviour from the public writeup and docs only. The prototype is MIT-licensed, which is compatible with reading it, but robroz remains AGPL-3.0-only and does not take that code. |
| External Threads (GitHub, mail, webhooks) and two-level event filters for those sources | Not in this pass | There is no such intake on `main`. The Subscription shape should leave room for them, and the work itself belongs with those sources. |
| Inline & Spill / chain of recall (writeup section 3.3) | Not in this pass | Useful later for long-running context; it is not one of the three ideas this document covers. |

## Open questions

These need a decision before implementation PRs are opened, in addition to
the contract items flagged above.

1. Is the Router a host function, or a designated bro that people can see and
   `@`? A hidden host function stays out of the member list; a visible bro
   matches the rest of the IM model but must be prevented from executing.
2. In a group with several bros, if two open tasks subscribe to the same
   follow-up, do both act, or does the host pick one? Fan-out is the
   prototype's rule; robroz may want a tighter default to avoid two deliveries.
3. What is the first timeout budget for a model loop, a tool, and a
   container? This document does not pick numbers.
4. When a follow-up arrives for a `blocked` task, is it queued until the
   person answers the card, or is it visible to the worker immediately?

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

robroz behaviour cited above was checked against the tree at `1acbae1` on
`main`, in particular `host/src/say.rs`, `host/src/pump.rs`,
`host/src/main.rs`, `host/src/tools.rs`, `host/src/sandbox.rs`,
`host/src/progress.rs`, `host/src/schedule.rs`,
`crates/robroz-contract/src/lib.rs`, `crates/robroz-model/src/lib.rs`,
[`docs/architecture.md`](../architecture.md),
[`docs/concepts.md`](../concepts.md),
[`docs/permissions.md`](../permissions.md), and
[`docs/api.md`](../api.md).

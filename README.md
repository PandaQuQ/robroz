<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
    <img src="assets/brand/wordmark-light.svg" alt="robroz" width="240">
  </picture>
</p>

<p align="center"><b>English</b> | <a href="README.zh-CN.md">简体中文</a></p>

<p align="center">
  <a href="https://github.com/PandaQuQ/robroz/actions/workflows/ci.yml"><img src="https://github.com/PandaQuQ/robroz/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
</p>

Self-hosted multi-agent workspace you drive through chat. Agents (bros) run in Docker sandboxes; approvals and results come back as cards. Rust host + native desktop client.

<!-- TODO(screenshot): main window — 最优雅UI姐 will add a real client shot in docs/images/ after the English UI lands. Do not use /workspace/shot/. Do not add an image link until the file exists. -->

The desktop client UI is currently in Chinese; English UI is in progress (English default, Chinese optional).

## Works today

Verified on `main` (`67dde00`).

- **Group turns.** A person's group message gives every bro in the room one turn. The bro named by a leading `@name` goes first, on the existing direct path (status card visible at once). The rest go one at a time in join order and may stay silent (no message, no card, no unread).
- **Follow-ups.** In a group, `@name` of a bro that still has an open task (`queued`, `running`, or `blocked`) attaches to that task instead of starting a new one. The same rule applies in a direct message, without `@`.
- **Direct messages.** A DM is a group with you and one bro. Every message is a task for that bro, unless it attaches as a follow-up.
- **Docker sandbox.** Each task runs in a container (`robroz-runner`) with no network. The group folder is mounted at `/workspace`; the task's control directory is mounted read-only at `/control`.
- **Approvals and Allow rules.** Outbound writes and similar actions wait for the owner. Auto-review checks each card against your allow rules; otherwise the card is sent to the owner's DM with the bro. Memory and skill changes also wait for a confirm card.
  <!-- TODO(screenshot): approval card — same `docs/images/` set, after the English UI lands. -->
- **Model settings.** Any OpenAI-compatible `/chat/completions` endpoint. A base URL and a model name are enough; the key is optional (for example a local Ollama). Configure this in the client or with environment variables; values saved in settings win, field by field.

Also on `main`: three permission tiers (group-folder access, metered GET-only outbound, and anything that needs a person); web search (Brave or SearXNG); MCP tools on the host; scheduled routines; Watchdog hints on a long or stalled step (they do not cancel the work).

## Not yet

These are accepted design or in progress. They are **not** shipped on `main`. Details: [docs/design/multi-agent.md](docs/design/multi-agent.md).

- A bro's message waking the other bros, so bros can reply to each other. In progress; the planned round cap is `MAX_BRO_DEPTH = 2`.
- An internal Router inside one bro (at most three sub-steps, one status card, one delivery).
- Cancelling a timed-out step or stopping the container.

## Quick start

You need a Rust stable toolchain (`rustup`, `cargo`) and Docker. The walk-through in [docs/install.md](docs/install.md) also uses `curl` and `jq`. From the repository root:

```sh
cargo build -p robroz-host -p robroz-client
docker build -f runner/Dockerfile -t robroz-runner:local .
cargo run -p robroz-host     # listens on 127.0.0.1:8790, no token required
cargo run -p robroz-client   # in a second terminal; default host http://127.0.0.1:8790
```

The host writes `./robroz.db`, `./data/groups` and `./data/control` by default. A loopback listener needs no token; listening on a non-loopback address without `ROBROZ_TOKEN` or `ROBROZ_TOKEN_FILE` refuses to start.

In the client, click 「引入一个 bro」 (add a bro) next to the 「聊天」 (Chats) heading, open the DM and send a message. That is your first task. With no model configured, the delivery is 「还没设置模型」 (the model isn't set up yet) and the card offers 「去设置」 (Go to settings).

### Configure a model

<!-- TODO(screenshot): settings page — same `docs/images/` set, after the English UI lands. -->

Open 「模型」 (Model) in the client, or the 「去设置」 (Go to settings) link on a delivery card. Fill in the address and the model name; the key is optional. Saving tests the connection. You can also set environment variables and keep the key in a file outside the repository:

```sh
mkdir -p "$HOME/.config/robroz"
(umask 077 && printf '%s' '<your-key>' > "$HOME/.config/robroz/model-key")
ROBROZ_MODEL_BASE_URL=https://api.openai.com/v1 \
ROBROZ_MODEL=gpt-4o-mini \
ROBROZ_MODEL_KEY_FILE="$HOME/.config/robroz/model-key" \
cargo run -p robroz-host
```

`ROBROZ_MODEL=echo` is a test switch (CI uses it) that echoes the prompt when no complete model is configured. Do not set it in a real deployment.

Other host variables you may want: `ROBROZ_STEP_TIMEOUT_SECS` (default `600`) and `ROBROZ_STALL_SECS` (default `900`) control Watchdog hints; `ROBROZ_APPROVAL_TTL_SECS` (default `86400`) is how long an approval card stays open. The full list is in [docs/configuration.md](docs/configuration.md). End-to-end setup, tokens, and a `curl` first-task walk-through are in [docs/install.md](docs/install.md).

## Architecture

The native desktop client (`robroz-client`, [iced](https://iced.rs/) 0.14, Windows, Linux and macOS, no web view) talks to a single-machine host (`robroz-host`) over HTTP and SSE. The host owns SQLite, model and tool calls, search, MCP, approvals and the container pump. Each task runs in a Docker container (`robroz-runner`) with no network; the container never sees model keys. How the pieces fit together: [docs/architecture.md](docs/architecture.md).

| Path | Crate | Role |
| --- | --- | --- |
| `host/` | `robroz-host` | HTTP API (axum), SQLite, container pump, schedules, model and tool calls, search, MCP, auto-review |
| `runner/` | `robroz-runner` | Task executor inside the container |
| `client/` | `robroz-client` | iced desktop client |
| `crates/robroz-contract` | `robroz-contract` | API contract: request and response bodies, event (card) types, task states |
| `crates/robroz-policy` | `robroz-policy` | Permission tiers, outbound metering, blocking lines |
| `crates/robroz-card` | `robroz-card` | Card status text, tone and actions |
| `crates/robroz-tokens` | `robroz-tokens` | Design tokens generated from `tokens.json` |
| `crates/robroz-ui` | `robroz-ui` | Shared client UI layer and theme |
| `crates/robroz-model` | `robroz-model` | OpenAI-compatible `/chat/completions` client with tool calls |
| `crates/robroz-mcp` | `robroz-mcp` | MCP client (stdio and Streamable HTTP) |

"Bro" is the UI name for a bot: code, API and the database still say bot (`botId` / `botName`). The four task states `queued` / `running` / `blocked` / `done` are frozen.

## Documentation

Repository docs are the contract. The [wiki](https://github.com/PandaQuQ/robroz/wiki) is informal; when the two disagree, this repository wins. Rules for contributors and coding agents: [AGENTS.md](AGENTS.md).

| File | Contents |
| --- | --- |
| [docs/install.md](docs/install.md) | Prerequisites, build, run, connect a model, first task, tests |
| [docs/configuration.md](docs/configuration.md) | All environment variables and defaults |
| [docs/api.md](docs/api.md) | HTTP endpoints and events |
| [docs/concepts.md](docs/concepts.md) | Glossary, task states, cards |
| [docs/permissions.md](docs/permissions.md) | Permission tiers, approvals, takeover, `failure` |
| [docs/design/auto-review.md](docs/design/auto-review.md) | Auto-review and allow rules |
| [docs/memory-and-skills.md](docs/memory-and-skills.md) | Memory, skills, confirm cards |
| [docs/search-and-mcp.md](docs/search-and-mcp.md) | Web search and MCP |
| [docs/client.md](docs/client.md) | Client UI behaviour |
| [docs/architecture.md](docs/architecture.md) | Components, containers, theming |
| [docs/reference-features.md](docs/reference-features.md) | Reference products and clean-room rules |
| [docs/roadmap-phase2.md](docs/roadmap-phase2.md) | Later roadmap |
| [docs/design/multi-agent.md](docs/design/multi-agent.md) | Group turns, follow-ups, Router and Watchdog design |

Most files under `docs/` are still Chinese-first. They will be switched to English in follow-up PRs; quoted UI strings stay in the language the client currently shows.

## Status

There is no tagged release yet. Use `main` as a development branch, not as a formal release.

## Development

Run the same commands as CI before opening a pull request:

```sh
cargo test -p robroz-mcp --features fixture
cargo test -p robroz-host -p robroz-contract -p robroz-policy -p robroz-card -p robroz-runner -p robroz-tokens
cargo build -p robroz-client
cargo test -p robroz-client -p robroz-ui -p robroz-tokens
scripts/e2e.sh   # needs Docker, curl and jq
```

CI builds and tests the client on Linux for pull requests, and on Windows, Linux and macOS after merging to `main` and on a nightly schedule. A `cargo-deny` job checks that new dependencies use an AGPL-compatible licence and come from a known registry; advisories are reported and do not block the merge. Run the same checks locally after installing [cargo-deny](https://github.com/EmbarkStudios/cargo-deny):

```sh
cargo deny check licenses bans sources
cargo deny check advisories   # reported, does not block merging
```

## Contributing

- One pull request per change.
- Every contributor must sign the [Contributor License Agreement](CLA.md) once: tick the CLA checkbox in the pull request template, or add the CLA line from `CLA.md` to the pull request body. Pull requests without it are not merged.
- Commit messages and pull request titles and bodies are in English and follow [Conventional Commits](https://www.conventionalcommits.org/): `feat(host): …`, `fix(client): …`, `docs: …`. Pull requests are squash-merged with the title as the commit message.
- The contract is add-only: new card types, endpoints and optional fields are fine; existing fields, the task states and `botId` / `botName` are frozen.
- No force-push. If your branch is behind, merge `main` into it.
- Never commit `.env`, `.pem`, `.dev.vars`, keys or tokens.
- Keep `README.md` and `README.zh-CN.md` in sync; change both in the same pull request.

The full rules are in [AGENTS.md](AGENTS.md).

## License

SPDX-License-Identifier: `AGPL-3.0-only`

robroz is licensed under the [GNU Affero General Public License v3.0 only](LICENSE). If you modify robroz and let others use it over a network, you must offer them your modified source code (AGPL section 13).

robroz is also available under a commercial licence for closed-source or proprietary use; see [COMMERCIAL.md](COMMERCIAL.md). For commercial licensing, [contact the author](https://github.com/PandaQuQ).

Contributions are accepted under the [Contributor License Agreement](CLA.md).

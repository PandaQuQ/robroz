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

**robroz** is a self-hosted, single-machine multi-agent workspace that you drive through chat, not an IDE. You hand tasks to *bros* (agents) in groups and direct messages; each bro works in a Docker sandbox on your machine, and progress, approvals and results come back to the conversation as cards. The host, the runner and the native desktop client (Windows, Linux, macOS) are all written in Rust, with no web view.

<!-- Screenshots pending: one at 390 px and one at 1440 px, shared by README.md and README.zh-CN.md. -->

## Features

Phase 1 (on `main` today):

- **Groups and DMs.** Start a message with `@name` in a group to give that bro a task; in a DM every message is a task. Status and delivery cards, live updates, unread counts.
- **Three permission tiers.** Reading and writing the group folder is allowed; outbound network is GET-only and counted per task; external writes and captchas stop the task and wait for a person.
- **Cards you answer.** Approval cards, choice cards, and routines (scheduled tasks that deliver a briefing).
- **Memory and skills.** A bro proposes a memory or a skill; it takes effect only after you confirm it.
- **Any OpenAI-compatible model**, web search (Brave or SearXNG), and MCP server configuration on the host (bros cannot call MCP tools yet).

Not in phase 1: file tree, terminal, desktop streaming, bot detail pages, webhooks, E2B, multi-tenancy.

## Quick start

You need the Rust stable toolchain and Docker; the examples also use `curl`. From the repository root:

```sh
cargo build -p robroz-host -p robroz-client                # host and desktop client
docker build -f runner/Dockerfile -t robroz-runner:local .  # runner image used for tasks
cargo run -p robroz-host                                    # listens on 127.0.0.1:8790 only, no token needed
cargo run -p robroz-client                                  # in a second terminal
```

In the client, click 「引入一个 bro」 (*add a bro*) next to the 「聊天」 (*Chats*) heading, open its DM and send a message: that is your first task. Without a model configured, the bro writes your message back as the result.

To connect a model, set a token, reach the host from another machine, or walk through the API with `curl`, see [docs/install.md](docs/install.md).

## Configuration

Everything is configured with environment variables on the host and the client (listen address, token, database, model, search, runner image). The full list with defaults is in [docs/configuration.md](docs/configuration.md). Keep keys and tokens in files outside the repository (`*_FILE` variables); never commit them.

## Documentation

- **For people:** the [robroz wiki](https://github.com/PandaQuQ/robroz/wiki) covers installation, everyday use, FAQ and roadmap.
- **For coding agents and contributors:** [AGENTS.md](AGENTS.md) and [`docs/`](docs/) hold the repository rules, the API contract, fields, environment variables and code locations. The wiki is not the contract; when they disagree, the repository wins.

| File | Contents |
| --- | --- |
| [docs/install.md](docs/install.md) | Build, run, connect a model, first task, tests |
| [docs/configuration.md](docs/configuration.md) | All environment variables |
| [docs/api.md](docs/api.md) | All HTTP endpoints and events |
| [docs/concepts.md](docs/concepts.md) | Glossary, task states, cards |
| [docs/permissions.md](docs/permissions.md) | Permission tiers, approvals, takeover, `failure` |
| [docs/memory-and-skills.md](docs/memory-and-skills.md) | Memory, skills, confirm cards |
| [docs/search-and-mcp.md](docs/search-and-mcp.md) | Web search and MCP configuration |
| [docs/client.md](docs/client.md) | Client UI behaviour |
| [docs/architecture.md](docs/architecture.md) | Components, containers, theming layer |
| [docs/reference-features.md](docs/reference-features.md) | Reference products and clean-room rules |
| [docs/roadmap-phase2.md](docs/roadmap-phase2.md) | Phase 2 roadmap |

"Bro" is the UI name for a bot: code, API and database still say bot (`botId` / `botName`). A DM is a group with only you and one bro.

## Contributing

### Layout

| Path | Crate | Role |
| --- | --- | --- |
| `host/` | `robroz-host` | HTTP API (axum), SQLite, container pump, schedules, model calls, search, MCP configuration |
| `runner/` | `robroz-runner` | Task executor inside the container |
| `client/` | `robroz-client` | iced desktop client |
| `crates/robroz-contract` | `robroz-contract` | API contract: request/response bodies, event (card) types, task states |
| `crates/robroz-policy` | `robroz-policy` | Permission tiers, outbound metering, blocking lines |
| `crates/robroz-card` | `robroz-card` | Card status text, tone and actions |
| `crates/robroz-tokens` | `robroz-tokens` | Design tokens generated from `tokens.json` |
| `crates/robroz-ui` | `robroz-ui` | Shared client UI layer and theme |
| `crates/robroz-model` | `robroz-model` | OpenAI-compatible `/chat/completions` client with tool calls |
| `crates/robroz-mcp` | `robroz-mcp` | MCP client (stdio and Streamable HTTP) |

See [docs/architecture.md](docs/architecture.md) for how they fit together.

### Build and test

Run the same commands as CI before opening a PR:

```sh
cargo test -p robroz-mcp --features fixture
cargo test -p robroz-host -p robroz-contract -p robroz-policy -p robroz-card -p robroz-runner -p robroz-tokens
cargo build -p robroz-client
cargo test -p robroz-client -p robroz-ui -p robroz-tokens
scripts/e2e.sh   # needs docker, curl and jq
```

CI builds the client on Linux for pull requests, and on Windows, Linux and macOS after merging to `main` and nightly.

### Rules

- One PR per change.
- Commit messages and PR titles and bodies are in English and follow [Conventional Commits](https://www.conventionalcommits.org/): `feat(host): ...`, `fix(client): ...`, `docs: ...`. PRs are squash-merged with the PR title as the commit message.
- The contract is add-only: new card types, endpoints and optional fields are fine; existing fields, the task states (`queued` / `running` / `blocked` / `done`) and `botId` / `botName` are frozen.
- No force-push. If your branch is behind, merge `main` into it.
- Never commit `.env`, `.pem`, `.dev.vars`, keys or tokens. Use placeholders in docs.
- Keep `README.md` and `README.zh-CN.md` in sync; change both in the same PR.

The full rules for agents and contributors are in [AGENTS.md](AGENTS.md).

## License

To be decided.

# Install and first run

How to install the robroz host from a GitHub Release or build it from source, run it, connect a model, and send a first task. Installing and running the desktop or mobile client is in the client repository's [install.md](https://github.com/PandaQuQ/robroz-client/blob/main/docs/install.md). The full list of environment variables is in [configuration.md](configuration.md).

Quoted UI text below is what the English client shows; the Chinese is in parentheses where it helps.

## Download a release

When a `v*` tag is published, download the archives and `SHA256SUMS.txt` from the [GitHub Release](https://github.com/PandaQuQ/robroz/releases) for that tag. The current release is v0.1.1, published on 2026-11-25 as a pre-release. The examples below use v0.1.1; replace it with the tag you download. Archives are named `<bin>-<tag>-<target>`.

| Platform | Archive | Contents |
| --- | --- | --- |
| Linux x86_64 | `robroz-host-v0.1.1-linux-x86_64.tar.gz` | `robroz-host` |

The host is packaged for Linux x86_64 only. The v0.1.1 release also has the client archives; later client releases come from [robroz-client](https://github.com/PandaQuQ/robroz-client/releases), see its [install.md](https://github.com/PandaQuQ/robroz-client/blob/main/docs/install.md#download-a-release). Each archive also includes `LICENSE` and `README.md`. After unpacking, the binary is inside a folder named after the archive. Builds are not code-signed. The Linux binary is built on Ubuntu 22.04 and needs glibc 2.34 or newer, so it also runs on RHEL 9. Check your version with `ldd --version`.

Verify the files you download against `SHA256SUMS.txt` from the same release:

```sh
# Linux (only the files you downloaded need to be present)
sha256sum -c SHA256SUMS.txt --ignore-missing

# macOS: print the hash and compare it with the matching line in SHA256SUMS.txt
shasum -a 256 robroz-client-v0.1.1-macos-aarch64.zip
```

On Windows, compare `Get-FileHash -Algorithm SHA256 .\robroz-client-v0.1.1-windows-x86_64.zip` with the matching line in `SHA256SUMS.txt`.

To see which build you are running, use `./robroz-host --version` (or `-V`). It prints `robroz-host <version> (<short git sha>)` and exits with code 0 before it reads the token, opens the database or talks to Docker. The flag is in builds from `main` after v0.1.1; the v0.1.1 binary does not have it.

The host's default runner image is still `robroz-runner:local`. For a tagged release, point it at the image published for that tag (linux/amd64 only; there is no `latest` tag):

```sh
ROBROZ_IMAGE=ghcr.io/pandaquq/robroz-runner:v0.1.1 ./robroz-host
```

## One-command install (Docker)

On a Linux machine with Docker Engine and the `docker compose` plugin, one command installs the host from the published images. This command works from the v0.2.0 release, once the v0.2.0 host image has been published and this script is on `main`. Until then the pull of `ghcr.io/pandaquq/robroz-host:v0.2.0` fails; run the script from a checkout, or wait for that release.

```sh
curl -fsSL https://raw.githubusercontent.com/PandaQuQ/robroz/main/deploy/install.sh | sh
```

Read the script first if you like: [`deploy/install.sh`](../deploy/install.sh). To see what it would do without changing anything, add `--dry-run` (`... | sh -s -- --dry-run`).

The script:

1. Checks that `docker`, `docker compose` and the engine socket work. It stops with a message if not.
2. Creates the install directory, `~/robroz` by default. Change it with `ROBROZ_DIR=/opt/robroz` or `--dir`. The path may only contain letters, digits and `/ . _ -` (no spaces or non-ASCII characters).
3. Writes `compose.yml` (the same file as [`deploy/compose.host.yml`](../deploy/compose.host.yml), with the image tags filled in), a `.env` with placeholders and local settings only, and `secrets/robroz_token`, a random token it generates on your machine. It never asks for a credential and writes none. Your model account goes into the client's settings.
4. Pulls `ghcr.io/pandaquq/robroz-host` and `ghcr.io/pandaquq/robroz-runner` for the version it installs, and runs `docker compose up -d`. If the images for that tag are already on the machine, the pull is skipped. Pass `--no-pull` to never pull (the images must already be present, or the script stops).
5. Prints the URL (`http://127.0.0.1:8790`, this machine only) and the next steps. It does not print the token; it tells you which file holds it. If you want to connect from a phone, set `ROBROZ_BIND` in `.env` to this machine's LAN or Tailscale address and run the script again.

Images are tagged per release and there is no `latest` tag, so the script pins a version (see `DEFAULT_VERSION` at the top of the script). Choose another with `ROBROZ_VERSION=v0.2.1` or `--version v0.2.1`. The script does not call the GitHub `releases/latest` API, because `v0.*` tags are pre-releases and that endpoint skips them. The release workflow fails a tag push if the pin (and the tags in `deploy/compose.host.yml`) do not match the tag: bump both in a PR, merge, then tag. If that tag has no image yet, the pull fails and the script says so; nothing is changed except the files in the install directory.

Run it again to upgrade: it replaces `compose.yml` (the old copy is kept as `compose.yml.bak`), pulls the images for the version it names and recreates the container. It never overwrites an existing `.env`, `secrets/robroz_token` or anything under `data/`, so your database, group folders and settings stay.

The data lives in `<install dir>/data`, mounted at the same absolute path inside the container, because the host starts runners through the engine; see the same-path rule in [docker-host.md](docker-host.md#same-absolute-path-inside-and-outside-the-container). The port is published on `127.0.0.1` only. To reach the host from another machine, put an HTTPS reverse proxy or tunnel in front, as described in [Connecting to a remote host](#connecting-to-a-remote-host). Mounting `docker.sock` is root-equivalent on that machine; read [docker-host.md](docker-host.md#security-dockersock-is-root-on-this-machine) before you run this on a shared server.

The token file is owned by the user who ran the script, with the container's gid as the group and mode 0640. That way you can `cat` it, and the host process can read it, even if your uid is not the container's. The data directory is mode 0700, owned by the container user, and the database file is mode 0600, so other users on the machine cannot read it. A rerun of the script tightens these modes on an older install. The container user is taken from the host image (`USER robroz` in [`host/Dockerfile`](../host/Dockerfile), uid 1000 with a matching group). `deploy/compose.host.yml` does not set `user:`, so that is the process. `group_add` / `DOCKER_GID` only add the engine socket's group; they are not used for the token. Until the image is pulled (for example with `--no-start`), the token stays owned by you with mode 0600, and the script sets 0640 on the next run that can inspect the image.

Options: `--dir DIR`, `--version TAG`, `--dry-run`, `--no-start` (write the files, pull and start nothing), `--no-pull` (do not pull images; they must already be present locally), `-h`. The script refuses a rootless or otherwise non-default Docker socket (`DOCKER_HOST` set, or `/var/run/docker.sock` missing). The error names the reason and links to [the docker.sock section](https://github.com/PandaQuQ/robroz/blob/main/docs/docker-host.md#security-dockersock-is-root-on-this-machine); follow that page by hand for rootless Docker.

### Get the client

The host has no UI. Download the client for your computer from [GitHub Releases of PandaQuQ/robroz-client](https://github.com/PandaQuQ/robroz-client/releases): pick the archive for your platform and verify it against `SHA256SUMS.txt` from the same release (commands as in [Download a release](#download-a-release)). Builds are not code-signed. Then enter the host URL and the token from `secrets/robroz_token` in the client; the client's own [install.md](https://github.com/PandaQuQ/robroz-client/blob/main/docs/install.md#download-a-release) has the per-platform steps. Show the token with `cat ~/robroz/secrets/robroz_token` (no `sudo` needed once the script has set mode 0640).

## Prerequisites

| Need | Used for | Notes |
| --- | --- | --- |
| Rust stable toolchain (`rustup`, `cargo`) | Building the host and the runner | There is no `rust-toolchain` file and `Cargo.toml` does not set `rust-version`. CI uses `dtolnay/rust-toolchain@stable`, so follow current stable. All crates are edition 2021. |
| Docker | The host starts a runner container for each task | The host calls the `docker` CLI (`docker run` / `inspect` / `rm`), so the user who starts the host must be able to run `docker`. Unit tests do not need Docker. |
| `curl`, `jq` | The examples below and `scripts/e2e.sh` | The e2e script requires both. |

## Build

From the repository root:

```sh
cargo build -p robroz-host      # host binary: target/debug/robroz-host
```

Add `--release` for a release build; the binary then lands in `target/release/`.

### Runner image

Each task runs in a Docker container that contains `robroz-runner`. The host's default image name is `robroz-runner:local` (override with `ROBROZ_IMAGE`). For a tagged release you can use `ROBROZ_IMAGE=ghcr.io/pandaquq/robroz-runner:v0.1.1` instead of a local build (linux/amd64 only; there is no `latest` tag). The package is public, so `docker pull` needs no login. To build the image yourself, run this from the repository root, because the Dockerfile copies the workspace:

```sh
docker build -f runner/Dockerfile -t robroz-runner:local .
```

The image is two stages: `cargo build --release -p robroz-runner` in `rust:1-bookworm`, then the binary is copied into `debian:bookworm-slim`. The image's default user is `65534:65534` (nobody), not root; the host passes `--user` when it starts the container.

The host adds these limits at `docker run`:

- `--network none`: the container has no network.
- Two mounts only: the group folder at `/workspace` (writable), and this task's control directory at `/control` (read-only; it holds `prompt.txt`, the model's reply `reply.txt`, and a takeover flag `takeover`).
- One environment variable, `ROBROZ_TASK_ID`. The prompt is not passed in the environment.
- `--user` defaults to the host process's `uid:gid` (omitted on non-Unix). Files written in the group folder then belong to the host user. Override with `ROBROZ_CONTAINER_USER`.

## Run the host

```sh
./target/debug/robroz-host
```

To run the host as a container instead, see [docker-host.md](docker-host.md).

With no variables set:

| Item | Default |
| --- | --- |
| Listen address `ROBROZ_ADDR` | `127.0.0.1:8790` |
| Database `ROBROZ_DB` | `./robroz.db` (SQLite; created if missing) |
| Group folder root `ROBROZ_VOLUMES` | `./data/groups` |
| Control directory root `ROBROZ_CONTROL` | `./data/control` |
| Runner image `ROBROZ_IMAGE` | `robroz-runner:local` |

Relative paths are resolved from the current directory at startup. `ROBROZ_CONTROL` must sit outside `ROBROZ_VOLUMES` (neither path may contain the other), or the host refuses to start.

On a successful start, stderr prints two lines, for example:

```text
robroz-host: 还没设置模型，任务会提示去设置
robroz-host listening on 127.0.0.1:8790（仅本机，无令牌）
```

(The first line means the model is not set up yet, so tasks will tell you to go to settings. The second means the host is listening on loopback, with no token required.)

### Token

- A loopback listener can start without a token.
- A non-loopback address (for example `0.0.0.0:8790`) without a token exits immediately with code 2: `拒绝启动：0.0.0.0:8790 不是本机回环地址，必须先设置 ROBROZ_TOKEN 或 ROBROZ_TOKEN_FILE` (refused: this is not a loopback address; set `ROBROZ_TOKEN` or `ROBROZ_TOKEN_FILE` first).
- The token comes from `ROBROZ_TOKEN` or `ROBROZ_TOKEN_FILE` (a file path). You may set only one. Leading and trailing whitespace is stripped; the token must be at least 16 characters and must not contain whitespace in the middle. Prefer a file, so the token is not left in shell history or the process environment.
- Once a token is set, every `/api` request needs `Authorization: Bearer <token>`, including requests from the same machine. A missing or wrong token returns 401.

```sh
# Create a token file that only you can read
mkdir -p "$HOME/.config/robroz"
(umask 077 && openssl rand -hex 24 > "$HOME/.config/robroz/token")
ROBROZ_ADDR=0.0.0.0:8790 ROBROZ_TOKEN_FILE="$HOME/.config/robroz/token" ./target/debug/robroz-host
```

Keep token files and the model-key file below outside the repository, and do not commit them.

## Connect a model

Until a model is configured, a task delivers No model set up yet (「还没设置模型」). The delivery card has `needsSetup:"model"`, and the client shows Set up (「去设置」). A base URL and a model name are enough; the key is optional. You can fill this in after startup from the client or the settings API (`GET` / `PUT /api/settings/model`), without restarting. CI and e2e set `ROBROZ_MODEL=echo`, which echoes the prompt as the result. That is a test switch; do not set it in a real deployment.

### In the client

Open Model (「模型」) in the sidebar, or Set up (「去设置」) on a delivery card. The Language / 语言 row is on this page: English is the default, and you can switch to 中文. `ROBROZ_LANG=en|zh` overrides the saved choice for one run.

- Three fields: URL (「地址」), Model name (「模型名」), and Key (optional) (「密钥（选填）」). The key field is shown for every provider, including Ollama.
- Picking a provider fills in its OpenAI-compatible address. Example model names: Gemini `gemini-3.8-flash`, Groq `llama-3.3-70b-versatile`, OpenRouter `meta-llama/llama-3.3-70b-instruct:free`, Ollama `llama3.2`; Custom leaves the name empty. For OpenRouter, a grey line under the model name says Free model names end in :free (「免费模型的名字以 :free 结尾」).
- You can save as soon as the address and model name are filled in. A local service such as Ollama can omit the key. Clearing the key does not clear the address or the model name.
- Saving tests the connection. Success shows Working (「能用了」). Failure shows Saved, but can't connect: reason (「已保存，但连不上：原因」): the settings were written, but this attempt could not connect. If the reason is key is wrong or missing (「密钥不对或没填」), the provider returned 401 or 403: the key is wrong, or this service needs a key and you did not provide one. The page does not show a raw 401. The combined line is Saved, but can't connect: key is wrong or missing (「已保存，但连不上：密钥不对或没填」).

Field priority and the API are in [configuration.md, Model](configuration.md#model) and [client.md, Model page](https://github.com/PandaQuQ/robroz-client/blob/main/docs/client.md#model-page).

### With environment variables

To use any OpenAI-compatible `/chat/completions` service, you can set these (the key is optional):

```sh
mkdir -p "$HOME/.config/robroz"
(umask 077 && printf '%s' '<your-key>' > "$HOME/.config/robroz/model-key")
ROBROZ_MODEL_BASE_URL=https://api.openai.com/v1 \
ROBROZ_MODEL=gpt-4o-mini \
ROBROZ_MODEL_KEY_FILE="$HOME/.config/robroz/model-key" \
./target/debug/robroz-host
```

- The key is `ROBROZ_MODEL_KEY_FILE` or `ROBROZ_MODEL_KEY`; you may set only one. Prefer a file. A local service can omit it.
- The base URL must start with `http://` or `https://`. A trailing `/` is stripped.
- A partial environment configuration still starts; you can finish it on the settings page. Setting both key variables, an empty key file, or a key that contains whitespace makes the host refuse to start. The error names the variable, not the value.
- When the environment already has a complete model, the startup log is `robroz-host: 环境变量里有模型 <name> @ <url>，设置页保存的值优先` (a model is set in the environment; values saved on the settings page win). The test switch logs `robroz-host: 测试回显（ROBROZ_MODEL=echo）`. With nothing configured: `robroz-host: 还没设置模型，任务会提示去设置`.

Web search (Brave or SearXNG) can be set with environment variables or through the settings API after the host is running; see [configuration.md, Search](configuration.md#search).

## Run the client

Install or build the client from [robroz-client](https://github.com/PandaQuQ/robroz-client) and follow its [install.md](https://github.com/PandaQuQ/robroz-client/blob/main/docs/install.md#run-the-client). It connects to `http://127.0.0.1:8790` by default. In the client, click Add a bro (「引入一个 bro」) next to the Chats (「聊天」) heading, open the DM and send a message. That is a first task from the UI.

## Connecting to a remote host

The host does not terminate TLS. It will not bind a non-loopback address (`ROBROZ_ADDR=0.0.0.0:8790`) unless a token is set (`ROBROZ_TOKEN` or `ROBROZ_TOKEN_FILE`); otherwise it exits with code 2. Once a token is set, every `/api` request needs `Authorization: Bearer <token>`, including ones from the same machine.

To reach the host from another machine, keep it on loopback (`127.0.0.1:8790`) and put HTTPS in front of it. Still set a token: the proxy makes the API reachable from outside, and a missing token would leave it open. Never expose plain HTTP with the token on the public internet; the token would travel in the clear.

The commands below use the release binary `./robroz-host`; with a source build use `./target/debug/robroz-host`.

### Host in Docker

If the host runs from [`deploy/compose.host.yml`](../deploy/compose.host.yml), it already listens on `0.0.0.0:8790` inside the container with a token from a Compose secret, and the port is published only on `127.0.0.1:8790`. Point the tunnel or the reverse proxy below at `127.0.0.1:8790` the same way. Setup, the `docker.sock` risk and the same-path rule are in [docker-host.md](docker-host.md).

### Cloudflare Tunnel

Install `cloudflared` and point a tunnel at the local host. This is an example config only (replace the hostname and start `cloudflared` with your own tunnel); it is not a real tunnel, and no Cloudflare account is used here.

```yml
ingress:
  - hostname: robroz.example.com
    service: http://127.0.0.1:8790
  - service: http_status:404
```

Start the host on loopback with a token, then run the tunnel:

```sh
mkdir -p "$HOME/.config/robroz"
(umask 077 && openssl rand -hex 24 > "$HOME/.config/robroz/token")
ROBROZ_ADDR=127.0.0.1:8790 ROBROZ_TOKEN_FILE="$HOME/.config/robroz/token" ./robroz-host
# in another terminal: cloudflared tunnel --config /path/to/config.yml run
```

### Reverse proxy (Caddy)

Caddy issues a certificate automatically:

```
robroz.example.com {
    reverse_proxy 127.0.0.1:8790
}
```

### Giving the client the address and the token

Point the client at the proxy address, not at the host's loopback port, and give it the same token. How the client saves the address and the token is in the client repository's [install.md](https://github.com/PandaQuQ/robroz-client/blob/main/docs/install.md#connecting-to-a-remote-host).

## First task

This walk-through uses `curl`: create a group, add a bro, send a message, and read the cards. Build the runner image first, then start the host (default address, no token). If a token is set, add `-H "Authorization: Bearer <your-token>"` to every request.

```sh
B=http://127.0.0.1:8790
J='content-type: application/json'

# 1. Create a group. 201 and {"id","name","tenantId"}
GID=$(curl -s -X POST $B/api/groups -H "$J" -d '{"name":"website"}' | jq -r .id)

# 2. Add a bro: botId is the bro's id; name is what you type after @ in the group
curl -s -X POST $B/api/groups/$GID/members -H "$J" -d '{"botId":"bot-ada","name":"ada"}'

# 3. Speak in the group. A leading @name gives that bro the first turn
#    and returns its task. A message with no @ still gives every bro a turn;
#    Said.task is then null.
curl -s -X POST $B/api/groups/$GID/say -H "$J" -d '{"body":"@ada Write a welcome line"}'
```

Step 3 returns 201. For a leading `@ada`, `task` is that bro's new queued task (unless it attaches as a follow-up, below):

```json
{"message":{"id":"…","kind":"text","body":"@ada Write a welcome line","createdAt":"…"},
 "task":{"id":"…","assigneeBotId":"bot-ada","prompt":"Write a welcome line","state":"queued"}}
```

What happens when you send a group message:

- If no one is named, one bro answers: the one already working with you here, the only bro in the group, the one a short model call picks, else the bro that answered last (or the first who joined). That task is returned in `Said.task`. `ROBROZ_SINGLE_RESPONDER=0` gives every bro a turn instead.
- If a bro is named, every bro in the group gets a turn. The bro named by a leading `@name` or `＠name` (full-width) speaks first, and that task is returned in `Said.task`. The rest go one at a time in join order and may stay silent (no message, no card, no unread).
- The mention must be at the start of the message, followed by whitespace or the end of the line. If several member names could match, the longest wins. A leading `@name` with nothing after it does not start a direct-path task; `task` is `null`.
- If that named bro already has an open task in the group (still in progress, waiting, or blocked), the message is a follow-up: it attaches to that task instead of starting a new one. `Said.task` is the existing task, and `Said.appendedToTaskId` is set. The same rule applies in a direct message, without `@`.
- When a bro posts a clean reply in a group (not silence, not a failed result), the other bros get a turn to join in. The speaker is not asked again, and neither are bros still waiting in the same round. A leading `@name` in that reply moves that bro to the front. This stops after two such rounds (`MAX_BRO_DEPTH = 2`). Silence, a failed result, a DM, `POST /api/groups/{id}/tasks`, and scheduled tasks do not open a new round.

In a DM you do not need `@`; the message goes to that bro, or attaches as a follow-up if the bro already has an open task there.

The host picks up queued work about once a second and starts a container. After a few seconds, look at the cards:

```sh
curl -s $B/api/groups/$GID/events | jq -c '.events[] | {type, state, label, path}'
```

```text
{"type":"message","state":null,"label":null,"path":null}
{"type":"status","state":"done","label":"完成","path":null}
{"type":"delivery","state":null,"label":null,"path":"tasks/<taskId>/result.txt"}
```

- A `status` card is the task's status, with `botId`, `botName`, `state`, `label` and `containerUp`. The API `label` 「完成」 means the task is done; the client shows Done.
- A `delivery` card means the result was written, in the group folder: `./data/groups/<groupId>/tasks/<taskId>/result.txt`. If no model is configured and `ROBROZ_MODEL=echo` is off, the file is still 「还没设置模型」 (the client shows No model set up yet), and the delivery has `needsSetup:"model"`. Only with the test switch on is the file the prompt itself, here `Write a welcome line`.
- The status card may also show a light-grey hint under the bro's name, No reply from the model or tools for N min (「模型和工具还没回，已等 N 分钟」) or No new progress for N min (「N 分钟没有新进度」). The stall hint is only a hint. The No reply hint shows at half of `ROBROZ_STEP_TIMEOUT_SECS`. A step that runs past `ROBROZ_STEP_TIMEOUT_SECS` (default 600 seconds) is cancelled: the container is stopped and the task ends as Finished with no result (「没写出结果」). A grey Finished with no result (「没写出结果」) means the container exited non-zero, wrote no result, or disappeared. The task has already ended as `done`; it is not started again automatically, and `resume` is refused. It is not shown as Couldn't finish: Finished with no result (「没做成：没写出结果」).
- For a live feed of cards, use `GET /api/groups/{id}/stream`. For only what changed, `GET …/events?since=<RFC3339 timestamp>`.

Another way to start is to create a bro; the host also opens a DM for it:

```sh
curl -s -X POST $B/api/bros -H "$J" -d '{"name":"bob"}'
# {"id":"…","name":"bob","dmGroupId":"…"}
curl -s -X POST $B/api/groups/<dmGroupId>/say -H "$J" -d '{"body":"Write a welcome line"}'
```

Other endpoints (approvals, Allow rules, choice cards, scheduled routines, memory, skills, search settings, model settings, MCP servers) are in [api.md](api.md).

## Tests

Unit tests do not need Docker. CI's `host` job runs:

```sh
cargo test -p robroz-mcp --features fixture
cargo test -p robroz-host -p robroz-contract -p robroz-policy -p robroz-runner
```

The client's tests, including the ones that start this host in-process, run in the client repository.

CI also runs a `cargo-deny` job, which checks that dependency licences are compatible with AGPL-3.0 and rejects unknown sources. From the repository root:

```sh
cargo deny check licenses bans sources
```

Install [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) first. New dependencies must use an AGPL-3.0-compatible licence. The allowed licences are listed in the root `deny.toml`. Do not add an incompatible licence to that list. CI also runs `cargo deny check advisories`, which currently does not block a merge.

### End-to-end: `scripts/e2e.sh`

This runs a full flow against a real host binary and real Docker. It is CI's `e2e` job. It needs `docker`, `curl` and `jq`. From the repository root:

```sh
scripts/e2e.sh
```

What it does:

1. Builds the runner image (default tag `robroz-runner:e2e`, override with `ROBROZ_IMAGE`) and `cargo build -p robroz-host`.
2. Puts the database, group folder and control directory in a temporary directory. The host listens on `127.0.0.1:18790` (change the port with `E2E_PORT`). The script sets `ROBROZ_ADDR`, `ROBROZ_DB`, `ROBROZ_VOLUMES`, `ROBROZ_CONTROL` and `ROBROZ_IMAGE` itself, so it does not use your existing values for the first four. `ROBROZ_IMAGE` is the exception: if you already set it, that value is used. It also sets `ROBROZ_MODEL=echo`.
3. Creates a group, adds member `bot-e2e`, and starts a task with two blocking lines (`robroz-action:external` and `robroz-action:captcha`). There is no live model; the runner follows the prompt deterministically.
4. Checks that the task stops as blocked with no delivery; that it is still blocked after the host restarts; that the first `resume` stops on the second line instead of repeating the first; and that the second `resume` finishes the task as `done`, with only three body lines in `result.txt` and exactly one delivery.
5. Restarts once more and checks that a finished task is not replayed: the state, the delivery count and `result.txt`'s modification time stay the same, `resume` on a finished task returns 409, and no container is left behind.

It prints `[e2e] PASS` at the end. On failure it prints the reason and the last 40 lines of the host log. The temporary directory is removed on exit.

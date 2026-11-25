# Install and first run

How to install robroz from a GitHub Release or build it from source, start the host and the desktop client, connect a model, and send a first task. The full list of environment variables is in [configuration.md](configuration.md).

The desktop client UI is English by default. Switch to 中文 with the Language / 语言 row on the Settings (Model) page. `ROBROZ_LANG=en|zh` overrides the saved choice for one run and is not written back. Quoted UI text below is what the English UI shows; the Chinese is in parentheses where it helps. See [configuration.md](configuration.md) for the saved `lang` file.

## Download a release

When a `v*` tag is published, download the archives and `SHA256SUMS.txt` from the [GitHub Release](https://github.com/PandaQuQ/robroz/releases) for that tag. v0.1.0, published on 2026-11-25, is the first release and is marked as a pre-release. Archives are named `<bin>-<tag>-<target>`.

| Platform | Archive | Contents |
| --- | --- | --- |
| Linux x86_64 | `robroz-client-v0.1.0-linux-x86_64.tar.gz` | `robroz-client`, `robroz.desktop`, `robroz.png` |
| Windows x86_64 | `robroz-client-v0.1.0-windows-x86_64.zip` | `robroz-client.exe` |
| macOS aarch64 | `robroz-client-v0.1.0-macos-aarch64.zip` | `robroz.app` |
| Linux x86_64 | `robroz-host-v0.1.0-linux-x86_64.tar.gz` | `robroz-host` |

The host is packaged for Linux x86_64 only. The macOS client is Apple Silicon only; Intel Macs are not covered. Each archive also includes `LICENSE` and `README.md`. `robroz.app` includes `icon.icns`. After unpacking, the binary or `robroz.app` is inside a folder named after the archive. Builds are not code-signed. The Linux binaries are built on Ubuntu 22.04. The client needs glibc 2.35 or newer (Ubuntu 22.04+, Debian 12+). The host needs glibc 2.34 or newer, so it also runs on RHEL 9. The client does not run on RHEL 9 (glibc 2.34); build it from source there, see [Build](#build). Check your version with `ldd --version`.

Verify the files you download against `SHA256SUMS.txt` from the same release:

```sh
# Linux (only the files you downloaded need to be present)
sha256sum -c SHA256SUMS.txt --ignore-missing

# macOS: print the hash and compare it with the matching line in SHA256SUMS.txt
shasum -a 256 robroz-client-v0.1.0-macos-aarch64.zip
```

On Windows, compare `Get-FileHash -Algorithm SHA256 .\robroz-client-v0.1.0-windows-x86_64.zip` with the matching line in `SHA256SUMS.txt`.

The host's default runner image is still `robroz-runner:local`. For a tagged release, point it at the image published for that tag (linux/amd64 only; there is no `latest` tag):

```sh
ROBROZ_IMAGE=ghcr.io/pandaquq/robroz-runner:v0.1.0 ./robroz-host
```

### Linux desktop entry

The Linux client runs straight from the unpacked folder. To get a launcher entry with an icon for your user, copy the binary, the desktop file and the icon into the standard per-user locations:

```sh
cd robroz-client-v0.1.0-linux-x86_64
install -Dm755 robroz-client ~/.local/bin/robroz-client
install -Dm644 robroz.desktop ~/.local/share/applications/robroz.desktop
install -Dm644 robroz.png ~/.local/share/icons/hicolor/512x512/apps/robroz.png
```

`robroz.desktop` runs `robroz-client` from your `PATH`, so `~/.local/bin` must be on it. If the entry does not appear, log out and back in, or run `update-desktop-database ~/.local/share/applications`.

### Unsigned builds

**macOS.** Gatekeeper warns the first time you open an unsigned `robroz.app`. To allow it: right-click the app, choose Open, and confirm; or go to System Settings > Privacy & Security and click Open Anyway. You can also clear the quarantine flag after unzipping:

```sh
xattr -dr com.apple.quarantine robroz-client-v0.1.0-macos-aarch64/robroz.app
```

**Windows.** SmartScreen may show "Windows protected your PC". Click More info, then Run anyway.

In v0.1.0 an extra command-line window opens next to the client. Leave it open; closing it also quits the client. v0.1.1 removes it.

## Prerequisites

| Need | Used for | Notes |
| --- | --- | --- |
| Rust stable toolchain (`rustup`, `cargo`) | Building the host, the client and the runner | There is no `rust-toolchain` file and `Cargo.toml` does not set `rust-version`. CI uses `dtolnay/rust-toolchain@stable`, so follow current stable. All crates are edition 2021. |
| Docker | The host starts a runner container for each task | The host calls the `docker` CLI (`docker run` / `inspect` / `rm`), so the user who starts the host must be able to run `docker`. Unit tests do not need Docker. |
| `curl`, `jq` | The examples below and `scripts/e2e.sh` | The e2e script requires both. |

The client builds on Windows, Linux and macOS. CI builds it on Linux for pull requests, and also on Windows and macOS after a merge to `main` and on a nightly schedule.

## Build

From the repository root:

```sh
cargo build -p robroz-host      # host binary: target/debug/robroz-host
cargo build -p robroz-client    # desktop client: target/debug/robroz-client
```

Add `--release` for a release build; the binaries then land in `target/release/`.

### Runner image

Each task runs in a Docker container that contains `robroz-runner`. The host's default image name is `robroz-runner:local` (override with `ROBROZ_IMAGE`). For a tagged release you can use `ROBROZ_IMAGE=ghcr.io/pandaquq/robroz-runner:v0.1.0` instead of a local build (linux/amd64 only; there is no `latest` tag). The package stays private until the maintainer makes it public after the first release; if `docker pull` is denied, build the image locally as shown below. To build the image yourself, run this from the repository root, because the Dockerfile copies the workspace:

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

Field priority and the API are in [configuration.md, Model](configuration.md#model) and [client.md, Model page](client.md#model-page).

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

The client is a native iced desktop app. A downloaded binary, `robroz.app`, or `cargo run -p robroz-client` all work the same way. It speaks HTTP or HTTPS (rustls) and connects to `http://127.0.0.1:8790` by default.

Fill in Host and Token on the offline pane or in Settings, under Connection (「连接」), then click Connect (「连接」). The address is saved in a `host` file next to `lang` in the client config folder. The token is stored in the OS keyring (macOS Keychain, Windows Credential Manager, Secret Service on Linux), falling back to a `token` file next to `lang` with mode `0600` on Unix. The client never logs or displays the token. A saved token is shown as `••••••••`.

`ROBROZ_HOST` and `ROBROZ_TOKEN` override saved values for this run and are never written back. A field set by the environment is read-only and shows Set by ROBROZ_HOST (「由 ROBROZ_HOST 设置」) or Set by ROBROZ_TOKEN (「由 ROBROZ_TOKEN 设置」). When both are set by the environment, Connect is disabled. The client reads `ROBROZ_TOKEN` only, not the host's `ROBROZ_TOKEN_FILE`. A non-local `http://` host shows Unencrypted. Use HTTPS for remote hosts. (「未加密，远程请用 HTTPS。」)

A 401 is titled Token rejected (「令牌被拒绝」). The reason is The host rejected the token. Check ROBROZ_TOKEN. (「宿主拒绝了令牌，请检查 ROBROZ_TOKEN。」). An unreachable host is titled Can't reach the host (「连不上宿主」); the reason line is Can't reach host. (「连不上宿主。」) followed by the detail. If the host has never answered, the form stays on that pane. After the host has answered once, a later failure keeps what is already on screen and shows the error in the bottom bar.

Linux / macOS:

```sh
cargo run -p robroz-client
# Host elsewhere, or a token is set
ROBROZ_HOST=http://<host-address>:8790 ROBROZ_TOKEN='<your-token>' cargo run -p robroz-client
```

Windows (PowerShell):

```powershell
$env:ROBROZ_HOST = "http://<host-address>:8790"
$env:ROBROZ_TOKEN = "<your-token>"
cargo run -p robroz-client
```

- The group list comes from `GET /api/groups`. If `ROBROZ_GROUPS` is set, the client shows only those groups and does not follow the host's list.
- To look at the UI without a host: `cargo run -p robroz-client -- --fake` (or `ROBROZ_FAKE=1`).

Linux needs a graphical session (X11 or Wayland). CI's `ubuntu-latest` can build the client without extra system packages.

In the client, click Add a bro (「引入一个 bro」) next to the Chats (「聊天」) heading, open the DM and send a message. That is a first task from the UI.

## Connecting to a remote host

The host does not terminate TLS. It will not bind a non-loopback address (`ROBROZ_ADDR=0.0.0.0:8790`) unless a token is set (`ROBROZ_TOKEN` or `ROBROZ_TOKEN_FILE`); otherwise it exits with code 2. Once a token is set, every `/api` request needs `Authorization: Bearer <token>`, including ones from the same machine.

To reach the host from another machine, keep it on loopback (`127.0.0.1:8790`) and put HTTPS in front of it. Still set a token: the proxy makes the API reachable from outside, and a missing token would leave it open. Never expose plain HTTP with the token on the public internet; the token would travel in the clear.

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
(umask 077 && openssl rand -hex 24 > "$HOME/.config/robroz/token")
ROBROZ_ADDR=127.0.0.1:8790 ROBROZ_TOKEN_FILE="$HOME/.config/robroz/token" ./target/debug/robroz-host
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

The client speaks HTTPS (rustls). Point it at the proxy address, not at the host's loopback port.

You can save the address and the token in the client. The address is written to a `host` file next to `lang` in the client config folder (`~/.config/robroz/host` on Linux, `~/Library/Application Support/robroz/host` on macOS, `%APPDATA%\robroz\host` on Windows). The token is stored in the OS keychain (`robroz` / `host-token`: macOS Keychain, Windows Credential Manager, or the Secret Service on Linux); if the keychain is unavailable or the write cannot be read back, it is written to a `token` file next to `lang`, mode `0600` on Unix. The client never logs or displays the token.

`ROBROZ_HOST` and `ROBROZ_TOKEN` override any saved values for this run and are never written back. The client does not read the host's `ROBROZ_TOKEN_FILE` variable.

```sh
ROBROZ_HOST=https://robroz.example.com ROBROZ_TOKEN='<your-token>' cargo run -p robroz-client
```

To seed the fallback file by hand (only needed if you are not saving from the client, and the keychain is not used):

```sh
mkdir -p "$HOME/.config/robroz"
(umask 077 && printf '%s\n' '<your-token>' > "$HOME/.config/robroz/token")
printf '%s\n' 'https://robroz.example.com' > "$HOME/.config/robroz/host"
cargo run -p robroz-client
```

If the host and the client share a machine, the same `$HOME/.config/robroz/token` file can be the host's `ROBROZ_TOKEN_FILE` and the client's fallback file. A missing or wrong token shows "The host rejected the token. Check `ROBROZ_TOKEN`." (「宿主拒绝了令牌，请检查 `ROBROZ_TOKEN`。」). A host that cannot be reached shows "Can't reach host." (「连不上宿主。」), followed by the reason.


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

- Every bro in the group gets a turn. The bro named by a leading `@name` or `＠name` (full-width) speaks first, and that task is returned in `Said.task`. The rest go one at a time in join order and may stay silent (no message, no card, no unread). If no one is named, `Said.task` is `null` and every member still gets a turn.
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
cargo test -p robroz-host -p robroz-contract -p robroz-policy -p robroz-card -p robroz-runner -p robroz-tokens
```

The `client` job runs:

```sh
cargo build -p robroz-client
cargo test -p robroz-client -p robroz-ui -p robroz-tokens
```

`cargo test -p robroz-client` starts a real host router inside the process, so you do not need a separate host. If you only changed the host, `cargo test -p robroz-host` is enough.

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

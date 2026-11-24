# Configuration

robroz is configured with environment variables only; there is no config file. Model and web search are the exceptions: they can also be set through the settings API (search can also be filled in on the client's Web search page; see [client.md](client.md#web-search-page)). See [Model](#model) and [Search](#search) below.

“Read at” in the tables is `file:function`, as on the `main` branch. An empty or whitespace-only value is treated as unset for most variables; the tables call out the exceptions.

Placeholders such as `<your-key>`, `<your-token>` and `<host-address>` in the examples are not real secrets. Keep real keys and tokens on the machine that runs the host; do not put them in the repository, in a group chat, or in a ticket.

## Host (robroz-host)

### Listen and auth

| Variable | Read at | Default | Meaning | Example |
| --- | --- | --- | --- | --- |
| `ROBROZ_ADDR` | `host/src/main.rs:main` | `127.0.0.1:8790` | HTTP listen address, `IP:port`. A non-loopback address requires a token, or the host refuses to start (exit code 2). A bad format panics at startup. | `0.0.0.0:8790` |
| `ROBROZ_TOKEN` | `host/src/auth.rs:token_from_env` | unset, no auth | API token. When set, every `/api` request must send `Authorization: Bearer <token>`, including local ones, or the host returns 401. Leading and trailing whitespace is stripped; the token must be at least 16 characters and must not contain whitespace in the middle. | `<your-token>` |
| `ROBROZ_TOKEN_FILE` | `host/src/auth.rs:token_from_env` | unset | Read the token from this file, with the same rules; a trailing newline is stripped. Cannot be set together with `ROBROZ_TOKEN`; if both are set, the host refuses to start. Prefer the file. | `/etc/robroz/token` |

The token never appears in logs or error text. A whitespace-only value counts as unset. Comparison is constant-time (a length mismatch fails immediately, `host/src/auth.rs:ApiToken::matches`).

### Storage and containers

| Variable | Read at | Default | Meaning | Example |
| --- | --- | --- | --- | --- |
| `ROBROZ_DB` | `host/src/main.rs:main` | `./robroz.db` | Path to the SQLite file; created if missing. A relative path is resolved from the host's working directory at startup. | `/var/lib/robroz/robroz.db` |
| `ROBROZ_VOLUMES` | `host/src/main.rs:main` | `./data/groups` | Root of the group folders. Each group is `{root}/{groupId}`, mounted in the container as `/workspace`. Result files live at `{root}/{groupId}/tasks/{taskId}/result.txt`. | `/var/lib/robroz/groups` |
| `ROBROZ_CONTROL` | `host/src/main.rs:main` | `./data/control` | Root of the task control folders. Each task is `{root}/{taskId}`, with `prompt.txt`, the model reply `reply.txt`, and the takeover marker `takeover`, mounted read-only in the container as `/control`. Must sit outside `ROBROZ_VOLUMES` (neither path may contain the other), or the host refuses to start. | `/var/lib/robroz/control` |
| `ROBROZ_IMAGE` | `host/src/main.rs:main` | `robroz-runner:local` | Runner image used for tasks. Build it first with `docker build -f runner/Dockerfile -t <tag> .`. | `robroz-runner:local` |
| `ROBROZ_CONTAINER_USER` | `host/src/sandbox.rs:DockerSandbox::new` | the host process's `uid:gid` (on non-unix platforms `--user` is not passed, and the image default `65534:65534` is used) | The container's `--user`. An empty value is treated as unset. | `1000:1000` |
| `ROBROZ_GET_LIMIT` | `host/src/lib.rs:get_limit_from_env` | `8` | Outbound GET requests allowed per task (including `web_search` and `web_fetch`). This is the number allowed: using all 8 is fine; the 9th ends the task as `done` with `failure` 「出网超额：已用 9 次，上限 8」, and no delivery is sent. A value that is not a non-negative integer silently becomes 8, with no error. | `20` |
| `ROBROZ_STEP_TIMEOUT_SECS` | `host/src/watchdog.rs:WatchdogConfig::from_env` | `600` | A step (the model phase, one Router step, or the container run) that runs this many seconds is cancelled: the container is stopped and the task ends as `done` + 「没写出结果」 (Finished with no result), with no requeue (`host/src/timeouts.rs`). Time `blocked` on a card does not count. At half this value the Watchdog writes the No reply from the model or tools for N min (「模型和工具还没回，已等 N 分钟」) hint, so the card shows it before the cancel. A value that is not a positive integer keeps the default. | `300` |
| `ROBROZ_STALL_SECS` | `host/src/watchdog.rs:WatchdogConfig::from_env` | `900` | If the container is running but `progress` has not changed for this many seconds, the Watchdog logs and writes No new progress for N min (「N 分钟没有新进度」). Hint only; the state does not change. A value that is not a positive integer keeps the default. | `1800` |
| `ROBROZ_APPROVAL_TTL_SECS` | `host/src/review/mod.rs:approval_ttl` | `86400` (24 hours) | Seconds after an approval card is issued before it expires: it is recorded as `denied` with `reason` 「审批已过期」 (Approval expired), and a task it was gating ends as `done` + `failure` 「审批已过期」. Applies to task approval cards, MCP tool approval cards, bro-issued approval cards, and Allow-rule confirm cards. Memory and skill confirm cards, and a takeover, do not expire. A value that is not a positive integer keeps the default. | `3600` |

### Model

| Variable | Read at | Default | Meaning | Example |
| --- | --- | --- | --- | --- |
| `ROBROZ_MODEL_KEY` | `host/src/model_settings.rs:effective_config` | unset, optional | Model key. The key must not contain whitespace. Optional: a local service such as Ollama can omit it; with no key, the request has no `Authorization` header. | `<your-key>` |
| `ROBROZ_MODEL_KEY_FILE` | `host/src/model_settings.rs:effective_config` | unset | Read the model key from this file; leading and trailing whitespace is stripped. An empty file refuses to start. Cannot be set together with `ROBROZ_MODEL_KEY`. Prefer the file. | `/etc/robroz/model-key` |
| `ROBROZ_MODEL_BASE_URL` | `host/src/model_settings.rs:effective_config` | none | Address of an OpenAI-compatible service; the host requests `{url}/chat/completions`. Must start with `http://` or `https://`; a trailing `/` is stripped. An incomplete environment configuration does not block startup; missing fields can be filled in through the settings API. | `https://api.openai.com/v1` |
| `ROBROZ_MODEL` | `host/src/model_settings.rs:effective_config` | none | Model name. The value `echo` (case-insensitive) is a **test switch**; see below. | `gpt-4o-mini` |

Only OpenAI-compatible `/chat/completions` is supported. The host does not keep a provider list; the preset URLs come from the client's dropdown.

**Priority: the settings API wins over environment variables, field by field.** The host looks at the URL, the model name and the key separately: a field filled in on the settings API (stored in the SQLite `settings` table) is used; an empty field falls back to the matching environment variable. A model is configured when both the URL and the model name are set; the key is optional. Changing settings does not require a host restart; later tasks use the new values immediately.

- The `GET` / `PUT /api/settings/model` body includes an optional `source` field when a model is in effect: `"settings"` if any of the three fields was filled in on the settings API, `"env"` if all three come from the environment. The field is omitted when nothing is configured. So `source:"settings"` can still be a mix, for example a model name from the settings API and a key from the environment.
- On `PUT`, `null` for a field clears that settings-API value and falls back to the environment; omitting the field leaves it unchanged. `baseUrl` must start with `http://` or `https://`; a trailing `/` is stripped.
- The API returns only `hasKey` and `keyLast4`; it never echoes the key.
- `POST /api/settings/model/test` sends one short request, times out after 10 seconds, and returns `{ok, error?}`. When the provider returns 401 or 403, `error` is the contract constant `MODEL_TEST_BAD_KEY` (「密钥不对或没填」, the client shows key is wrong or missing).

When no model is configured (the URL and model name are not both set, and the test switch is off), the task delivers 「还没设置模型」 (the client shows No model set up yet), and the delivery card has the optional field `needsSetup:"model"`. The prompt is no longer written through as the result.

`ROBROZ_MODEL=echo` is a test switch; do not set it in a real deployment. When it is on, and no complete usable model is configured (settings API first), the bro still writes the prompt through as the result; a key or URL is not required. CI and `scripts/e2e.sh` turn it on explicitly. When the settings API or the environment already has a complete model, that model wins and the switch has no effect. Setting the model name to `echo` on the settings API alone, with no URL, does not turn echo on.

If both key variables are set, the key file is empty, or the key contains whitespace, the host refuses to start. The error names the variable, not the value. A partial environment configuration still starts; the rest can be filled in through the settings API.

### Search

| Variable | Read at | Default | Meaning | Example |
| --- | --- | --- | --- | --- |
| `ROBROZ_SEARCH_PROVIDER` | `host/src/search.rs:effective_config` | unset, web search is not configured | `brave` or `searxng`, case-insensitive. Any other value counts as unset. | `brave` |
| `ROBROZ_BRAVE_KEY` | `host/src/search.rs:effective_config` | unset | Brave Search subscription key, sent as the `X-Subscription-Token` header. If the provider is `brave` and there is no key, search returns 「还没设置联网搜索」 (web search isn't set up yet). If the provider is `searxng`, the request does not send a key, but if this variable is set the settings API still reports `hasKey: true`. | `<your-key>` |
| `ROBROZ_BRAVE_URL` | `host/src/search.rs:effective_config` | `https://api.search.brave.com/res/v1/web/search` | Brave API URL, used only when the provider is `brave`. Usually left alone; set it to point at a proxy or a test stub. The format is not checked at startup. | `http://127.0.0.1:9000/search` |
| `ROBROZ_SEARXNG_URL` | `host/src/search.rs:effective_config` | none | Address of a SearXNG instance. If the provider is `searxng` and there is no URL, web search is not configured. The instance must enable `json` in `search.formats` in `settings.yml`. A URL from the environment is not format-checked at startup; a URL filled in through the settings API must start with `http://` or `https://`. | `http://127.0.0.1:8888` |

**Priority: the settings API wins over environment variables, field by field.** The host looks at the provider, the URL and the key separately: a field filled in on the settings API (stored in the SQLite `settings` table) is used; an empty field falls back to the matching environment variable. Changing settings does not require a host restart.

- The `GET` / `PUT /api/settings/search` body includes an optional `source` field when search is in effect: `"settings"` if any of the three fields was filled in on the settings API, `"env"` if all three come from the environment. The field is omitted when nothing is in effect. So `source:"settings"` can still be a mix, for example a provider from the settings API and a key from the environment.
- On `PUT`, `null` for a field clears that settings-API value and falls back to the environment; omitting the field leaves it unchanged.
- The API returns only `hasKey` and `keyLast4`; it never echoes the key.

How to use the settings API, with curl examples, is in [search-and-mcp.md, How to configure search](search-and-mcp.md#怎么填搜索服务).

## Runner (inside the container)

| Variable | Read at | Default | Meaning | Example |
| --- | --- | --- | --- | --- |
| `ROBROZ_TASK_ID` | passed with `-e` by `host/src/sandbox.rs:run_argv`; read by `runner/src/main.rs:main` | none; the runner panics if unset | The current task's id. This is the container's only environment variable, set by the host; you do not set it yourself. | `ROBROZ_TASK_ID=<taskId>` |

The prompt does not go through the environment. The host writes it to `ROBROZ_CONTROL/{taskId}/prompt.txt`, and the runner only reads `/control/prompt.txt` in the container. `ROBROZ_PROMPT` appears in the code only in a test assertion, which checks that this variable is **not** passed into the container; do not set it, and setting it has no effect.

## Client (robroz-client)

| Variable | Read at | Default | Meaning | Example |
| --- | --- | --- | --- | --- |
| `ROBROZ_HOST` | `client/src/conn.rs:Conn::from_env` | `http://127.0.0.1:8790` | Host address; a trailing `/` is stripped. The client supports plain HTTP only. | `http://<host-address>:8790` |
| `ROBROZ_TOKEN` | `client/src/conn.rs:Conn::from_env` | unset | When set, every request sends `Authorization: Bearer <token>`, with leading and trailing whitespace stripped. The client does not read `ROBROZ_TOKEN_FILE`. | `<your-token>` |
| `ROBROZ_GROUPS` | `client/src/source/http.rs:HttpSource::from_env` | unset; the group list comes from `GET /api/groups` | Show only these groups; the list is no longer synced from the host. Comma-separated items, each `id` or `id=name`. | `<groupId>=website,<groupId2>` |
| `ROBROZ_USER_NAME` | `client/src/main.rs:user_name` | the computer's hostname, or `robroz` if that cannot be read | This client's name. Sent as `answeredBy` when it answers an approval or choice card; when a card comes back with the same name, the client shows You (「我」). First non-empty line only, at most 40 characters. Two devices must not share a name. | `study-computer` |
| `ROBROZ_THEME` | `client/src/main.rs:App::boot` (parsed in `crates/robroz-ui/src/theme.rs:Mode::from_env_value`) | light | `dark` (case-insensitive) starts in dark mode; any other value is light. | `dark` |
| `ROBROZ_LANG` | `client/src/prefs.rs:initial_locale` | English, or the language last picked with the language setting on the Settings page | UI language for this run: `en` or `zh` (`zh-CN`, `en_GB.UTF-8` and similar also work). Wins over the saved choice and is never written back. Unknown values are ignored. | `zh` |
| `ROBROZ_CONFIG_DIR` | `client/src/prefs.rs:config_dir` | `$XDG_CONFIG_HOME/robroz` or `~/.config/robroz` (Linux), `~/Library/Application Support/robroz` (macOS), `%APPDATA%\robroz` (Windows) | Folder for client preferences. The language setting on the Settings page saves `en` or `zh` in its `lang` file. Fake and gallery runs neither read nor write it. | `/tmp/robroz-client` |
| `ROBROZ_SCALE` | `client/src/main.rs:App::boot` | `1.0` | UI scale factor, a decimal. An unparsable value becomes `1.0`. | `1.25` |
| `ROBROZ_TOUCH` | `client/src/main.rs:App::boot` | off | Set to `1` to start with a larger touch target. If unset, the client still switches automatically after it detects touch input. | `1` |

## Development and tests only

These variables are for screenshots, UI review and e2e. You do not need them for everyday use.

| Variable | Read at | Default | Meaning | Example |
| --- | --- | --- | --- | --- |
| `ROBROZ_FAKE` | `client/src/main.rs:wants_fake` | off | Set to `1` to run the client offline on built-in fake data, the same as `--fake`. The `--gallery` flag also turns on fake mode. | `1` |
| `ROBROZ_WINDOW` | `client/src/main.rs:env_size` | `1040x680` | Initial window size, in logical pixels, `widthxheight`. A bad format keeps the default. | `390x844` |
| `ROBROZ_SHOT` | `client/src/main.rs:App::boot`, `App::update` | unset | Fake mode only: after `ROBROZ_SHOT_DELAY` milliseconds, write the window as raw RGBA to this path and exit. This is not a PNG; convert it separately. | `/tmp/shot.rgba` |
| `ROBROZ_SHOT_DELAY` | `client/src/main.rs:App::boot` | `2500` | Milliseconds to wait before the screenshot. | `4000` |
| `ROBROZ_OPEN` | `client/src/main.rs:App::update` | unset | Fake mode only: open this group on startup (the value is a group id), for a chat-page screenshot. | `<groupId>` |
| `ROBROZ_RENAME` | `client/src/main.rs:App::update` | unset | Fake mode only: when the open chat is a DM, open the rename box and fill in this value. | `New name` |
| `ROBROZ_RENAME_SAVE` | `client/src/main.rs:App::update` | off | Used with `ROBROZ_RENAME`; set to `1` to also press Save (「保存」). | `1` |
| `ROBROZ_IMAGE` (e2e) | `scripts/e2e.sh` | `robroz-runner:e2e` | Runner image tag that e2e builds and uses. | `robroz-runner:e2e` |
| `ROBROZ_MODEL=echo` | `host/src/model_settings.rs:echo_mode`, `scripts/e2e.sh` | unset | Test switch: when no complete model is configured, write the prompt through as the result. CI and e2e turn it on explicitly. Do not set it in a real deployment. See [Model](#model). | `echo` |
| `E2E_PORT` | `scripts/e2e.sh` | `18790` | Host port used in e2e. It does not have the `ROBROZ_` prefix. | `18800` |

`scripts/e2e.sh` sets `ROBROZ_ADDR`, `ROBROZ_DB`, `ROBROZ_VOLUMES`, `ROBROZ_CONTROL`, `ROBROZ_IMAGE` and `ROBROZ_MODEL=echo` itself and passes them to the host. The first four point at a temporary directory and override any values already in the environment. `ROBROZ_MODEL=echo` is a test switch; see [Model](#model) above.

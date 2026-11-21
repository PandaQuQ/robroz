# 配置参考

robroz 只用环境变量配置，没有配置文件。联网搜索例外：它也可以用设置接口填，见下面的[搜索](#搜索)。

下表的「读取位置」是 `文件:函数`，以 `main` 分支为准。值为空字符串或只有空白时，多数变量按没设处理，表里单独说明的除外。

示例里的 `<你的密钥>`、`<你的令牌>`、`<宿主机地址>` 都是占位符。真实的密钥和令牌只放在运行宿主机的那台机器上，不要写进仓库、群聊或工单。

## 宿主机（robroz-host）

### 监听和鉴权

| 变量 | 读取位置 | 默认值 | 含义 | 示例 |
| --- | --- | --- | --- | --- |
| `ROBROZ_ADDR` | `host/src/main.rs:main` | `127.0.0.1:8790` | HTTP 监听地址，`IP:端口`。不是本机回环地址时必须设令牌，否则拒绝启动（退出码 2）。格式不对时启动即 panic。 | `0.0.0.0:8790` |
| `ROBROZ_TOKEN` | `host/src/auth.rs:token_from_env` | 不设，不鉴权 | 接口令牌。设了以后所有 `/api` 请求都要带 `Authorization: Bearer <令牌>`，本机访问也要，否则 401。去掉首尾空白；至少 16 个字符；中间不能有空白。 | `<你的令牌>` |
| `ROBROZ_TOKEN_FILE` | `host/src/auth.rs:token_from_env` | 不设 | 从这个文件读令牌，规则同上，文件末尾的换行会去掉。和 `ROBROZ_TOKEN` 只能设一个，两个都设时拒绝启动。推荐用文件。 | `/etc/robroz/token` |

令牌不会出现在日志和报错里。

### 存储和容器

| 变量 | 读取位置 | 默认值 | 含义 | 示例 |
| --- | --- | --- | --- | --- |
| `ROBROZ_DB` | `host/src/main.rs:main` | `./robroz.db` | SQLite 文件路径，不存在就新建。相对路径按宿主机启动时的当前目录算。 | `/var/lib/robroz/robroz.db` |
| `ROBROZ_VOLUMES` | `host/src/main.rs:main` | `./data/groups` | 群目录根。每个群一个子目录 `{根}/{groupId}`，挂到容器的 `/workspace`。结果文件在 `{根}/{groupId}/tasks/{taskId}/result.txt`。 | `/var/lib/robroz/groups` |
| `ROBROZ_CONTROL` | `host/src/main.rs:main` | `./data/control` | 任务控制目录根。每个任务一个子目录 `{根}/{taskId}`，里面是 `prompt.txt`、模型回答 `reply.txt` 和接管标记 `takeover`，只读挂到容器的 `/control`。必须在 `ROBROZ_VOLUMES` 外面（两者互不包含），否则拒绝启动。 | `/var/lib/robroz/control` |
| `ROBROZ_IMAGE` | `host/src/main.rs:main` | `robroz-runner:local` | 跑任务用的 runner 镜像。要先用 `docker build -f runner/Dockerfile -t <标签> .` 构建。 | `robroz-runner:local` |
| `ROBROZ_CONTAINER_USER` | `host/src/sandbox.rs:DockerSandbox::new` | 宿主机进程的 `uid:gid`（非 unix 平台不传 `--user`，用镜像默认的 `65534:65534`） | 容器的 `--user`。空值按没设处理。 | `1000:1000` |
| `ROBROZ_GET_LIMIT` | `host/src/lib.rs:get_limit_from_env` | `8` | 每个任务允许的出网 GET 次数（含 `web_search`、`web_fetch`）。这是允许的次数：用满 8 次仍算正常，第 9 次时任务以 `done` 结束，带 `failure`「出网超额：已用 9 次，上限 8」，不发 delivery。不是非负整数时静默用 8，不报错。 | `20` |

### 模型

| 变量 | 读取位置 | 默认值 | 含义 | 示例 |
| --- | --- | --- | --- | --- |
| `ROBROZ_MODEL_KEY` | `crates/robroz-model/src/lib.rs:config_from_env` | 不设，不接模型 | 模型密钥。不设（`ROBROZ_MODEL_KEY_FILE` 也不设）时 bro 把 prompt 原样写成结果。密钥里不能有空白。 | `<你的密钥>` |
| `ROBROZ_MODEL_KEY_FILE` | `crates/robroz-model/src/lib.rs:config_from_env` | 不设 | 从这个文件读模型密钥，首尾空白会去掉；文件是空的就拒绝启动。和 `ROBROZ_MODEL_KEY` 只能设一个。推荐用文件。 | `/etc/robroz/model-key` |
| `ROBROZ_MODEL_BASE_URL` | `crates/robroz-model/src/lib.rs:config_from_env` | 无 | OpenAI 兼容服务的地址，宿主机请求 `{地址}/chat/completions`。设了密钥就必须设；要以 `http://` 或 `https://` 开头；末尾的 `/` 会去掉。 | `https://api.openai.com/v1` |
| `ROBROZ_MODEL` | `crates/robroz-model/src/lib.rs:config_from_env` | 无 | 模型名。设了密钥就必须设。 | `gpt-4o-mini` |

没设密钥时，另外两个变量即使设了也不生效。配置有错时宿主机拒绝启动，报错只写变量名，不写值。

### 搜索

| 变量 | 读取位置 | 默认值 | 含义 | 示例 |
| --- | --- | --- | --- | --- |
| `ROBROZ_SEARCH_PROVIDER` | `host/src/search.rs:effective_config` | 不设，未设置联网搜索 | `brave` 或 `searxng`，不分大小写。别的值等于没设置。 | `brave` |
| `ROBROZ_BRAVE_KEY` | `host/src/search.rs:effective_config` | 不设 | Brave Search 的订阅密钥，以 `X-Subscription-Token` 请求头发出。provider 是 `brave` 却没有密钥时，搜索会返回「还没设置联网搜索」。provider 是 `searxng` 时请求里不带密钥，但设了这个变量的话，设置接口仍会显示 `hasKey: true`。 | `<你的密钥>` |
| `ROBROZ_BRAVE_URL` | `host/src/search.rs:effective_config` | `https://api.search.brave.com/res/v1/web/search` | Brave 接口地址，只在 provider 是 `brave` 时用。一般不用改，接自建代理或测试桩时才设。启动时不检查格式。 | `http://127.0.0.1:9000/search` |
| `ROBROZ_SEARXNG_URL` | `host/src/search.rs:effective_config` | 无 | SearXNG 实例地址。provider 是 `searxng` 而没有地址时，等于没设置联网搜索。实例要在 `settings.yml` 的 `search.formats` 里打开 `json`。通过环境变量给的地址启动时不检查格式；通过设置接口填的地址必须以 `http://` 或 `https://` 开头。 | `http://127.0.0.1:8888` |

**优先级：设置接口优先于环境变量，逐个字段比较。** 宿主机分别看 provider、地址、密钥三项：设置接口（存在 SQLite 的 `settings` 表里）填了的项用设置接口的值，没填的项退回对应的环境变量。改设置不用重启宿主机。

- `GET` / `PUT /api/settings/search` 的返回体在已生效时带可选字段 `source`：设置接口里填过任意一项就是 `"settings"`，三项都没填、全部来自环境变量时是 `"env"`。完全没生效时没有这个字段。所以 `source:"settings"` 也可能是「provider 来自设置接口、密钥来自环境变量」的混合情况。
- `PUT` 时某个字段传 `null` 表示清掉设置接口里的那一项，之后退回环境变量；不传表示不改。
- 接口只返回 `hasKey` 和 `keyLast4`，不回显密钥。

设置接口的用法和 curl 示例见 [README 的「怎么填搜索服务」](../README.md)。

## runner（容器内）

| 变量 | 读取位置 | 默认值 | 含义 | 示例 |
| --- | --- | --- | --- | --- |
| `ROBROZ_TASK_ID` | 由 `host/src/sandbox.rs:run_argv` 以 `-e` 传入；`runner/src/main.rs:main` 读取 | 无，没设时 runner panic | 当前任务的 id。这是容器唯一的环境变量，由宿主机设置，不需要手动配。 | `ROBROZ_TASK_ID=<taskId>` |

prompt 不走环境变量。宿主机把它写进 `ROBROZ_CONTROL/{taskId}/prompt.txt`，runner 只读容器里的 `/control/prompt.txt`。`ROBROZ_PROMPT` 在代码里只出现在一条测试断言中，用来确认这个变量**没有**传给容器；不要设它，设了也没用。

## 客户端（robroz-client）

| 变量 | 读取位置 | 默认值 | 含义 | 示例 |
| --- | --- | --- | --- | --- |
| `ROBROZ_HOST` | `client/src/conn.rs:Conn::from_env` | `http://127.0.0.1:8790` | 宿主机地址，末尾的 `/` 会去掉。客户端只支持明文 HTTP。 | `http://<宿主机地址>:8790` |
| `ROBROZ_TOKEN` | `client/src/conn.rs:Conn::from_env` | 不设 | 设了就在每个请求上带 `Authorization: Bearer <令牌>`，去掉首尾空白。客户端不读 `ROBROZ_TOKEN_FILE`。 | `<你的令牌>` |
| `ROBROZ_GROUPS` | `client/src/source/http.rs:HttpSource::from_env` | 不设，群列表来自 `GET /api/groups` | 只显示这几个群，不再从宿主机同步群列表。逗号分隔，每项是 `id` 或 `id=名字`。 | `<groupId>=官网,<groupId2>` |
| `ROBROZ_USER_NAME` | `client/src/main.rs:user_name` | 电脑的主机名，拿不到时是 `robroz` | 这台客户端的名字。答审批卡、选项卡时作为 `answeredBy` 发出，卡片回来时和它一样就显示「我」。只取第一行非空内容，最多 40 个字。两台设备不要用同一个名字。 | `书房电脑` |
| `ROBROZ_THEME` | `client/src/main.rs:App::boot`（解析在 `crates/robroz-ui/src/theme.rs:Mode::from_env_value`） | 浅色 | `dark`（不分大小写）启动时用深色，其他值都是浅色。 | `dark` |
| `ROBROZ_SCALE` | `client/src/main.rs:App::boot` | `1.0` | 界面缩放倍数，小数。解析不了时用 `1.0`。 | `1.25` |
| `ROBROZ_TOUCH` | `client/src/main.rs:App::boot` | 关 | 设为 `1` 时一开始就按触屏放大点击区域。不设也会在检测到触摸输入后自动切换。 | `1` |

## 开发和测试专用

这些变量给截图、界面验收和 e2e 用，日常使用不需要设。

| 变量 | 读取位置 | 默认值 | 含义 | 示例 |
| --- | --- | --- | --- | --- |
| `ROBROZ_FAKE` | `client/src/main.rs:wants_fake` | 关 | 设为 `1` 时客户端不连宿主机，用内置的假数据。等同于 `--fake` 参数。`--gallery` 参数也会打开假数据模式。 | `1` |
| `ROBROZ_WINDOW` | `client/src/main.rs:env_size` | `1040x680` | 初始窗口大小，逻辑像素，格式 `宽x高`。格式不对时用默认值。 | `390x844` |
| `ROBROZ_SHOT` | `client/src/main.rs:App::boot`、`App::update` | 不设 | 只在假数据模式下生效：等 `ROBROZ_SHOT_DELAY` 毫秒后把窗口截成原始 RGBA 写到这个路径，然后退出。不是 PNG，要另行转换。 | `/tmp/shot.rgba` |
| `ROBROZ_SHOT_DELAY` | `client/src/main.rs:App::boot` | `2500` | 截图前等待的毫秒数。 | `4000` |
| `ROBROZ_OPEN` | `client/src/main.rs:App::update` | 不设 | 只在假数据模式下生效：启动后直接打开这个群（值是群 id），用来截聊天页。 | `<groupId>` |
| `ROBROZ_RENAME` | `client/src/main.rs:App::update` | 不设 | 只在假数据模式下生效：选中的是私聊时，打开改名框并填入这个值。 | `新名字` |
| `ROBROZ_RENAME_SAVE` | `client/src/main.rs:App::update` | 关 | 和 `ROBROZ_RENAME` 一起用，设为 `1` 时顺便按下「保存」。 | `1` |
| `ROBROZ_IMAGE`（e2e） | `scripts/e2e.sh` | `robroz-runner:e2e` | e2e 构建和使用的 runner 镜像标签。 | `robroz-runner:e2e` |
| `E2E_PORT` | `scripts/e2e.sh` | `18790` | e2e 里宿主机的端口。不带 `ROBROZ_` 前缀。 | `18800` |

`scripts/e2e.sh` 会自己设 `ROBROZ_ADDR`、`ROBROZ_DB`、`ROBROZ_VOLUMES`、`ROBROZ_CONTROL`、`ROBROZ_IMAGE` 并传给宿主机，前四个指向临时目录，覆盖你环境里已有的值。

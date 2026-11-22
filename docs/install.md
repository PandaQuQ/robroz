# 安装和上手

这篇讲怎么从源码把 robroz 跑起来：装工具、构建宿主机和 runner 镜像、选模型、跑客户端，再用 curl 走一遍第一个任务。所有环境变量的完整说明见 [configuration.md](configuration.md)。

## 准备

| 需要 | 用途 | 说明 |
| --- | --- | --- |
| Rust stable 工具链（`rustup`、`cargo`） | 构建宿主机、客户端、runner | 仓库没有 `rust-toolchain` 文件，`Cargo.toml` 也没写 `rust-version`。CI 用 `dtolnay/rust-toolchain@stable`，跟着最新 stable 走。所有 crate 是 edition 2021。 |
| Docker | 宿主机用它跑 runner 容器 | 宿主机直接调 `docker` 命令行（`docker run` / `inspect` / `rm`），所以跑宿主机的用户要能执行 `docker`。只跑单测不需要 Docker。 |
| `curl`、`jq` | 下面的示例和 `scripts/e2e.sh` | e2e 脚本必须有这两个。 |

客户端在 Windows、Linux、macOS 上都能构建。CI 在 PR 上只构建 Linux，合到 `main` 后和每天定时会再跑 Windows 和 macOS。

## 构建

在仓库根目录：

```sh
cargo build -p robroz-host      # 宿主机，产物 target/debug/robroz-host
cargo build -p robroz-client    # 桌面客户端，产物 target/debug/robroz-client
```

要发布版就加 `--release`，产物在 `target/release/`。

### runner 镜像

bro 的每个任务在一个 Docker 容器里跑，容器里是 `robroz-runner`。宿主机默认用的镜像名是 `robroz-runner:local`（`ROBROZ_IMAGE` 可以改）。必须在仓库根目录构建，因为 Dockerfile 要拷整个工作区：

```sh
docker build -f runner/Dockerfile -t robroz-runner:local .
```

镜像分两段：先在 `rust:1-bookworm` 里 `cargo build --release -p robroz-runner`，再把二进制放进 `debian:bookworm-slim`。镜像默认用户是 `65534:65534`（nobody），不是 root；宿主机启动容器时会传 `--user`，见下文。

容器的限制由宿主机在 `docker run` 时加上：

- `--network none`，容器里没有网络；
- 只有两个挂载：群目录挂到 `/workspace`（可写），这个任务自己的控制目录挂到 `/control`（只读，里面有 `prompt.txt`、模型回答 `reply.txt` 和接管标记 `takeover`）；
- 只传一个环境变量 `ROBROZ_TASK_ID`，prompt 不走环境变量；
- `--user` 默认是宿主机进程的 `uid:gid`（非 unix 平台不加），群目录里写出的文件归宿主机用户。要换就设 `ROBROZ_CONTAINER_USER`。

## 跑宿主机

```sh
./target/debug/robroz-host
```

不设任何变量时：

| 项 | 默认值 |
| --- | --- |
| 监听地址 `ROBROZ_ADDR` | `127.0.0.1:8790` |
| 数据库 `ROBROZ_DB` | `./robroz.db`（SQLite，不存在就新建） |
| 群目录根 `ROBROZ_VOLUMES` | `./data/groups` |
| 控制目录根 `ROBROZ_CONTROL` | `./data/control` |
| runner 镜像 `ROBROZ_IMAGE` | `robroz-runner:local` |

相对路径按启动时的当前目录算。`ROBROZ_CONTROL` 必须在 `ROBROZ_VOLUMES` 外面（两者都不能包含对方），否则宿主机拒绝启动。

启动成功时 stderr 会打两行：

```text
robroz-host: 还没设置模型，任务会提示去设置
robroz-host listening on 127.0.0.1:8790（仅本机，无令牌）
```

第一行说的是模型，有三种（`host/src/main.rs:main`）：

| 日志 | 什么时候 |
| --- | --- |
| `robroz-host: 环境变量里有模型 <模型名> @ <地址>，设置页保存的值优先` | 环境变量里地址、模型名、密钥三项都齐了 |
| `robroz-host: 测试回显（ROBROZ_MODEL=echo）` | 环境变量没配齐，并且 `ROBROZ_MODEL=echo`（测试开关，见下文） |
| `robroz-host: 还没设置模型，任务会提示去设置` | 其他情况 |

这一行只看环境变量，不看设置接口里存的值。用设置接口选过模型的话，重启后这里仍可能显示「还没设置模型」，以 `GET /api/settings/model` 为准。

### 令牌

- 监听本机回环地址时可以不设令牌。
- 监听别的地址（比如 `0.0.0.0:8790`）而没设令牌，宿主机直接退出（退出码 2），提示：`拒绝启动：0.0.0.0:8790 不是本机回环地址，必须先设置 ROBROZ_TOKEN 或 ROBROZ_TOKEN_FILE`。
- 令牌从 `ROBROZ_TOKEN` 或 `ROBROZ_TOKEN_FILE`（文件路径）读，两个只能设一个。去掉首尾空白，至少 16 个字符，中间不能有空白。推荐用文件，令牌不会留在 shell 历史和进程环境里。
- 设了令牌后，所有 `/api` 请求都要带 `Authorization: Bearer <令牌>`，本机访问也要。缺了或错了返回 401。

```sh
# 生成一个令牌文件，只给自己读
mkdir -p "$HOME/.config/robroz"
(umask 077 && openssl rand -hex 24 > "$HOME/.config/robroz/token")
ROBROZ_ADDR=0.0.0.0:8790 ROBROZ_TOKEN_FILE="$HOME/.config/robroz/token" ./target/debug/robroz-host
```

令牌文件和下面的密钥文件都不要放进仓库目录，也不要提交。

## 选模型

robroz 没有默认模型，用哪个服务、哪个模型由部署的人在部署时自己选。只支持 OpenAI 兼容的 `/chat/completions`，后端不维护服务商列表。要准备三样：服务地址、模型名、密钥。

还没选模型时宿主机照常启动，任务也会做完（`done`），但结果文件里是「还没设置模型」，不是模型的回答；交付卡带 `needsSetup:"model"`。不会再把 prompt 原样写成结果。

### 部署时用环境变量（推荐）

把密钥写进仓库外的文件，用 `ROBROZ_MODEL_KEY_FILE` 指过去，密钥就不会留在 shell 历史和进程环境里：

```sh
mkdir -p "$HOME/.config/robroz"
(umask 077 && printf '%s' '<你的密钥>' > "$HOME/.config/robroz/model-key")
ROBROZ_MODEL_BASE_URL='<服务地址>' \
ROBROZ_MODEL='<模型名>' \
ROBROZ_MODEL_KEY_FILE="$HOME/.config/robroz/model-key" \
./target/debug/robroz-host
```

- `<服务地址>` 是 OpenAI 兼容接口的根地址，例如 `https://api.openai.com/v1`，宿主机请求的是 `<服务地址>/chat/completions`。要以 `http://` 或 `https://` 开头，末尾的 `/` 会去掉。`<模型名>` 是这个服务认的模型名。
- 密钥用 `ROBROZ_MODEL_KEY_FILE` 或 `ROBROZ_MODEL_KEY`，只能设一个，推荐文件。
- 两个密钥变量都设、密钥文件读不出来或是空的、密钥里有空白时，宿主机拒绝启动。报错里只有变量名，不会出现密钥。
- 只设了一部分（比如只有密钥）不会拒绝启动，缺的项可以之后用设置接口补。地址格式不对也不会拒绝启动：这时等于没选模型，试连接口会说明原因，见下文。
- 三项都齐了时，启动日志是 `robroz-host: 环境变量里有模型 <模型名> @ <地址>，设置页保存的值优先`。
- 同时要设令牌的话，把 `ROBROZ_TOKEN_FILE` 也写在这条命令里。

### 启动后用设置接口

也可以先把宿主机跑起来，再用设置接口选模型。存进 SQLite，改完下一个任务就用上，不用重启：

```sh
B=http://127.0.0.1:8790
curl -s -X PUT $B/api/settings/model -H 'content-type: application/json' \
  -d '{"baseUrl":"<服务地址>","model":"<模型名>","key":"<你的密钥>"}'
curl -s -X POST $B/api/settings/model/test     # {"ok":true} 就是连上了
```

- 设置接口里填了的项优先于环境变量，逐项比较，没填的项退回环境变量。
- 接口不回显密钥，只返回 `hasKey` 和后 4 位 `keyLast4`。
- 设了令牌的话，每条 curl 都加 `-H "Authorization: Bearer <你的令牌>"`。
- 上面的 curl 会把密钥留在 shell 历史里，用完清掉，或者改用环境变量的方式。

字段、错误文字和优先级的细节见 [configuration.md 的「模型」](configuration.md#模型) 和 [api.md 的「模型设置」](api.md#模型设置)。

### 测试开关 `ROBROZ_MODEL=echo`

CI 和 `scripts/e2e.sh` 会显式设 `ROBROZ_MODEL=echo`：没有配齐模型时，bro 把 prompt 原样写成结果，这样测试不需要真模型。这只是测试开关，不是默认值，正式部署不要设。

联网搜索（Brave 或 SearXNG）可以用环境变量，也可以在宿主机跑起来后用设置接口填，见 [configuration.md 的「搜索」一节](configuration.md#搜索)。

## 跑客户端

客户端是 iced 写的桌面程序，三个平台命令相同，只是设环境变量的写法不同。默认连 `http://127.0.0.1:8790`。

Linux / macOS：

```sh
cargo run -p robroz-client
# 宿主机在别处、或者设了令牌
ROBROZ_HOST=http://<宿主机地址>:8790 ROBROZ_TOKEN='<你的令牌>' cargo run -p robroz-client
```

Windows（PowerShell）：

```powershell
$env:ROBROZ_HOST = "http://<宿主机地址>:8790"
$env:ROBROZ_TOKEN = "<你的令牌>"
cargo run -p robroz-client
```

- 客户端只走明文 HTTP，没有 TLS。跨机器用时请走内网或自己加隧道。
- 客户端只认 `ROBROZ_TOKEN`，不读 `ROBROZ_TOKEN_FILE`。
- 群列表来自 `GET /api/groups`。设了 `ROBROZ_GROUPS` 就只显示这几个群，不再跟宿主机同步。
- 不连宿主机、只看界面：`cargo run -p robroz-client -- --fake`（或 `ROBROZ_FAKE=1`）。
- 连不上宿主机时保留已经显示的内容，过一会再试。

Linux 上需要图形桌面（X11 或 Wayland）。CI 的 `ubuntu-latest` 不装额外的系统包就能构建。

## 第一个任务

下面用 curl 走一遍：建群、拉 bro 进群、发一句话、看卡片。先按上文构建 runner 镜像，再起宿主机（这里用默认地址、不设令牌）。设了令牌的话，每条 curl 都加 `-H "Authorization: Bearer <你的令牌>"`。

```sh
B=http://127.0.0.1:8790
J='content-type: application/json'

# 1. 建群，返回 201 和 {"id","name","tenantId"}
GID=$(curl -s -X POST $B/api/groups -H "$J" -d '{"name":"官网"}' | jq -r .id)

# 2. 把 bro 拉进群：botId 是 bro 的 id，name 是在群里 @ 用的名字
curl -s -X POST $B/api/groups/$GID/members -H "$J" -d '{"botId":"bot-ada","name":"ada"}'

# 3. 在群里说话：以 @名字 开头才会派任务
curl -s -X POST $B/api/groups/$GID/say -H "$J" -d '{"body":"@ada 写一句欢迎语"}'
```

第 3 步返回 201，`task` 是新排队的任务：

```json
{"message":{"id":"…","kind":"text","body":"@ada 写一句欢迎语","createdAt":"…"},
 "task":{"id":"…","assigneeBotId":"bot-ada","prompt":"写一句欢迎语","state":"queued"}}
```

`@` 规则：消息要以 `@名字`（全角 `＠` 也行）开头，名字后面是空格或结尾；多个成员名字都能匹配时取最长的；`@名字` 后面没内容就不派任务，`task` 是 `null`。私聊里不需要 `@`，直接派给那个 bro。

宿主机每秒扫一次队列，起容器跑任务。几秒后看卡片：

```sh
curl -s $B/api/groups/$GID/events | jq -c '.events[] | {type, state, label, path}'
```

```text
{"type":"message","state":null,"label":null,"path":null}
{"type":"status","state":"done","label":"完成","path":null}
{"type":"delivery","state":null,"label":null,"path":"tasks/<taskId>/result.txt"}
```

- `status` 卡是任务状态，带 `botId`、`botName`、`state`、`label`、`containerUp`。
- `delivery` 卡说明结果写好了，文件在群目录里：`./data/groups/<groupId>/tasks/<taskId>/result.txt`。还没选模型时内容是「还没设置模型」，交付卡带 `needsSetup:"model"`；选了模型时是模型的回答。
- 要实时收卡片，用 `GET /api/groups/{id}/stream`；只要变化的部分，`GET …/events?since=<RFC3339 时间>`。

另一种开局方式是先建 bro，宿主机会顺带给它开一个私聊：

```sh
curl -s -X POST $B/api/bros -H "$J" -d '{"name":"bob"}'
# {"id":"…","name":"bob","dmGroupId":"…"}
curl -s -X POST $B/api/groups/<dmGroupId>/say -H "$J" -d '{"body":"写一句欢迎语"}'
```

其他接口（审批、选项卡、例行任务、记忆、技能、搜索设置、MCP 服务器）见 [api.md](api.md)。

## 跑测试

单测不需要 Docker。CI 的 `host` 任务跑的是：

```sh
cargo test -p robroz-mcp --features fixture
cargo test -p robroz-host -p robroz-contract -p robroz-policy -p robroz-card -p robroz-runner -p robroz-tokens
```

`client` 任务跑的是：

```sh
cargo build -p robroz-client
cargo test -p robroz-client -p robroz-ui -p robroz-tokens
```

`cargo test -p robroz-client` 会在进程里起一份真实的宿主机路由，不需要单独起宿主机。只改了宿主机时，`cargo test -p robroz-host` 就够了。

CI 还有一个 `cargo-deny` 任务，检查依赖许可证是否与 AGPL-3.0 兼容，并挡住未知来源。仓库根目录跑：

```sh
cargo deny check licenses bans sources
```

需要先安装 [cargo-deny](https://github.com/EmbarkStudios/cargo-deny)。新依赖必须使用与 AGPL-3.0 兼容的许可证，允许的许可证写在根目录的 `deny.toml`。不要把不兼容的许可证加进白名单。CI 也会跑 `cargo deny check advisories`，但这一项目前不挡住合并。


### 端到端：`scripts/e2e.sh`

用真实的宿主机二进制和真实的 Docker 跑一遍完整流程，CI 的 `e2e` 任务就是它。需要 `docker`、`curl`、`jq`，在仓库根目录跑：

```sh
scripts/e2e.sh
```

它做的事：

1. 构建 runner 镜像（默认标签 `robroz-runner:e2e`，用 `ROBROZ_IMAGE` 改）和 `cargo build -p robroz-host`。
2. 在临时目录里放数据库、群目录和控制目录，宿主机监听 `127.0.0.1:18790`（用 `E2E_PORT` 改端口）。脚本自己设 `ROBROZ_ADDR`、`ROBROZ_DB`、`ROBROZ_VOLUMES`、`ROBROZ_CONTROL`、`ROBROZ_IMAGE` 和测试开关 `ROBROZ_MODEL=echo`，不读你环境里的这几个值（`ROBROZ_IMAGE` 除外）。
3. 建群、加成员 `bot-e2e`，派一个带两行阻塞指令（`robroz-action:external` 和 `robroz-action:captcha`）的任务。不接真模型（`ROBROZ_MODEL=echo`），runner 按 prompt 确定地执行。
4. 检查：任务停在 `blocked` 且没有 delivery；重启宿主机后仍是 `blocked`；第一次 `resume` 后停在第二行而不是重复第一行；第二次 `resume` 后 `done`，`result.txt` 只有三行正文，delivery 恰好一条。
5. 再重启一次，检查已完成的任务不重放：状态、delivery 数量、`result.txt` 的修改时间都不变，对已完成任务 `resume` 返回 409，没有留下容器。

最后输出 `[e2e] PASS`。失败时打印原因和宿主机日志的最后 40 行，退出时清掉临时目录。

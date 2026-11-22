# 安装和上手

这篇讲怎么从源码把 robroz 跑起来：装工具、构建宿主机和 runner 镜像、接模型、跑客户端，再用 curl 走一遍第一个任务。所有环境变量的完整说明见 [configuration.md](configuration.md)。

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
robroz-host: 没配模型密钥，bro 原样回显
robroz-host listening on 127.0.0.1:8790（仅本机，无令牌）
```

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

## 接模型

不接模型时 bro 把 prompt 原样写成结果，CI 和 e2e 就是这样跑的。接任意 OpenAI 兼容的 `/chat/completions` 服务要设三个变量：

```sh
mkdir -p "$HOME/.config/robroz"
(umask 077 && printf '%s' '<你的密钥>' > "$HOME/.config/robroz/model-key")
ROBROZ_MODEL_BASE_URL=https://api.openai.com/v1 \
ROBROZ_MODEL=gpt-4o-mini \
ROBROZ_MODEL_KEY_FILE="$HOME/.config/robroz/model-key" \
./target/debug/robroz-host
```

- 密钥用 `ROBROZ_MODEL_KEY_FILE` 或 `ROBROZ_MODEL_KEY`，只能设一个，推荐文件。
- 设了密钥就必须同时设 `ROBROZ_MODEL_BASE_URL` 和 `ROBROZ_MODEL`。地址要以 `http://` 或 `https://` 开头，末尾的 `/` 会去掉。
- 配错时宿主机拒绝启动，报错里只有变量名，不会出现密钥。
- 配好后启动日志是 `robroz-host: 模型 <模型名> @ <地址>`。

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
- `delivery` 卡说明结果写好了，文件在群目录里：`./data/groups/<groupId>/tasks/<taskId>/result.txt`。没接模型时内容就是 prompt「写一句欢迎语」。
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
2. 在临时目录里放数据库、群目录和控制目录，宿主机监听 `127.0.0.1:18790`（用 `E2E_PORT` 改端口）。脚本自己设 `ROBROZ_ADDR`、`ROBROZ_DB`、`ROBROZ_VOLUMES`、`ROBROZ_CONTROL`、`ROBROZ_IMAGE`，不读你环境里的这几个值（`ROBROZ_IMAGE` 除外）。
3. 建群、加成员 `bot-e2e`，派一个带两行阻塞指令（`robroz-action:external` 和 `robroz-action:captcha`）的任务。不接模型，runner 按 prompt 确定地执行。
4. 检查：任务停在 `blocked` 且没有 delivery；重启宿主机后仍是 `blocked`；第一次 `resume` 后停在第二行而不是重复第一行；第二次 `resume` 后 `done`，`result.txt` 只有三行正文，delivery 恰好一条。
5. 再重启一次，检查已完成的任务不重放：状态、delivery 数量、`result.txt` 的修改时间都不变，对已完成任务 `resume` 返回 409，没有留下容器。

最后输出 `[e2e] PASS`。失败时打印原因和宿主机日志的最后 40 行，退出时清掉临时目录。

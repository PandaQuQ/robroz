# 配置

host 主要靠环境变量配置，没有单独的配置文件。模型和联网搜索是例外：host 跑起来之后，可以在客户端的设置页里填，改完不用重启。

下面只写日常会碰到的几项。全部变量、默认值和更细的规则，见英文的 [configuration.md](../configuration.md)；客户端的变量见 robroz-client 的 [configuration.md](https://github.com/PandaQuQ/robroz-client/blob/main/docs/configuration.md)。

示例里的 `<你的令牌>`、`<你的密钥>`、`<宿主机地址>` 都是占位符。真实的密钥和令牌只放在运行 host 的那台机器上，不要写进仓库，也不要发到群里。

## 用安装脚本装的 host

[安装上手](install.md) 里的一条命令会在安装目录（默认 `~/robroz`）下写好这几样：

| 文件 | 内容 |
| --- | --- |
| `compose.yml` | 由脚本生成，每次重跑都会替换（旧的留成 `compose.yml.bak`），不要手改 |
| `.env` | 本机设置，只在第一次写，以后重跑不会动 |
| `secrets/robroz_token` | 随机生成的 API 令牌，只在第一次写 |
| `data/` | 数据库、群目录和任务控制文件 |

`.env` 里能改的：

- `ROBROZ_DATA`：数据目录。容器里外要用同一个绝对路径，只能连同里面的数据一起挪。
- `ROBROZ_BIND`、`ROBROZ_PORT`：host 在这台机器上监听的地址和端口，默认 `127.0.0.1` 和 `8790`。
- `DOCKER_GID`：本机 docker 组的 id，由脚本填好。

改完 `.env` 后重跑一次安装脚本（或在安装目录里 `docker compose up -d`）生效。

要给 host 加别的环境变量，比如下面提到的 `ROBROZ_SINGLE_RESPONDER`、`ROBROZ_GET_LIMIT`，写在安装目录下的 `compose.override.yml` 里：

```yaml
services:
  host:
    environment:
      ROBROZ_GET_LIMIT: "12"
```

`docker compose` 会自动合并这个文件，重跑安装脚本也不会覆盖它。

## 令牌

v0.2.0 的 host 只有一个令牌。带着这个令牌连上来的，就是 host 的管理员 `local-admin`，也就是你自己；你引入的 bro 都归你。用 `GET /api/me` 可以看到当前身份和角色。

- 用安装脚本装的 host 一定有令牌，在 `secrets/robroz_token`。
- 设了令牌以后，所有 `/api` 请求都要带 `Authorization: Bearer <你的令牌>`，本机也不例外。没带或带错返回 401。
- 自己构建运行的 host，令牌来自 `ROBROZ_TOKEN` 或 `ROBROZ_TOKEN_FILE`（文件路径），两者只能设一个，推荐用文件。令牌至少 16 个字符，中间不能有空白。
- 不设令牌时，host 只肯在 `127.0.0.1` 这类本机回环地址上启动；监听别的地址又没有令牌，会直接拒绝启动。
- 令牌不会出现在日志或报错里。

```sh
mkdir -p "$HOME/.config/robroz"
(umask 077 && openssl rand -hex 24 > "$HOME/.config/robroz/token")
ROBROZ_TOKEN_FILE="$HOME/.config/robroz/token" ./target/debug/robroz-host
```

客户端在连接表单里填同一个令牌，见 [安装上手](install.md#3-连上-host)。

## 从别的机器连上来

host 自己不做 TLS。要从别的机器访问，推荐让 host 仍然只监听 `127.0.0.1:8790`，在前面放一层 HTTPS 反向代理或隧道（例如 Caddy、Cloudflare Tunnel），客户端连代理的 HTTPS 地址，并带上同一个令牌。不要把带令牌的明文 HTTP 暴露到公网。配置示例见英文 [install.md](../install.md#connecting-to-a-remote-host)。

只在自己的局域网或 Tailscale 里用手机连的话，可以把 `.env` 里的 `ROBROZ_BIND` 改成这台机器的局域网或 Tailscale 地址，再重跑安装脚本。客户端连明文 `http://` 的远程地址时会提示「未加密，远程请用 HTTPS。」。

## 数据放在哪里

用安装脚本装的，数据都在安装目录的 `data/` 下，重跑脚本不会动。

自己构建运行的 host，三样东西可以分别指定：

- `ROBROZ_DB`：SQLite 数据库文件，默认 `./robroz.db`，没有就新建。
- `ROBROZ_VOLUMES`：群目录的根，默认 `./data/groups`。每个群一个子目录，任务写出的结果也在这里。
- `ROBROZ_CONTROL`：任务控制目录的根，默认 `./data/control`。

相对路径按启动 host 时的当前目录计算，换个目录启动就像换了一份数据。控制目录必须在群目录外面，两边都不能包住对方，否则 host 拒绝启动。跑任务用的镜像默认是 `robroz-runner:local`，用 `ROBROZ_IMAGE` 换。

## 接上模型

robroz 能接任意一套 OpenAI 兼容的 `/chat/completions` 服务。需要的是地址和模型名，密钥可选（Ollama 这类本机服务可以不填）。

最简单的是在客户端里填：「设置 → bro → 模型」，或者点交付卡上的「去设置」。

- 先在「服务商」里选一个（Gemini、Groq、OpenRouter、Ollama 或自定义），会自动填好地址；再填「模型名」和「密钥（选填）」。
- 点「保存」后会自动试连一次：成功显示「能用了」；失败显示「已保存，但连不上：原因」，设置已经存下，只是这次没连通。原因是「密钥不对或没填」时，说明服务返回了 401 或 403。
- 改完不用重启 host。

也可以用环境变量：`ROBROZ_MODEL_BASE_URL`（地址）、`ROBROZ_MODEL`（模型名）、`ROBROZ_MODEL_KEY_FILE` 或 `ROBROZ_MODEL_KEY`（密钥，只能设一个，推荐文件）。**设置页里保存的值优先于环境变量**，而且逐项比较：哪一项在设置页填过就用设置页的，没填的那一项再退回环境变量。设置页上标着「来自环境变量」，说明当前生效的值全部来自环境变量。

没有接模型、也没开测试回显时，任务的结果是「还没设置模型」，交付卡上有「去设置」。测试回显（`ROBROZ_MODEL=echo`）只能用环境变量打开，见 [安装上手](install.md#不接模型先试一下)。

## 联网搜索

可以选 [Brave Search](https://brave.com/search/api/)，或自建 [SearXNG](https://docs.searxng.org/)。在客户端「设置 → bro → 联网搜索」里填，也可以用设置接口或环境变量，见 [搜索和 MCP](search-and-mcp.md)。

## 其他常用的 host 变量

| 变量 | 默认 | 作用 |
| --- | --- | --- |
| `ROBROZ_SINGLE_RESPONDER` | 打开 | 群里没 @ 任何 bro 的消息只由一个 bro 回。设成 `0` 恢复成每个 bro 都轮一遍，见 [日常使用](usage.md#在群里说话) |
| `ROBROZ_GET_LIMIT` | `8` | 每个任务最多出网几次（联网搜索、读网页、只读的 MCP 工具都算） |
| `ROBROZ_MAX_TOOL_ROUNDS` | `8` | 一个任务里模型和工具最多来回几轮 |
| `ROBROZ_STEP_TIMEOUT_SECS` | `600` | 一步跑这么多秒还没完就取消，任务以「没写出结果」结束 |
| `ROBROZ_APPROVAL_TTL_SECS` | `86400`（24 小时） | 审批卡多久没人答就过期 |
| `ROBROZ_MCP_IDLE_SECS` | `600` | stdio MCP 服务器的进程闲置多久后停掉，`0` 表示每次都重新启动 |

## 客户端

客户端的大部分设置在界面里改，存在客户端自己的配置目录（Linux 是 `~/.config/robroz`，macOS 是 `~/Library/Application Support/robroz`，Windows 是 `%APPDATA%\robroz`）：

- 「设置 → 通用」：「Language / 语言」（English 或中文，默认英文）和「外观」（跟随系统、浅色、深色）。
- 「设置 → 连接 → 地址」：host 地址和令牌。令牌存进系统钥匙串。
- 「在这台设备上退出」：删掉这台设备上保存的令牌，host 那边什么都不变。

启动时还可以用这些环境变量，只对这一次运行有效，不会写回：

- `ROBROZ_HOST`、`ROBROZ_TOKEN`：host 地址和令牌，覆盖保存的值。客户端不读 host 的令牌文件。
- `ROBROZ_LANG=en|zh`：这次用的界面语言。
- `ROBROZ_USER_NAME`：这台客户端的名字。答审批卡、选项卡时会带上它，名字和本机相同的回答显示成「我」。不设时用电脑的主机名。你有几台设备时，别给它们起同一个名字。
- `ROBROZ_SCALE`：界面缩放倍数，默认 `1.0`。
- `ROBROZ_TOUCH=1`：一开始就按触屏放大点击区域；不设的话，检测到触摸后也会自动切换。

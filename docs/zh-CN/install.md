# 安装上手

robroz 分两部分：host（宿主机，后台服务）和客户端。host 推荐用 Docker 一条命令装好；客户端目前还没有发布版，要从源码构建。按下面的顺序做，就能跑起来，并跟第一个 bro 说上话。

日常会碰到的设置见 [配置](configuration.md)。英文的完整安装说明见 [docs/install.md](../install.md) 和 [docs/docker-host.md](../docker-host.md)。

## 1. 装 host（推荐：Docker 一条命令）

需要一台 Linux 机器，装好 Docker Engine 和 `docker compose` 插件，并且是 rootful Docker、默认的 socket 路径。镜像只有 linux/amd64。

```sh
curl -fsSL https://raw.githubusercontent.com/PandaQuQ/robroz/main/deploy/install.sh | sh
```

想先看看脚本，可以直接读 [`deploy/install.sh`](../../deploy/install.sh)。只想看它会做什么、不改任何东西，加 `--dry-run`：

```sh
curl -fsSL https://raw.githubusercontent.com/PandaQuQ/robroz/main/deploy/install.sh | sh -s -- --dry-run
```

脚本做这些事：

1. 检查 `docker`、`docker compose` 和 Docker 的 socket 能不能用，不行就停下并说明原因。rootless Docker 或改过 socket 路径的会被拒绝。
2. 建安装目录，默认 `~/robroz`。用 `--dir DIR` 或 `ROBROZ_DIR=...` 换位置；路径只能用字母、数字和 `/ . _ -`，不能有空格或中文。
3. 写好 `compose.yml`、一个只有本机设置的 `.env`，以及在你本机随机生成的令牌 `secrets/robroz_token`。脚本不问也不写任何账号密钥，模型的地址和密钥以后在客户端里填。
4. 拉取 `ghcr.io/pandaquq/robroz-host` 和 `ghcr.io/pandaquq/robroz-runner` 两个镜像，然后 `docker compose up -d`。本机已经有这个版本的镜像时跳过拉取。
5. 打印访问地址 `http://127.0.0.1:8790`（只在本机能访问）和下一步怎么做。令牌不会打印出来，只告诉你在哪个文件里。

脚本的参数：

| 参数 | 作用 |
| --- | --- |
| `--dir DIR` | 安装目录，默认 `~/robroz` |
| `--version TAG` | 装哪个版本，默认是脚本里写死的 `v0.2.0`；也可以设 `ROBROZ_VERSION` |
| `--dry-run` | 只打印要做的事，什么都不写、不启动 |
| `--no-start` | 写好文件，但不拉镜像、不启动 |
| `--no-pull` | 不拉镜像，镜像必须已经在本机 |
| `-h` | 显示帮助 |

镜像按版本打标签，没有 `latest`。升级就是再跑一次脚本（或加 `--version`）：它会替换 `compose.yml`（旧的留成 `compose.yml.bak`），但不会动已有的 `.env`、令牌和 `data/` 目录，数据库、群文件和设置都还在。

数据在安装目录下的 `data/`。看令牌：

```sh
cat ~/robroz/secrets/robroz_token
```

看日志：

```sh
cd ~/robroz && docker compose logs -f host
```

挂载 `docker.sock` 等于给了容器这台机器的 root 权限。在多人共用的服务器上装之前，先读 [docker-host.md 的安全说明](../docker-host.md#security-dockersock-is-root-on-this-machine)。

### 不接模型，先试一下

还没有模型密钥时，可以打开测试回显：bro 会把你发的话原样写成结果，用来确认整条链路是通的。在安装目录下新建 `compose.override.yml`：

```yaml
services:
  host:
    environment:
      ROBROZ_MODEL: echo
```

然后 `cd ~/robroz && docker compose up -d`。`docker compose` 会自动读这个文件，重跑安装脚本也不会覆盖它。

注意：

- 只在客户端的「模型」页里把模型名填成 `echo` 不会打开回显，必须用这个环境变量。
- 只要设置里已经有一套完整的模型（地址和模型名），就用那套模型，回显不起作用。
- 这是测试开关，正式使用时把它删掉。

### 另一种方式：从源码构建 host

想改代码或不用 Docker 跑 host 时，按英文 [install.md](../install.md#build) 构建 `robroz-host` 和 runner 镜像。简要步骤：

```sh
git clone https://github.com/PandaQuQ/robroz.git
cd robroz
cargo build -p robroz-host
docker build -f runner/Dockerfile -t robroz-runner:local .
./target/debug/robroz-host
```

需要 Rust stable 工具链和 Docker，跑 host 的用户要能直接执行 `docker`。这样起来的 host 默认只在 `127.0.0.1:8790` 监听，不设令牌；要设令牌或换数据位置，见 [配置](configuration.md)。

## 2. 构建客户端

robroz-client 还没有发布版。v0.2.0 的 host 要配基于 `contract-v0.3.1` 或更新版本的客户端，所以先从源码构建：

```sh
git clone https://github.com/PandaQuQ/robroz-client.git
cd robroz-client
cargo run -p robroz-client
```

桌面客户端能在 Windows、Linux、macOS 上构建，需要 Rust stable 工具链。Linux 上要有图形桌面（X11 或 Wayland）。细节见客户端仓库的 [install.md](https://github.com/PandaQuQ/robroz-client/blob/main/docs/install.md)。

手机端（`robroz-mobile`，Slint）是 v0.2 的试验版，还没在真机上测过，构建方法见 [mobile/README.md](https://github.com/PandaQuQ/robroz-client/blob/main/mobile/README.md)。

## 3. 连上 host

第一次打开客户端，连不上 host 时会显示「连不上宿主」，下面是连接表单：

1. 「地址」填 host 的地址。客户端和 host 在同一台机器上时就是 `http://127.0.0.1:8790`。
2. 「令牌」填 `secrets/robroz_token` 里的那串字符。
3. 点「连接」。

地址保存在客户端的配置目录里，令牌存进系统钥匙串（macOS 钥匙串、Windows 凭据管理器、Linux 的 Secret Service），钥匙串不可用时才写成本地文件。以后在「设置 → 连接 → 地址」里可以改。

也可以用环境变量 `ROBROZ_HOST` 和 `ROBROZ_TOKEN` 在启动时指定，它们只对这一次运行有效，不会写回保存的设置。客户端不读 host 的令牌文件。

令牌不对时会提示「令牌被拒绝」。host 在别的机器上时，不要直接把端口开到外网，要在前面加 HTTPS 反向代理或隧道，见 [配置](configuration.md#从别的机器连上来)。

## 4. 引入一个 bro，发出第一句话

最直接的开局是私聊。

1. 点会话列表顶部的圆形「+」，打开「引入一个 bro」页。给它起个名字（不填就默认叫 `bro`，已经有了就叫 `bro 2`、`bro 3`……），可以顺便选个头像，点「引入」。
2. 客户端会打开和它的私聊，标题旁标着「私聊」。
3. 在底部输入框里写一句话，回车发送。私聊里不用 `@`，话会直接派给这个 bro。

> 截图待补：引入 bro 之后的私聊界面。

### 想要一个有多个 bro 的群

客户端里还没有「建群」和「把 bro 拉进群」的按钮，要用 host 的接口来做。需要 `curl` 和 `jq`：

```sh
B=http://127.0.0.1:8790
T="Authorization: Bearer $(cat ~/robroz/secrets/robroz_token)"
J='content-type: application/json'

# 看你的 bro 的 id 和名字
curl -s $B/api/bros -H "$T"

# 建群
GID=$(curl -s -X POST $B/api/groups -H "$T" -H "$J" -d '{"name":"官网"}' | jq -r .id)

# 把已有的 bro 拉进群（botId 换成上一步看到的 id；name 只在新建 bro 时才用）
curl -s -X POST $B/api/groups/$GID/members -H "$T" -H "$J" -d '{"botId":"<bro 的 id>","name":"ada"}'
```

群会出现在客户端的列表里。群里怎么说话、谁来回，见 [日常使用](usage.md#在群里说话)。

## 你应该看到什么

发出去之后，对话里先出现你那句话，接着这个 bro 有一张状态卡，依次是「排队」「进行中」，做完变成「完成」，卡上带这次用了多久。随后多一张交付卡，指向群目录里的结果文件。

打开了测试回显时，结果就是你刚才那句话。既没接模型、也没开回显时，结果是「还没设置模型」，交付卡上有「去设置」按钮，点开就是「模型」页，见 [配置](configuration.md#接上模型)。

> 截图待补：状态卡从「排队」到「进行中」再到「完成」，并出现交付卡。

派了任务一直停在「排队」，或者变成「没写出结果」，见 [常见问题](faq.md)。

# Run the host in Docker

[中文](#中文) follows the English version below.

The host can run as a container. The image holds the `robroz-host` binary and the Docker CLI only. It has no Docker daemon, no Node and no `npx`. The host still starts each task in a sibling runner container on the machine's own Docker engine, so the host container needs access to that engine's socket.

- Image: `ghcr.io/pandaquq/robroz-host:<tag>` (linux/amd64 only; tagged per release; there is no `latest` tag). The package is public, so `docker pull` needs no login. Until the first release that includes the host image, only `main-<sha>` tags exist, which are built from `main` for CI; to run a fixed version, build it yourself (see below).
- Example compose file: [`deploy/compose.host.yml`](../deploy/compose.host.yml). To set it up with one command instead of by hand, see [One-command install](install.md#one-command-install-docker): `deploy/install.sh` writes this file, a `.env` and a random token, then pulls and starts the host.
- Dockerfile: [`host/Dockerfile`](../host/Dockerfile). Base image `debian:bookworm-slim`, the same as the runner image.
- The host binary release is unchanged. You can still run the host without Docker; see [install.md](install.md).

## Quick start with Compose

1. Create the data directory and a token file on the machine that runs Docker. Run this in the directory where you will keep `compose.host.yml`: the compose file reads `./secrets/robroz_token` relative to its own directory.

   ```sh
   sudo install -d -o 1000 -g 1000 /srv/robroz
   mkdir -p secrets
   (umask 077 && openssl rand -hex 24 > secrets/robroz_token)
   sudo chown 1000:1000 secrets/robroz_token   # the container user must be able to read it
   export DOCKER_GID="$(getent group docker | cut -d: -f3)"
   ```

2. Pull the runner image on the engine, with the same tag as the host image:

   ```sh
   docker pull ghcr.io/pandaquq/robroz-runner:v0.2.0
   ```

3. Copy [`deploy/compose.host.yml`](../deploy/compose.host.yml), adjust the tags, and start it:

   ```sh
   docker compose -f compose.host.yml up -d
   ```

4. Check it. Without the token you get 401; with the token you get 200:

   ```sh
   curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8790/api/bros
   curl -s -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer $(cat secrets/robroz_token)" http://127.0.0.1:8790/api/bros
   ```

To build the image yourself, run this from the repository root, because the Dockerfile copies the workspace:

```sh
docker build -f host/Dockerfile -t robroz-host:local .
```

## Security: `docker.sock` is root on this machine

The host starts runner containers with `docker run`, so it needs the engine socket. Mounting `/var/run/docker.sock` into the host container is **root-equivalent on that machine**: anything that can talk to the socket can start a privileged container or bind-mount `/`. If the host process is compromised, the machine is too.

- Prefer **rootless Docker** when you can. Point the mount at the rootless socket (`/run/user/<uid>/docker.sock`) and set `DOCKER_HOST=unix:///run/user/<uid>/docker.sock` for the container. The same-path rule below still applies.
- A **socket proxy** that allows only the container and image endpoints reduces the attack surface (no build, exec, swarm or volume APIs). It is not a security boundary: a compromised host could still create a privileged container through the allowed endpoints. If you use one, `docker run` may also call the image endpoints, so pre-pull `ROBROZ_IMAGE` if you block pulls.
- Do not use Docker-in-Docker for this.
- The runners themselves never get the socket. They run with `--network none` and two mounts, as described in [install.md](install.md#runner-image).
- The host container runs as the non-root user `robroz` (uid 1000). To write to the socket it needs the engine's `docker` group id, which differs between machines and cannot be baked into the image. The compose file adds it with `group_add: ["${DOCKER_GID}"]`. Do not run the host container as root: runners would then default to `--user 0:0` unless you set `ROBROZ_CONTAINER_USER`.

## Same absolute path inside and outside the container

When the host starts a runner it passes `-v <group dir>:/workspace` and `-v <control dir>:/control:ro` to `docker run`. The engine resolves those source paths on the **machine**, not inside the host container. So the data directory must appear at the same absolute path in both places. The compose file mounts `/srv/robroz:/srv/robroz` and sets:

| Variable | Value |
| --- | --- |
| `ROBROZ_DB` | `/srv/robroz/robroz.db` |
| `ROBROZ_VOLUMES` | `/srv/robroz/groups` |
| `ROBROZ_CONTROL` | `/srv/robroz/control` |

`groups` and `control` are siblings, which satisfies the rule that the control directory sits outside the volumes. The database is used only by the host and is not mounted into runners, but it must be persisted, so it lives under the same root.

If you pick a different path, change it on both sides of the mount and in all three variables. A mismatch such as `-v /srv/robroz:/data` with `ROBROZ_VOLUMES=/data/groups` makes the engine mount a path that does not exist on the machine (or creates an empty one owned by root). The runner then cannot see the group files, and the host never sees `result.txt`.

Set `ROBROZ_CONTAINER_USER` (the example uses `1000:1000`) to the uid:gid that should own the files the runners write, and own `/srv/robroz` with the same ids.

## Token

- Inside the container the host listens on `0.0.0.0:8790`, so a token is mandatory. Without `ROBROZ_TOKEN` or `ROBROZ_TOKEN_FILE` the host exits with code 2.
- Use a Compose **secret** file mounted at `/run/secrets/robroz_token` and `ROBROZ_TOKEN_FILE=/run/secrets/robroz_token`. Do not bake the token into an image, commit it, or write it into `environment:`. Set only one of `ROBROZ_TOKEN` and `ROBROZ_TOKEN_FILE`. The token must be at least 16 characters with no whitespace inside.
- The secret file is bind-mounted with its host owner and mode. Make it readable by the container user (`USER robroz`, uid 1000 in the image; compose does not set `user:`). For a manual setup, `chown 1000:1000` and mode `0600` is enough. `deploy/install.sh` instead uses mode `0640`, owner the installing user, group the container's gid, so you can read the file without being uid 1000. `group_add` / `DOCKER_GID` only grant access to the engine socket, not to the token.
- Every `/api` request needs `Authorization: Bearer <token>`, including requests from the same machine. A missing or wrong token returns 401.
- A model key follows the same pattern: a second secret and `ROBROZ_MODEL_KEY_FILE`, or enter it in the client. See [configuration.md](configuration.md).

## Reverse proxy or tunnel in front

The compose file publishes the port only on loopback: `127.0.0.1:8790:8790`. The host does not terminate TLS. To reach it from another machine, put an HTTPS reverse proxy or a tunnel in front of `127.0.0.1:8790` and give the client the `https://` address. A non-local `http://` host shows Unencrypted in the client. Do not publish the port on `0.0.0.0`.

## Stdio MCP servers and Node

stdio MCP servers run as child processes of the host, so they run inside this container. The default image has no Node or `npx`. To use a stdio server that needs them, extend the image in your own Dockerfile:

```dockerfile
FROM ghcr.io/pandaquq/robroz-host:v0.2.0
USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends nodejs npm \
    && rm -rf /var/lib/apt/lists/*
USER robroz
```

Build it, and point the `image:` line of the compose file at your tag. You can also use an HTTP MCP server, which needs nothing in the image. A `-node` variant of the image is not planned unless people ask for it.

## Not covered yet

- A policy socket proxy that allows only the configured `ROBROZ_IMAGE`, `--network none` and bind sources under the data root.
- `linux/arm64` images.
- Private runner images: `docker run` uses the credentials of the Docker CLI inside the host container, which has none. Pre-pull the runner image on the engine instead.
- Docker Desktop on Windows and macOS (path translation).

---

## 中文

<a id="中文"></a>

宿主机可以作为容器运行。镜像里只有 `robroz-host` 二进制和 Docker 客户端（CLI），没有 Docker 守护进程，也没有 Node 和 `npx`。宿主机仍然在这台机器自己的 Docker 引擎上，为每个任务起一个并列的 runner 容器，所以宿主机容器必须能访问这个引擎的 socket。

- 镜像：`ghcr.io/pandaquq/robroz-host:<tag>`（仅 linux/amd64；每个发布一个 tag，没有 `latest`）。这个包是公开的，`docker pull` 不用登录。在第一个包含 host 镜像的发布之前，只有从 `main` 构建、给 CI 用的 `main-<sha>` tag；想跑固定版本，请自己构建（见下）。
- Compose 示例：[`deploy/compose.host.yml`](../deploy/compose.host.yml)。
- Dockerfile：[`host/Dockerfile`](../host/Dockerfile)。基础镜像是 `debian:bookworm-slim`，与 runner 镜像相同。
- 宿主机二进制的发布方式不变。不用 Docker 也可以直接运行宿主机，见 [install.md](install.md)。

### 用 Compose 快速开始

1. 在运行 Docker 的机器上创建数据目录和令牌文件。请在准备放 `compose.host.yml` 的目录里执行：compose 文件里的 `./secrets/robroz_token` 是相对它自己所在目录的路径。

   ```sh
   sudo install -d -o 1000 -g 1000 /srv/robroz
   mkdir -p secrets
   (umask 077 && openssl rand -hex 24 > secrets/robroz_token)
   sudo chown 1000:1000 secrets/robroz_token   # 容器里的用户必须能读
   export DOCKER_GID="$(getent group docker | cut -d: -f3)"
   ```

2. 在引擎上先拉取 runner 镜像，tag 与宿主机镜像一致：

   ```sh
   docker pull ghcr.io/pandaquq/robroz-runner:v0.2.0
   ```

3. 复制 [`deploy/compose.host.yml`](../deploy/compose.host.yml)，按需改 tag，然后启动：

   ```sh
   docker compose -f compose.host.yml up -d
   ```

4. 验证。不带令牌返回 401，带令牌返回 200：

   ```sh
   curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8790/api/bros
   curl -s -o /dev/null -w '%{http_code}\n' -H "Authorization: Bearer $(cat secrets/robroz_token)" http://127.0.0.1:8790/api/bros
   ```

自己构建镜像时，在仓库根目录执行下面的命令，因为 Dockerfile 要复制整个工作区：

```sh
docker build -f host/Dockerfile -t robroz-host:local .
```

### 安全：`docker.sock` 等于这台机器的 root

宿主机用 `docker run` 启动 runner 容器，所以需要引擎的 socket。把 `/var/run/docker.sock` 挂进宿主机容器，**等同于在这台机器上给了 root**：能访问 socket 的进程可以启动特权容器，或把 `/` 挂进去。宿主机进程一旦被攻破，整台机器也就被攻破。

- 有条件就用 **rootless Docker**。把挂载指向 rootless 的 socket（`/run/user/<uid>/docker.sock`），并给容器设 `DOCKER_HOST=unix:///run/user/<uid>/docker.sock`。下面的同路径规则照样适用。
- 只放行容器和镜像接口的 **socket 代理**可以缩小攻击面（没有 build、exec、swarm、volume 接口），但它不是安全边界：被攻破的宿主机仍可以通过放行的接口创建特权容器。用了代理的话，`docker run` 也可能调用镜像接口，如果代理禁止拉取，请先在引擎上拉好 `ROBROZ_IMAGE`。
- 不要为此使用 Docker-in-Docker。
- runner 自己拿不到这个 socket。它们以 `--network none` 运行，只有两个挂载，见 [install.md](install.md#runner-image)。
- 宿主机容器用非 root 用户 `robroz`（uid 1000）运行。要写 socket，需要引擎的 `docker` 组 id，它因机器而异，不能写进镜像。Compose 示例用 `group_add: ["${DOCKER_GID}"]` 加上。不要用 root 运行宿主机容器：否则除非设置 `ROBROZ_CONTAINER_USER`，runner 会默认用 `--user 0:0`。

### 容器内外要用同一个绝对路径

宿主机起 runner 时，会给 `docker run` 传 `-v <群目录>:/workspace` 和 `-v <control 目录>:/control:ro`。这些源路径由引擎在**机器上**解析，而不是在宿主机容器里。所以数据目录在两边必须是同一个绝对路径。Compose 示例挂载 `/srv/robroz:/srv/robroz`，并设置：

| 变量 | 值 |
| --- | --- |
| `ROBROZ_DB` | `/srv/robroz/robroz.db` |
| `ROBROZ_VOLUMES` | `/srv/robroz/groups` |
| `ROBROZ_CONTROL` | `/srv/robroz/control` |

`groups` 和 `control` 是并列目录，满足「control 必须在 volumes 之外」的规则。数据库只有宿主机用，不会挂进 runner，但必须持久化，所以放在同一个根目录下。

要换路径，就在挂载的两边和三个变量里一起改。比如 `-v /srv/robroz:/data` 配 `ROBROZ_VOLUMES=/data/groups` 就会不一致：引擎去挂机器上不存在的路径（或建一个 root 所有的空目录），runner 看不到群文件，宿主机也读不到 `result.txt`。

把 `ROBROZ_CONTAINER_USER`（示例是 `1000:1000`）设成应当拥有 runner 所写文件的 uid:gid，并让 `/srv/robroz` 属于同一组 id。

### 令牌

- 容器内宿主机监听 `0.0.0.0:8790`，所以令牌是必须的。没有 `ROBROZ_TOKEN` 或 `ROBROZ_TOKEN_FILE`，宿主机会以退出码 2 退出。
- 用 Compose 的 **secret** 文件挂到 `/run/secrets/robroz_token`，并设 `ROBROZ_TOKEN_FILE=/run/secrets/robroz_token`。不要把令牌打进镜像、提交进仓库，或写在 `environment:` 里。`ROBROZ_TOKEN` 和 `ROBROZ_TOKEN_FILE` 只能设一个。令牌至少 16 个字符，中间不能有空白。
- secret 文件按宿主上的属主和权限 bind mount。要让容器里的用户能读（镜像里是 `USER robroz`，uid 1000；compose 没有设 `user:`）。手工安装时，`chown 1000:1000`、权限 `0600` 即可。`deploy/install.sh` 会改成权限 `0640`、属主是执行脚本的用户、组是容器的 gid，这样你不必是 uid 1000 也能读。`group_add` / `DOCKER_GID` 只用来访问引擎 socket，和令牌文件无关。
- 每个 `/api` 请求都要带 `Authorization: Bearer <令牌>`，本机请求也一样。缺少或错误的令牌返回 401。
- 模型密钥同理：再加一个 secret 和 `ROBROZ_MODEL_KEY_FILE`，或者在客户端里填。见 [configuration.md](configuration.md)。

### 前面放反向代理或隧道

Compose 示例只在本机回环上发布端口：`127.0.0.1:8790:8790`。宿主机自己不做 TLS。要从别的机器访问，请在 `127.0.0.1:8790` 前面放 HTTPS 反向代理或隧道，并让客户端用 `https://` 地址。客户端遇到非本机的 `http://` 会显示「未加密」。不要把端口发布到 `0.0.0.0`。

### stdio MCP 服务器和 Node

stdio MCP 服务器是宿主机的子进程，所以它们跑在这个容器里。默认镜像没有 Node 和 `npx`。要用依赖它们的 stdio 服务器，请在自己的 Dockerfile 里扩展镜像：

```dockerfile
FROM ghcr.io/pandaquq/robroz-host:v0.2.0
USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends nodejs npm \
    && rm -rf /var/lib/apt/lists/*
USER robroz
```

构建后，把 compose 里的 `image:` 改成你的 tag。也可以改用 HTTP MCP 服务器，镜像里不需要任何额外东西。除非有人需要，否则不会出 `-node` 变体镜像。

### 暂不包含

- 带策略的 socket 代理：只允许配置的 `ROBROZ_IMAGE`、`--network none` 和数据根目录下的 bind 源。
- `linux/arm64` 镜像。
- 私有的 runner 镜像：`docker run` 用的是宿主机容器里 Docker CLI 的凭据，而它没有凭据。请改为在引擎上预先拉取 runner 镜像。
- Windows 和 macOS 上的 Docker Desktop（路径转换）。

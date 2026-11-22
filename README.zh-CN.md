<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
    <img src="assets/brand/wordmark-light.svg" alt="robroz" width="240">
  </picture>
</p>

<p align="center"><a href="README.md">English</a> | <b>简体中文</b></p>

<p align="center">
  <a href="https://github.com/PandaQuQ/robroz/actions/workflows/ci.yml"><img src="https://github.com/PandaQuQ/robroz/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
</p>

**robroz** 是单机私有化的多智能体协同工作台，交互是聊天，不是 IDE。你在群和私聊里把任务交给 *bro*（agent）；每个 bro 在你本机的 Docker 沙箱里干活，进度、审批和结果都以卡片回到对话里。宿主机、执行器（runner）和原生桌面客户端（Windows、Linux、macOS）全部用 Rust 写，不套网页。

<!-- 截图待补：390 px 和 1440 px 各一张，README.md 和 README.zh-CN.md 共用同一组。 -->

## 功能

第一期（已在 `main` 上）：

- **群和私聊。** 在群里以 `@名字` 开头就是给这个 bro 派任务；私聊里每条消息都是任务。状态卡和交付卡、实时推送、未读数。
- **三级权限。** 读写群目录直接放行；出网只许 GET，并按任务计次；对外写入和验证码会停下任务，等人处理。
- **要你回答的卡片。** 审批卡、选项卡，以及例行任务（到点执行并交付简报）。
- **记忆和技能。** bro 提议一条记忆或一个技能，你确认后才生效。
- **接任意 OpenAI 兼容模型**，联网搜索（Brave 或 SearXNG），以及宿主机上的 MCP 工具（写工具先出审批卡再调用）。

第一期不做：文件树、终端、桌面流、Bot 详情页、Webhook、E2B、多租户。

## 快速开始

需要 Rust stable 工具链和 Docker，示例里还用到 `curl`。在仓库根目录：

```sh
cargo build -p robroz-host -p robroz-client                # 宿主机和桌面客户端
docker build -f runner/Dockerfile -t robroz-runner:local .  # 跑任务用的 runner 镜像
cargo run -p robroz-host                                    # 只监听 127.0.0.1:8790，不用令牌
cargo run -p robroz-client                                  # 另开一个终端
```

在客户端「聊天」标题旁点「引入一个 bro」，进它的私聊发一条消息，这就是第一个任务。

robroz 没有默认模型，用哪个由你在部署宿主机时选。任何 OpenAI 兼容的 `/chat/completions` 服务都可以。启动宿主机前设好 `ROBROZ_MODEL_BASE_URL`、`ROBROZ_MODEL` 和 `ROBROZ_MODEL_KEY_FILE`（存放密钥的文件），或者之后用 `PUT /api/settings/model` 补上，不用重启。选好模型之前，任务照样会做完，但结果是「还没设置模型」，交付卡带 `needsSetup: "model"`。

选模型、设令牌、从别的机器访问宿主机、用 `curl` 走一遍接口，见 [docs/install.md](docs/install.md)。

## 配置

宿主机和客户端都用环境变量配置（监听地址、令牌、数据库、模型、搜索、runner 镜像）。完整列表和默认值见 [docs/configuration.md](docs/configuration.md)。模型和联网搜索也可以用设置接口填（搜索还有客户端的「联网搜索」页），那里保存的值逐项优先于环境变量。密钥和令牌放在仓库外的文件里（`*_FILE` 变量），不要提交进仓库。

## 文档

- **给人看：** [robroz wiki](https://github.com/PandaQuQ/robroz/wiki)，包括安装、日常使用、常见问题和路线图。
- **给编码 agent 和贡献者看：** [AGENTS.md](AGENTS.md) 和 [`docs/`](docs/)，包括仓库规则、接口契约、字段、环境变量和代码位置。wiki 不是契约，两边冲突时以仓库为准。

| 文件 | 内容 |
| --- | --- |
| [docs/install.md](docs/install.md) | 构建、运行、接模型、第一个任务、测试 |
| [docs/configuration.md](docs/configuration.md) | 全部环境变量 |
| [docs/api.md](docs/api.md) | 全部 HTTP 接口和事件 |
| [docs/concepts.md](docs/concepts.md) | 术语、任务状态、卡片 |
| [docs/permissions.md](docs/permissions.md) | 三级权限、审批、接管、`failure` |
| [docs/memory-and-skills.md](docs/memory-and-skills.md) | 记忆、技能、确认卡 |
| [docs/search-and-mcp.md](docs/search-and-mcp.md) | 联网搜索和 MCP |
| [docs/client.md](docs/client.md) | 客户端界面行为 |
| [docs/architecture.md](docs/architecture.md) | 各部分分工、容器、外观层 |
| [docs/reference-features.md](docs/reference-features.md) | 参考产品和净室规则 |
| [docs/roadmap-phase2.md](docs/roadmap-phase2.md) | 第二期路线图 |

bro 是界面上对 bot 的叫法：代码、接口和数据库里照旧叫 bot（`botId` / `botName`）。私聊就是只有你和一个 bro 的群。

## 参与开发

### 目录

| 路径 | crate | 作用 |
| --- | --- | --- |
| `host/` | `robroz-host` | HTTP 接口（axum）、SQLite、容器 pump、定时、调用模型、搜索、MCP 配置和工具调用 |
| `runner/` | `robroz-runner` | 容器里的任务执行器 |
| `client/` | `robroz-client` | iced 桌面客户端 |
| `crates/robroz-contract` | `robroz-contract` | 接口契约：请求体和返回体、事件（卡片）类型、任务状态 |
| `crates/robroz-policy` | `robroz-policy` | 权限分级、出网计次、阻塞行 |
| `crates/robroz-card` | `robroz-card` | 卡片的状态文案、色调和动作 |
| `crates/robroz-tokens` | `robroz-tokens` | 由 `tokens.json` 生成的设计 token |
| `crates/robroz-ui` | `robroz-ui` | 客户端共用的界面层和主题 |
| `crates/robroz-model` | `robroz-model` | OpenAI 兼容 `/chat/completions` 客户端，含工具调用 |
| `crates/robroz-mcp` | `robroz-mcp` | MCP 客户端（stdio 和 Streamable HTTP）：列出并调用工具 |

各部分怎么配合见 [docs/architecture.md](docs/architecture.md)。

### 构建和测试

开 PR 前跑一遍和 CI 相同的命令：

```sh
cargo test -p robroz-mcp --features fixture
cargo test -p robroz-host -p robroz-contract -p robroz-policy -p robroz-card -p robroz-runner -p robroz-tokens
cargo build -p robroz-client
cargo test -p robroz-client -p robroz-ui -p robroz-tokens
scripts/e2e.sh   # 需要 docker、curl、jq
```

CI 在 PR 上只构建 Linux 客户端；合到 `main` 后和每天定时会构建 Windows、Linux、macOS 三个平台。

### 规则

- 一个 PR 只做一件事。
- 每位贡献者都要签一次[贡献者许可协议](CLA.md)：把 `CLA.md` 里那行声明加到 PR 描述里（PR 模板里有 CLA 勾选项的话，勾上即可）。没有签署的 PR 不会合并。
- 提交信息、PR 标题和正文都用英文，按 [Conventional Commits](https://www.conventionalcommits.org/) 写：`feat(host): ...`、`fix(client): ...`、`docs: ...`。PR 用 squash merge 合并，PR 标题就是合并后的提交信息。
- 契约只增不改：可以新增卡片类型、接口和可选字段；已有字段、任务状态（`queued` / `running` / `blocked` / `done`）和 `botId` / `botName` 都冻住不改。
- 不 force-push。分支落后时把 `main` merge 进来。
- 不提交 `.env`、`.pem`、`.dev.vars`、密钥或令牌。文档里用占位符。
- `README.md` 和 `README.zh-CN.md` 保持一致，在同一个 PR 里一起改。

给 agent 和贡献者的完整规则见 [AGENTS.md](AGENTS.md)。

## 许可证

SPDX-License-Identifier: `AGPL-3.0-only`

robroz 采用 [GNU Affero 通用公共许可证 v3.0 only](LICENSE)。修改了 robroz 并通过网络给别人用的，必须向这些用户提供改过的源码（AGPL 第 13 条）。

闭源或专有用途可以另外取得商业授权，见 [COMMERCIAL.md](COMMERCIAL.md)。商业授权请[联系作者](https://github.com/PandaQuQ)。

贡献按[贡献者许可协议](CLA.md)接收。

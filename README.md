<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
    <img src="assets/brand/wordmark-light.svg" alt="robroz" width="240">
  </picture>
</p>

# robroz

robroz 是单机私有化的多智能体协同，交互是 IM，不是 IDE。你在「群」和「私聊」里给 bro（一个 agent）派任务，bro 在本机的 Docker 沙箱里干活，进度、审批和结果都以卡片回到对话里。宿主机、执行器（runner）和客户端都用 Rust 写；客户端是原生程序，Windows、Linux、macOS 各出一个包，不用 Tauri，也不套网页。

**第一期做的：**

- 群和私聊，`@名字` 派任务，状态卡、交付卡、实时推送和未读数
- 三级权限：群目录内放行，出网只许 GET 并计次，对外写入和验证码停下来等人
- 审批卡、选项卡、例行任务（到点出简报）
- 记忆和技能：bro 提议，你确认后才生效
- 接任意 OpenAI 兼容模型，联网搜索（Brave 或 SearXNG），MCP 服务器配置

**第一期不做的：** 文件树、终端、桌面流、Bot 详情、Webhook、E2B、多租户。

<!-- 截图待补：390 和 1440 各一张 -->

## 快速开始

需要 Rust stable、Docker，示例里还用到 `curl`。在仓库根目录：

```sh
cargo build -p robroz-host -p robroz-client                # 宿主机和客户端
docker build -f runner/Dockerfile -t robroz-runner:local .  # 跑任务用的 runner 镜像
cargo run -p robroz-host                                    # 默认只监听 127.0.0.1:8790，不用令牌
cargo run -p robroz-client                                  # 另开一个终端，打开客户端
```

在客户端点「＋ 新 bro」，进它的私聊说一句话，就会派出第一个任务。不接模型时 bro 把你的话原样写成结果；接模型、设令牌、跨机器访问、用 curl 走一遍，见 [docs/install.md](docs/install.md)。

## 文档

- **给人看：** [robroz wiki](https://github.com/PandaQuQ/robroz/wiki)：安装上手、日常怎么用、常见问题、路线图。
- **给 agent 看：** [AGENTS.md](AGENTS.md) 和 [`docs/`](docs/)：仓库规矩、接口契约、字段、环境变量和代码位置。wiki 不是契约，冲突时以仓库为准。

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

## 术语

bro 就是 bot：界面上叫 bro，代码、接口和数据库里照旧叫 bot（`botId` / `botName`）。私聊是只有你和一个 bro 的群。完整术语表见 [docs/concepts.md](docs/concepts.md)。

## 测试

```sh
cargo test -p robroz-host -p robroz-contract -p robroz-policy -p robroz-card -p robroz-runner -p robroz-tokens
cargo test -p robroz-client -p robroz-ui
scripts/e2e.sh   # 需要 docker、curl、jq
```

CI 跑宿主机测试和 e2e，并构建客户端：PR 上只构建 Linux，合到 `main` 后和每天定时再构建 Windows 和 macOS。完整命令见 [AGENTS.md](AGENTS.md#构建和测试)。

## 许可证

许可证待定。

# robroz 中文文档

robroz 是一套私有化部署的多智能体协作工具。交互方式是即时通讯，不是 IDE：你和 bro 说话、派活、看结果，都在对话里完成。

它跑在你自己的机器上。后台是用 Rust 写的宿主机（host），每个任务在本机的 Docker 容器里由执行器（runner）跑完。客户端在单独的仓库 [robroz-client](https://github.com/PandaQuQ/robroz-client)：桌面端是 Windows、Linux、macOS 上的原生程序，不套网页；手机端是用 Slint 写的安卓和 iOS 应用，目前还是试验版。

这里的 bro 就是一个 agent。界面上叫「引入一个 bro」，就是新建一个 agent。和某个 bro 单独说话叫私聊，其实也是一个群，只是里面只有你和它。

这套中文文档原先放在 GitHub Wiki，从 v0.2.0 起搬进仓库的 `docs/zh-CN/`，和代码一起走 PR 审阅。

## 现在的版本

最新发布是 [v0.2.0](https://github.com/PandaQuQ/robroz/releases/tag/v0.2.0)，预发布版本。这次只发 host：一个 Linux x86_64 压缩包，以及 GHCR 上的 `robroz-host` 和 `robroz-runner` 两个镜像。客户端从 robroz-client 发布，那边还没有发过版，目前要从源码构建，用基于 `contract-v0.3.1` 或更新版本的客户端。

v0.2.0 已经能做的：

- 用 Docker 一条命令装好 host，见 [安装上手](install.md)
- 引入 bro、和它私聊；建群，把 bro 拉进群，在群里用 `@名字` 派活
- 群里没 @ 任何 bro 的消息，由 host 挑一个 bro 回，不再每个 bro 都回一遍
- 你引入的 bro 归你所有：可以改名、选头像、拉进群，它的审批卡由你来批
- 群资料页、成员列表、改群名、删群、群内搜索
- 每个任务在隔离的容器里执行，状态卡带准确计时，断线重连后会补上漏掉的卡片
- 接任意一套 OpenAI 兼容的模型；先不接也能把流程跑通
- 三级权限、审批卡、放行规则（自动审核）
- 记忆和技能、例行任务
- 联网搜索：自己选 [Brave Search](https://brave.com/search/api/) 或自建 [SearXNG](https://docs.searxng.org/)，密钥留在本机
- MCP 服务器：bro 能调用它的工具；不是只读的工具，用之前先出审批卡问你

还没有的：网页版、文件树、终端、桌面流、Webhook。v0.2.0 的 host 只有一个令牌，连上来的就是你自己（`local-admin`）；多用户账号排在 v0.3，届时群、bro 和任务按用户隔离。手机端只能聊天和看卡片，设置里只有语言、外观、MCP（只读）和退出；技能、例行任务和记忆只在桌面端。完整的已知问题见 [v0.2.0 发布说明](https://github.com/PandaQuQ/robroz/releases/tag/v0.2.0)，后面的打算见 [路线图](roadmap.md)。

## 文档目录

- [安装上手](install.md)：装好 host 和客户端，发出第一句话
- [配置](configuration.md)：地址、令牌、数据位置、模型、远程访问，以及客户端的设置
- [日常使用](usage.md)：群和私聊、卡片、未读、例行任务，以及客户端有哪些页面
- [权限和审批](permissions.md)：bro 能做什么、什么时候要你点头、任务为什么没做成
- [记忆和技能](memory-and-skills.md)：bro 怎样记住你的事、学会一种做法
- [搜索和 MCP](search-and-mcp.md)：联网搜索怎么开，MCP 怎么配、bro 怎么用
- [概念](concepts.md)：术语表
- [常见问题](faq.md)
- [路线图](roadmap.md)：v0.3 和以后打算做的事

更细的接口、字段和环境变量，见 [英文参考文档](../)：[install.md](../install.md)、[configuration.md](../configuration.md)、[api.md](../api.md)、[permissions.md](../permissions.md) 等。客户端界面的细节见 robroz-client 的 [client.md](https://github.com/PandaQuQ/robroz-client/blob/main/docs/client.md)。

## 许可证

robroz 用 AGPL-3.0 开源，谁都可以用、改、提 PR；想闭源商用的，可以找作者买商业授权。贡献代码前要先签贡献者许可协议（CLA）。细节见 [常见问题](faq.md#许可证是什么)。

---

## 给 robots 的说明

如果你是 AI agent，请不要把 `docs/zh-CN/` 当作接口契约或实现依据。这里是写给人看的使用说明。请改读：

- [AGENTS.md](../../AGENTS.md)
- [docs/](../)（接口、字段、环境变量和代码位置的精确参考）

两边如果有出入，以英文参考文档和代码为准。

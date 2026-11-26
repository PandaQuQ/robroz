# 参考功能清单

robroz 的目标是做出一个开源版的 Grok Bot：用 Rust 自研、单机可私有化的多智能体协同。本清单只记录参考产品**用户能看见、用得着的功能和行为**，供 Panda 勾选后按节点排期。写 Rust 和客户端的人照这份清单实现。读参考产品源码的规则见下面的「清洁室与来源边界」，按许可证分。

固定版本（只读其 README、`docs/`、许可证和公开预览/截图）：

| 来源 | 固定版本 | 说明 |
| --- | --- | --- |
| [Comma](https://github.com/AFK-surf/Comma) | [`66ab3dc92475005aea9afd123df36a28e4ef86ab`](https://github.com/AFK-surf/Comma/tree/66ab3dc92475005aea9afd123df36a28e4ef86ab) | AGPL-3.0-only |
| [CopilotKit OpenBot](https://github.com/CopilotKit/openbot) | [`8d18f5d88be67334678c7f18c54c3b4797976a20`](https://github.com/CopilotKit/openbot/tree/8d18f5d88be67334678c7f18c54c3b4797976a20) | 应用 MIT；CopilotKit Intelligence 另行授权，robroz 不使用 |
| [OpenMausBot](https://github.com/milind-soni/OpenMausBot) | [`ecabd15caa08f674877e155d03c553c30a35a63b`](https://github.com/milind-soni/OpenMausBot/tree/ecabd15caa08f674877e155d03c553c30a35a63b) | 主体 Apache-2.0；`enterprise/` 为独立商业许可，**不参考、不使用** |
| Grok Bot 用户能看到的行为 | — | 只写用户看得见的产品行为，不写内部提示词或配置 |

「第一期？」按 robroz 已冻结的范围填写：单机私有化，跑通「说话、建群、拉 Bot、本机 Docker 隔离运行、做完休眠」，外加一次定时简报和一次私聊接管。文件树、终端、桌面流、Bot 详情页、Webhook、MCP 网关、浏览器自动化、多租户等不在第一期。勾选列按 Panda 2026-11-20 的要求（「我只要一个完美的完整的 grok bot 体验」）由 PM 拍板后勾选：「是」的行打勾，待定项已全部定完。

## 契约变更规则（2026-11-20 起）

第一期契约（节点 1）只开一个口子：**可以新增卡片类型和接口，已有字段一个都不改**，包括字段名、类型、含义和已有枚举值。新卡片和新接口必须是加法，旧客户端忽略不认识的卡片类型和字段后照样能用。改已有字段仍然要 Panda 拍板。

第一期据此新增两项：

- 选项卡：新增卡片类型 `choice`，带 `options[]`，用户的选择以 `{"choice":[ids]}` 回传，单选也传数组。状态卡保持冻结，不加字段。
- 例行任务：新增 `GET` 列表接口和 `PATCH` 的 `paused` 字段。
- 审批卡（2026-11-20 定）：在私聊 PR 合入之后做，按新增接口来。批准和拒绝各走一个回答接口，同一张卡只能答一次，规则和选项卡一致。

## IM 基础（2026-11-20 进第一期）

robroz 做成 IM 软件，对标微信和 WhatsApp。下面四项 2026-11-20 由 PM 定为第一期，后端按顺序做，每项单独一个 PR，都只做新增，不改已有字段：

| 顺序 | 功能 | 用户能看到什么 | 第一期做法 | 勾选 |
| --- | --- | --- | --- | --- |
| 1 | 监听地址加令牌鉴权 | 手机、网页版等远程客户端能连上宿主机。 | 监听地址可配置。默认仍只听 `127.0.0.1`，本机不用令牌；监听地址不是本机时，没配令牌就拒绝启动。令牌只放配置文件或 `ROBROZ_*` 环境变量，不写进日志。 | [x] |
| 2 | 实时推送 | 新消息和卡片变化立刻出现，不用等下一轮刷新。 | 新增一个事件流接口推送更新。现有 `GET /api/groups/:id/events` 轮询（含 `?since=`）留着做兜底。 | [x] |
| 3 | 未读数 | 会话列表每行有未读红点和数字。 | 每个群记一个已读位置，未读数由它算出。 | [x] |
| 4 | 私聊 | 和单个 bot 一对一聊天，与群聊混在同一个会话列表里，按时间排序。 | 实现上就是只有你和一个 bot 的群，不新造概念。 | [x] |

## 目的与规则

- 只参考功能和行为，用来对照 robroz 已有节点（节点 1 契约、节点 2 宿主机编排、节点 3 沙盒和共享卷、节点 4 权限、节点 5 定时、节点 6 客户端、节点 7 联调）。不新增节点；对不上已有节点的标「后续（第一期范围外）」。
- 不抄、不逐行翻译对方的应用源码。Elixir 或 TypeScript 逐行译成 Rust 同样算衍生作品。宽松许可的项目可以读源码学实现，见「清洁室与来源边界」。
- 每项在「来源」列注明出处，链接到该版本的 README 或文档章节。没在文档或公开预览里核到的不写，或标「待核」。
- OpenMausBot 的 `enterprise/` 以及仅在该目录/商业许可下出现的能力（白标、公司 SSO、托管登录适配、许可制月度支出上限、按模型售价出账等）一律不列入功能行。

表头：功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选

## 持久任务与执行

| 功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 请求变成可持久任务 | 简短提问可在对话里直接回答；较大的工作成为独立任务，在「进行中 / 待审 / 完成」一类状态里流转。同一任务只改状态，不另开一张卡。 | [Comma README「What Comma does」](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does)；[Comma 产品概念：Task lifecycle](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/architecture/PRODUCT_MODEL.md#核心概念) | 节点 2、节点 6 | 是（四态为 queued / running / blocked / done，没有单独的「待审」看板） | [x] |
| 后台执行，用户可继续聊天 | 长任务放到后台跑，聊天不卡住。进行中用状态卡原地更新。 | **Grok Bot 用户能看到的行为**；[Comma README：Tasks 是持久执行状态](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#philosophy)；[OpenBot README：在频道里看它工作](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#main-surfaces)；[OpenMausBot README：Status 环路含流式回复与工具](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#status) | 节点 2、节点 3、节点 6 | 是（状态卡排队 / 进行 / 卡住 / 完成） | [x] |
| 关键节点汇报进度 | 执行中在关键里程碑推一次进度，而不是不停刷屏。 | **Grok Bot 用户能看到的行为** | 节点 6 | 是（2026-11-20 PM 定入第一期；原备注：第一期已有四态原地更新；更细的里程碑文字需先对契约） | [x] |
| 收到任务先短确认 | 先回一句短确认再开始干活，之后只在关键节点更新。 | **Grok Bot 用户能看到的行为**（界面建议进第一期） | 节点 6 | 是（2026-11-20 PM 定入第一期；原备注：可用文本消息，但契约未要求执行器必须先发确认） | [x] |
| 卡住时提醒需要输入 | 任务需要人决定时停在「需要输入 / 卡住」，并给出下一步入口。 | [Comma 任务文档：escalated 显示为 Needs input](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/salix/tasks-background-execution.md#canonical-lifecycle)；robroz 卡住只留「去私聊接管」 | 节点 4、节点 6 | 是 | [x] |

## 审批与权限

| 功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 写外部资产或遇到验证码则停下 | 不代填凭据。任务变成卡住，删掉容器，人到私聊接管；恢复后挂回原来的共享卷。 | robroz 已冻结的权限契约；[OpenBot README：登录墙或 2FA 时把控制权交给用户](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features) | 节点 4 | 是 | [x] |
| 不可逆操作先出审批卡 | 对外发送、删除、花钱等不能撤回的操作，必须先出审批卡，用户点了才执行。这是节点 4 权限里的一级。拒绝或过期就是最终结果，不能自动重试。 | **Grok Bot 用户能看到的行为**；[OpenBot docs/approvals.md](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/approvals.md)；[OpenMausBot docs/approval-levels.md「Sending on your behalf」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/approval-levels.md)；[Comma README：Needs Review](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does) | 节点 4 | 是 | [x] |
| 审批级别与当场作答 | 每个 bot 可有审批级别（每次询问、自动接受编辑、代为批准、完全放行等）。会话里的审批卡可「允许一次」「本次会话允许」；对可信只读工具可给长期允许，并在该 bot 的访问设置里撤销。同一房间里的另一个 bot 仍会再问。问题卡任何级别都不会被自动代答。对外发送、删除、花钱这一级不可关掉，必须先出卡片。 | [OpenMausBot docs/approval-levels.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/approval-levels.md)；[OpenMausBot README「Bots ask before they act」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features) | 节点 4 | 是 | [x] |
| 选项卡，一点就能回复 | 需要用户拍板时发带选项的卡片，可点选项，也可自己写一句；不必为了选一项去打长文。同属节点 4 里「需要人决定」的权限交互，展示会落在节点 6 的消息流。 | **Grok Bot 用户能看到的行为**；[OpenMausBot 问题卡文档与截图](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/approval-levels.md#asking-you-questions)；[OpenBot：Bot 可停下来问人](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/architecture.md#asking-a-person) | 节点 4、节点 6 | 是（新增一种卡片 `choice`，带 `options[]`，用户的选择以 `{"choice":[ids]}` 回传，单选也传数组；状态卡不动） | [x] |
| 对外邮件和消息先出草稿卡 | 要发出去的邮件或消息先以草稿卡呈现，用户自己点发送。与审批卡同属节点 4 权限：对外发送必须先出卡片，点了才执行。 | **Grok Bot 用户能看到的行为**；[Comma 产品功能：草稿只经用户动作发出](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md#chat-and-tasks) | 节点 4 | 否（2026-11-20 由 PM 改到第二期，和频道、连接器一起做；第一期 bro 还不能往外发消息。客户端的草稿卡按钮只在 `--fake` 模式下显示） | [ ] |
| 外部内容只当数据 | 网页、邮件、工具返回的内容只当数据处理，里面写的指令一律不执行，也不能凭这些内容自行授权操作。 | **Grok Bot 用户能看到的行为**；[Comma docs/verification.md：Data, forwarded text, and untrusted pages do not authorize a request](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/verification.md#ifc-authorization-contract) | 节点 4 | 是（2026-11-20 PM 定入第一期；原备注：权限契约尚未写明这条，应对齐） | [x] |

## 重启续跑与防重放

| 功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 中断后从原工作区接着跑 | 进程或容器没了，任务身份还在；恢复后挂回原来的共享卷，历史文件还在。 | [Comma README：Accepted work is not silently lost](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#built-with-elixir-and-lean-verified-with-lean-and-tla)；robroz `POST /api/tasks/:id/resume` | 节点 4 | 是 | [x] |
| 被打断的修改不自动再跑一遍 | 已经做过或已被打断的改动操作，重启后不会自行重放。拒绝或过期的审批也不能自动重试；过期提案要重新出卡。 | [Comma README：An interrupted mutating action does not run again on its own](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#built-with-elixir-and-lean-verified-with-lean-and-tla)；**Grok Bot 用户能看到的行为**；[OpenMausBot 过期日程卡需重新提案](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/screenshots/moca-282-expired-card-ios.jpg) | 节点 4 | 是（2026-11-20 PM 定入第一期；原备注：第一期已跳过已接管的阻塞行；一般写操作的防重放尚未写成契约） | [x] |

## 记忆

| 功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 长期记忆按类型存放 | 用户资料、工作习惯、项目记录分开存，可查看、可改、可忘掉。Bot 形成的条目要用户确认后才收下。 | **Grok Bot 用户能看到的行为**；[OpenBot `/capabilities/memory`](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#main-surfaces)；[OpenMausBot docs/memory.md：Memory 面板、About me、日记](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/memory.md)；[Comma Drive 中的 memory / instructions](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md#files-and-navigation) | 后续（第一期范围外） | 否 | [ ] |
| 项目日志 | 按日或按项目留下工作记录，人可以打开看、可以撤销改动。 | **Grok Bot 用户能看到的行为**；[OpenMausBot：每日日志与变更日记](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/memory.md) | 后续（第一期范围外） | 否 | [ ] |

## 实时推送 / 流式

| 功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 同一张状态卡原地更新 | 一个任务一张卡，状态变化就地改，不另起一张。 | robroz 卡片规范；[Comma：Cards only project status](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/salix/tasks-background-execution.md#canonical-lifecycle) | 节点 6 | 是 | [x] |
| 流式回复与活动提示 | 回复边生成边出现；工具或步骤用活动提示标出。客户端从一条事件流收更新。 | [OpenMausBot README「Also in the box」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[OpenMausBot README「How it works」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#how-it-works)；[OpenBot：看屏幕与 Activity 列表](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features) | 节点 6 | 是（最初用 `GET /api/groups/:id/events` 轮询；2026-11-20 起新增事件流接口做实时推送，轮询留作兜底，见「IM 基础」）；WebSocket 桌面流为否 | [x] |

## 多 agent 与群聊

| 功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 建群、拉多个 Bot、指定执行者 | 一个群里可以有多个人和一个或多个 Bot；派任务时指定执行的 Bot。 | robroz 节点 2；[OpenBot：To 里放两个以上 Bot 即开群](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#main-surfaces)；[OpenMausBot 频道与可编辑 roster](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features) | 节点 2 | 是 | [x] |
| 群聊里多个 Bot 分工 | 被点到的 Bot 用自己的名字回答，不由另一个 Bot 代述。可把工作交给队友，按接收方自己的权限跑。 | **Grok Bot 用户能看到的行为**；[OpenBot docs/architecture.md「One Bot handing work to another」](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/architecture.md#one-bot-handing-work-to-another)；[OpenMausBot 审批文档中的队友交接](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/approval-levels.md) | 节点 2 | 否（第一期只做建群、拉 Bot、派任务；Bot 互派和自动分工为后续） | [ ] |
| 本地 agent 按 bot 接入 | 侧栏里每个 bot 是一台本地 agent（本机已登录的 Claude / Codex / Grok 命令行，或设置里接入的兼容引擎）。本机 127.0.0.1 上的 harness 托管这些进程；应用发命令、收一条事件流。可在对话中途换模型。数据和密钥默认留在本机目录。 | [OpenMausBot README「Why」「Pick a brain per bot」「How it works」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#why)；[OpenMausBot docs/custom-engines.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/custom-engines.md) | 节点 3 | 否（第一期执行器仍是本机 Docker 里的 Rust runner，不接外部 CLI） | [ ] |
| 把 Bot 当联系人管理 | 侧栏像聊天列表：置顶、未读、改资料、复制、隐藏、删除；可暂停某个 bot、整个部门或全部 bot，恢复后数据和例行任务还在。 | [OpenMausBot README「Manage bots like chats」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[OpenMausBot docs/pausing-bots.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/pausing-bots.md) | 节点 2、节点 6（第一期只有群列表，没有 Bot 详情页和暂停） | 否（其中未读数和私聊 2026-11-20 起进第一期，见「IM 基础」） | [ ] |
| 频道 / 房间组织 | 工作、日常、项目分频道，各有自己的记录、共用说明、工作目录、应答规则和可编辑的 bot 名单。未点名的消息可由决策模型选一个合适的 bot，拿不准就交给房间负责人。 | [OpenMausBot README「Channels」「Fast decisions with Jev」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[OpenMausBot docs/decision-model.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/decision-model.md) | 节点 2（群） | 否（第二期；第一期仍是每个群一个 Docker。原备注：分群是；自动选 Bot 和频道级说明为否） | [ ] |
| 整队导入与分享 | 用一份说明文件安装一组 bot、频道和建议例行任务；连接默认关闭，例行任务默认暂停，不携带会话、记忆、密钥或电脑权限。分享时只带角色、技能和要用的应用名。 | [OpenMausBot README「Install a complete team」「Share a bot or a department」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features) | 节点 2 | 否 | [ ] |

## 定时与事件触发

| 功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 定时任务，到点出简报 | 宿主机按计划拉起任务，写下产物，发一张标了「简报」的交付卡。睡着的容器不能自己醒来。错过的窗口不补跑。 | robroz 节点 5；**Grok Bot 用户能看到的行为**；[Comma README：Routines 在设定时间给简报](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does)；[OpenBot docs/routines.md](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/routines.md) | 节点 5、节点 6 | 是 | [x] |
| 事件触发 | 收件箱、仓库或已连接应用有约定变化时才叫醒 bot；安静时段不跑大模型。 | **Grok Bot 用户能看到的行为**；[Comma README：Loops](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does)；[OpenMausBot README「Triggers: schedules and webhooks」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#triggers-schedules-and-webhooks) | 节点 5 | 否（后续。Webhook 也不在第一期） | [ ] |
| 没变化时不打扰 | 没有新情况就不推送、不刷存在感。例行任务连续失败应停下并说明，而不是反复打扰。 | **Grok Bot 用户能看到的行为**；[Comma README：A quiet hour uses no large-model tokens](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does)；[OpenBot：疲劳规则与错过窗口跳过](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/routines.md)；[OpenMausBot：空简报不发出](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/proactive-routines.md#what-the-person-sees) | 节点 5（错过不补跑） | 是（2026-11-20 PM 定入第一期；原备注：完整「无变化不推送」还需产品规则） | [x] |
| 例行任务列表，可看可暂停 | 单独的列表里能看到所有定时和触发项、运行历史，并能暂停、恢复、立刻跑一次。对话里创建时出一张可编辑 / 暂停的卡片；过期或已变更的提案要重新出卡，不能再点旧卡。第一期已有的是节点 5 的 cron 与简报卡。 | **Grok Bot 用户能看到的行为**；[OpenBot `/routines`](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#main-surfaces)；[OpenMausBot README 与 Tasks & routines 截图](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#triggers-schedules-and-webhooks) | 节点 5 | 是（第一期只做列表和暂停：`GET` 列表、`PATCH paused`；运行历史、立刻跑一次、编辑放到后续） | [x] |

## 工具 / 连接器

| 功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 连接器 | 把常用应用接到 bot。OAuth 由用户完成；授权范围按 bot 授予。未授权的应用不能用。MCP 网关不在第一期。 | **Grok Bot 用户能看到的行为**；[OpenBot `/connections` 与 MCP 目录](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#main-surfaces)；[OpenMausBot README「Connected apps」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[Comma 插件与连接请求卡](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/tools-integrations.md#request-cards) | 后续（第一期范围外） | 否 | [ ] |
| 技能库 | 可复用的技能说明，对话里启用；保存或授予前要用户确认。技能本身不等于对外系统的权限。 | **Grok Bot 用户能看到的行为**；[OpenBot README：Skills are instructions, not capabilities](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features)；[Comma Drive 中的 skills](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md#files-and-navigation) | 后续（第一期范围外） | 否 | [ ] |
| 公开网页检索并注明出处 | 搜索或阅读公开网页，并引用 URL。检索查询按用户可见的「搜索网页 / 阅读页面」呈现。第一期只有节点 4 按任务计数的出网 GET，不做检索产品。 | [OpenBot docs/web-research.md](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/web-research.md) | 后续（第一期范围外） | 否 | [ ] |

## 沙箱电脑与浏览器

| 功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 本机隔离沙箱，做完休眠 | 每个任务在隔离环境里跑，做完删掉容器，群共享目录里的文件还在，交付卡上给出相对路径。 | robroz 节点 3；[Comma coding sandboxes](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/compute-devices.md#coding-sandboxes)；[OpenBot：每人/每 Bot 一台电脑](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features) | 节点 3 | 是（本机 Docker，不是微型虚拟机或第三方沙箱云） | [x] |
| 按 bot 分配电脑 | 每个 bot 可单独选择运行处：云桌面、本机隔离的 Local VM、这台电脑，或关闭。电脑面板显示该 bot 的屏幕；云电脑未配置时说明如何补上密钥。选「这台电脑」需明确开启，并受平台限制。 | [OpenMausBot README「Every bot gets a computer」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[Computer 面板截图](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/screenshots/docs-computer-panel.png)；[docs/computer-use-integration.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/computer-use-integration.md) | 节点 3 | 否（第一期仍是每个群一个 Docker 加共享卷；按 bot 单独虚拟机为后续） | [ ] |
| 云端电脑和浏览器代办 | 看它的桌面或浏览器，必要时由用户接管（登录、验证码）。第一期不做桌面流和浏览器自动化。 | **Grok Bot 用户能看到的行为**；[OpenBot README：Watch / Take the wheel](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features)；[OpenMausBot：实时预览与「Open desktop」接管](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[Comma：Agent browser / Salix](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does) | 节点 4（私聊接管，无屏幕） | 否（浏览器、Playwright、桌面流均不在第一期） | [ ] |
| 文件树、终端、桌面流 | 在卡片或侧栏里浏览文件树、打开终端、看实时桌面。 | [OpenBot：屏幕与 Activity](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features) | 明确不在本期 | 否 | [ ] |

## 产出页面

| 功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 交付卡片 | 摘要加一条不能点开的等宽相对路径。定时简报仍是同一张交付卡，角上标「简报」。 | robroz 卡片规范 | 节点 6 | 是 | [x] |
| 可发布的页面或小应用 | 用户让 agent 做一个可打开的预览页，确认后再发布；对外行动仍要在产品界面里点一下。 | [Comma docs/product-features.md Mini-apps](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md#mini-apps) | 后续（第一期范围外） | 否 | [ ] |
| 对话里的通用组件 | 除状态卡和交付卡之外，用可交互组件代替纯文字（例如画廊、选项板）。 | [OpenBot README：Components instead of prose](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features)；[Comma Chat dynamic UI](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md#chat-dynamic-ui) | 节点 6 | 否（第一期只做状态卡和交付卡） | [ ] |

## 部署与自托管

| 功能 | 行为描述 | 来源 | robroz 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 单机私有化，模型自备 | 在自己的机器上跑完整产品，不依赖对方账号；模型用自己的密钥或本机已有登录。 | robroz 第一期范围；[Comma README：Self-host with Docker Compose](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#self-host-with-docker-compose)；[OpenBot Personal Edition](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/personal-edition.md)；[OpenMausBot README：本机优先，桌面包内嵌 harness](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#why) | 节点 7 | 是 | [x] |
| 用量可见、可设上限 | 能看到用量；可为整台机器或单个 bot 设每日 / 每月上限。到上限默认只拦住无人值守的例行任务，本人发的消息仍可走。80% 只改变用量条颜色，不弹窗。 | [OpenMausBot docs/usage-caps.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/usage-caps.md)（开源版即有；许可制月度支出上限属 enterprise，不参考） | 后续（第一期范围外） | 否 | [ ] |

## 界面（前端补充）

以下由 @吊炸天前端专家 根据 Comma、OpenBot、OpenMausBot 的 README、文档和公开截图整理，节点对照和「未说明」标记保持原样。robroz 的卡片样式仍按已定规范做，不照搬对方视觉。

功能表里的来源版本见文首。界面笔记里 OpenBot、OpenMausBot 的部分条目当时读的是当前 `main` 或产品文档站，已在出处中标明，不当成 pin 里写死的内容。

**Comma 的缺口（缺失（未参考源码））**：没有官方的「任务列表 / 执行中 / 等审批 / 重启后恢复」四态产品截图；任务看板和 dynamic UI 的完整卡片字段只写在源码里，本文未打开源码，故标「未说明」，不以猜测补全。

### OpenMausBot 公开预览卡对照

固定版本 `ecabd15` 根目录的 `*-preview.html` 是公开设计预览入口（页面本身只挂了预览标题，可见字段来自 `docs/screenshots/` 与文档，没有去读应用源码）：

| 预览 | 已看见的字段与行为 | 对照 robroz |
| --- | --- | --- |
| 问题卡（`question-cards-preview.html`，README 审批截图、`docs/question-card-cutoff.png`） | 标题（如「你最想先得到哪类帮助？」「Your bot has a question」）；一段说明或正文；带字母编号的选项；「自己写答案」输入框；关闭。进行中可把回答送进当前这一轮。任何审批级别都不会代答问题。 | 第一期没有选项卡。若后续要做，需先扩充卡片契约。 |
| 审批卡（README「Bots ask before they act」，文档「Answering a request」） | 内联卡，针对命令、改文件等：允许一次、本次会话允许；对可信只读工具可长期允许（可在该 bot 的访问设置里撤销）；拒绝。同一房间的其他 bot 仍会再问。 | 第一期用「卡住 + 去私聊接管」，群里不出审批卡。 |
| 电脑询问 / 电脑面板（`computer-ask-preview.html` 标题为「Turn On Your Computer」；`docs/screenshots/docs-computer-panel.png`） | 面板标题带该 bot 的名字和「屏幕」；未配置云电脑时的说明；可选的云电脑密钥；「运行于」四选一：Cloud、Local VM、This computer、Off。 | 第一期无电脑面板和桌面流。 |
| 房间卡（`room-card-preview.html`） | 预览页存在。频道在 README 里有各自的记录、共用说明、工作目录、应答规则和 bot 名单，但房间卡上具体排了哪些字段待核。 | 节点 6 目前是群列表 + 消息流。 |
| 用量表（`usage-meter-preview.html`，`docs/usage-caps.md`「What a person sees」） | 设置里的用量条；接近上限变提示色，达上限变强调色并写明何时恢复；不弹审批卡。 | 后续（第一期范围外）。 |
| 日程 / 例行任务卡（Tasks & routines 截图、过期卡截图） | 列表页含 Routines / Webhooks 分页、日历、筛选、「新建」；说明一次任务与可复用日程的区别。对话里的日程提案卡有标题、将要创建的例行任务说明；若提案之后例行任务已变，卡片标过期，要求重新提案。 | 第一期只有定时简报交付卡，没有例行任务列表。 |
| 其他预览（欢迎流程、主页、电话连接、取消方案、云购买、吉祥物动作） | 仅确认有这些预览页，字段待核。取消方案、云购买若只服务于商业订购，补录时排除 enterprise 范围。 | — |

### 清洁室、三家页面与卡片笔记

本文只整理**用户能看到的页面、卡片字段、状态流转和交互**，交给最详尽开发文档哥并入 `docs/reference-features.md`。robroz 的卡片样式仍按已冻结的卡片规范（hive-2）做，**不照抄对方视觉**。

## 清洁室与来源边界

- **允许**：各项目的 README、文档站点、仓库内 `docs/`（含用户手册、工程契约中的产品说明）、LICENSE / 仓库许可元数据、公开截图与演示图。
- **宽松许可（MIT、Apache-2.0、BSD、ISC 等）可以读源码**（2026-11-26 起）：可以打开源码文件，学它是怎么实现的。不能整段搬，也不能逐行翻译。确实要借一小段时，保留原版权头，Apache-2.0 的还要带上 NOTICE，并在 PR 正文里单独写明来源、许可证和行号。
- **GPL、AGPL、其他 copyleft 许可或许可证不明确的项目，禁止读源码**：不打开、不阅读、不摘录源码文件（如 `.ex`、`.exs`、`.tsx`、`.ts`、`.jsx`、`.js`、`.rs`、`.css` 等）。文档里提到源码路径的，只当出处看，不点进去。许可证以仓库根目录的 LICENSE 为准，判断不了的按不明确处理。
- 本文 2026-11-26 之前写的条目都没有打开过任何一方的源码。
- **未写进文档的字段或交互标「未说明」**，不靠猜。宽松许可的项目可以从源码核实后补上，并注明看的是哪个文件、哪个版本；copyleft 或许可证不明确的项目不能用源码补。
- **robroz 节点列**对照：`hive`（第一期范围）、`hive-2`（卡片规则）、`hive-3`（接口契约）、`hive-4`（权限、恢复、定时简报）、`hive-5`（验收顺序，尤其第 6 节客户端）。对不上的标「待定」；第一期明确不做的标「不在第一期」。

| 节点 | 用来对照的要点 |
| --- | --- |
| hive | 第一期：单机私有化；说话、建群、拉 Bot、隔离运行、做完休眠；一次定时简报、一次私聊接管。客户端是 iced 原生包，不套网页。不做文件树、终端、桌面流、Bot 详情页等。 |
| hive-2 | 卡片在消息流里就地改状态；左 Bot 名、右一行状态；排队灰、进行深、完成绿、卡住琥珀；卡住只留「去私聊接管」；交付卡片是摘要加相对路径（等宽、不能点开）；定时简报仍是同一张交付卡片，角上标「简报」。 |
| hive-3 | 任务状态只有 `queued`、`running`、`blocked`、`done`；事件流是 `GET /api/groups/:id/events`；恢复是 `POST /api/tasks/:id/resume`。 |
| hive-4 | 工作区读写默认放行；出网只有 GET 且按任务计数；写外部资产或验证码则 `blocked`，删容器，人到私聊接管；定时在宿主机上跑，到点发简报卡片。 |
| hive-5 | 第 6 节才做客户端：会话列表、消息流、状态卡片和交付卡片；这一节不做文件树、终端和桌面流。 |

---

## 1. Comma（AFK-surf/Comma）

- 仓库：[https://github.com/AFK-surf/Comma](https://github.com/AFK-surf/Comma)
- 对照版本固定在 `66ab3dc92475005aea9afd123df36a28e4ef86ab`（2026-10-27「Sync Comma source」）。仓库内 README / `docs/` 均按该 SHA 的 raw 读取。
- 产品文档站 [https://comma.surf/docs](https://comma.surf/docs) 可能新于该 pin；下文凡引用文档站，都会标明，不当成 pin 里已写死的内容。
- 同仓库还有一份《Bridge For Teams 用户手册》（`docs/user_manual/bridge_for_teams/manual_zh.tex`），面向的是 [teams.bridge.surf](https://teams.bridge.surf) 的 Dashboard，**不是**个人版 Comma 应用。单独放在本节附录，避免和 Comma Web / Mac 界面混在一起。

### 1.a 页面 / 屏幕

| 页面或入口 | 文档里怎么说 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- |
| 客户端形态：macOS 应用、Web（`app.comma.surf`）、iPhone / Android 应用 | 下载 Mac 或打开 Web；通话录音和 Drive 需要 Mac 应用；手机有 iPhone 或 Android 应用。 | [Quickstart](https://comma.surf/docs/quickstart)、[README @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md) | 待定（第一期是 iced 桌面包，不是对方的多端应用） |
| Home / 聊天 | 在 Home 里像对同事一样写下需求。简单问题在聊天里回答；更大的工作变成 Task。 | [Quickstart](https://comma.surf/docs/quickstart)、[Agent](https://comma.surf/docs/agent) | hive、hive-5 第 6 节（会话列表与消息流） |
| Tasks 看板 | 「所有工作在一块板上」：任务在 In progress、Needs Review、Done 列之间移动，Comma 自己挪卡片。 | [README @ 66ab3dc「What Comma does」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md)、[Tasks](https://comma.surf/docs/agent/tasks) | hive-2、hive-3、hive-5 第 6 节（状态卡片） |
| Task 详情 | 打开一个 Task，阅读其对话以及 Comma 交付了什么。 | [Tasks](https://comma.surf/docs/agent/tasks) | hive-2、hive-5 |
| Settings → Channels | 在 Settings 打开 Channels，在 Linear 卡片上按 Connect。卡上会显示 Connected、组织与帐号。Manage 可重连、断开。 | [Linear](https://comma.surf/docs/agent/linear) | 待定 |
| Settings → General | 含 AirDrop 显示名、System prompt / Custom instructions（可 Reset）。 | [clients.md @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md)、[product-features.md @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md) | 待定（配置与运行分开，见 hive） |
| Settings → Keyboard shortcuts / App | 可改 Quick Chat 快捷键；Settings > App 默认为开，关掉则停止助手并释放快捷键。 | [clients.md @ 66ab3dc「Quick Chat」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | 待定 |
| Settings → Apps | 撤销授权、移除 Installs。 | [product-features.md @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md) | 待定 |
| 登录后的模型 / Settings | 可在登录后于 Settings 配置私有模型；Router 与 Worker 选择器相互独立；新 Workspace 没有 Worker；Owner 可把 Router 改名（1–40 字符）。 | [README 自托管](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md)、[Install](https://comma.surf/docs/self-host/install)、[product-features.md @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md) | 待定 |
| Drive / System Files | Drive 的 System Files 向 owner 展示 Router 的 `/memory`、`/imports`、`/Comma`、skills 与 instructions；仅部分路径可编辑。 | [product-features.md @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md) | 记忆：待定；文件树：不在第一期 |
| Apps / Plugins 页 | Apps 页布局与 Plugins 页相同：大标题、搜索、New app、画廊或列表。 | [clients.md @ 66ab3dc「Mini-app runtime」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | 不在第一期 |
| Comma Admin、Salix dashboard、本地收件箱 | 自托管会起 Web、Admin、Product API、Salix dashboard、Mailpit 本地收件箱。界面字段 **未说明**。 | [README @ 66ab3dc「Interfaces」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md) | 待定 |
| Apple Watch | 可访问 Home / Tasks / messages / review；手机撤销会撤销 Watch。 | [clients.md @ 66ab3dc「Native Apple clients」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | 待定 |
| iPhone Task sheet / iPad 分栏 | iPhone 的 Task sheet 仍保留 Home；iPad 为分栏。 | 同上 | 待定 |
| 嵌入式 Task 面板 | Web `/task-panel.html`；Telegram `/tasks`；微信「任务列表」需 Comma 登录。一页加载 20 个 Task。详情打开 Comma Web。 | [clients.md @ 66ab3dc「Embedded Task panel」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | 待定 |
| macOS / Windows Quick Chat（Notch / 托盘） | 双击修饰键或菜单栏打开；发送成功后可点进对应 Workspace 的 Home；进行中的 Task 会占据该行，点击打开该 Task。 | [clients.md @ 66ab3dc「Quick Chat」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | 待定 |
| SSH 终端客户端 | 一个 workspace 直接进聊天，多个则出选择器。命令含 `/help`、`/workspace`、`/keys`、`/quit` 等；`/stop` 报告不支持取消。附件会引导回 Comma。 | [clients.md @ 66ab3dc「SSH terminal client」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | 待定 |
| 独立的 Memory 页、独立的审批队列页 | 产品文档站与 README **未说明**有单独的 Memory 或审批中心页。记忆出现在 Drive System Files 的 `/memory`。审批体现在看板的 Needs Review 与聊天提示。 | [Tasks](https://comma.surf/docs/agent/tasks)、[product-features.md](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md) | — |

### 1.b 卡片上能看到的字段

**缺失（未参考源码）**：看板卡片的完整字段，以及 dynamic UI 的卡片种类和字段结构，只写在源码或 card contract 源文件里。本文未打开这些源码，故种类与字段清单保持「未说明」，不以猜测或拆源码补全。

| 卡片类型 | 文档写明的可见内容 | 未说明 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- | --- |
| 任务看板卡片 | 任务出现在 In progress / Needs Review / Done 列。卡片由客户端投影服务端的规范状态。看板卡片上还有哪些字段（标题、负责人、时间等）**未说明**。 | 字段清单 | [Tasks](https://comma.surf/docs/agent/tasks)、[tasks-background-execution.md @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/salix/tasks-background-execution.md) | hive-2、hive-3 |
| Task 详情 / 交付 | 打开后可看「对话」和「Comma 交付了什么」。在 Needs Review 时，满意后在详情里选 **Done**。 | 交付物是摘要、路径还是预览，**未说明** | [Tasks](https://comma.surf/docs/agent/tasks) | hive-2 交付卡片（robroz 只要摘要 + 相对路径，不能点开） |
| Worker 提问卡 | Worker 的 question card 会到达 Router；任务进入 `escalated`，客户端显示为 **Needs input**；人回答后恢复 `active`。卡片上的选项字段 **未说明**。 | 问题文案/选项 | [tasks-background-execution.md @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/salix/tasks-background-execution.md) | hive-2 卡住 +「去私聊接管」、hive-4 |
| 文件卡 | 保留 preview 与 Open in；Download 在预览侧栏，不是卡片上的独立按钮；附件可 Add to Context。 | 其他元数据 | [product-features.md @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md) | 待定（交付路径按 hive-2 用等宽、不可点） |
| 聊天跟进胶囊 | 完整聊天最多 4 条排序跟进；胶囊发送不消耗草稿；最佳项同时是占位符，Tab 可采纳但不发送。 | — | 同上「Chat and Tasks」 | 待定 |
| 结构化结果卡（dynamic UI） | 结构化结果默认用卡片呈现；模板可渲染文本、HTTPS 链接、有界列表、可读条目；过长文本在卡内以省略号截断。具体「卡片种类」和字段 schema 写在 Worker 提示与 Help，以及一份 card contract 源文件中——**源文件未打开，种类与字段清单标未说明**。 | 种类与字段 | [clients.md @ 66ab3dc「Chat dynamic UI」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | 待定 |
| Mini-app Task 卡 | 启动封面：手机宽度首屏缩小图、名称、版本；应用不可用时变成灰色「App unavailable」并说明原因。 | — | [clients.md @ 66ab3dc「Mini-app runtime」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | 不在第一期 |
| QQ 任务卡通知 | 对带 QQ 卡的任务，watch 会向该卡投递 working、blocked、paused 通知。 | 卡片其余字段 | [tasks-background-execution.md @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/salix/tasks-background-execution.md) | 待定 |
| Telegram review 卡 | 仅通过 owner review 完成任务；客户端超时不能完成/取消/重开任务。卡上按钮文案 **未说明**。 | 按钮文案 | 同上 | hive-2、hive-4 |
| 设备卡 | 先显示 agents 与 access，再显示 sandbox。维护失败并重装后，「the card then says so」。 | 其余字段 | [product-features.md @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md) | 不在第一期（桌面流 / 设备） |
| 授权 / grant sheet | grant sheet 会点名后端 egress 主机。 | 完整字段 | 同上 | 待定 |
| Linear 里的进度与结果 | 会话里会出现「Task in Comma」链接；Worker 的进度和结果会发回 Linear；PR 也会被链上。在 Comma 里可看完整 transcript、继续工作、或 review and accept the result。 | Comma 侧 accept 按钮的完整字段 | [Linear](https://comma.surf/docs/agent/linear) | hive-2 交付 / 审批对照 |

公开产品文档**没有**单独画出一张「审批卡字段表」。用户可见的审批，主要是看板的 Needs Review + 详情里选 Done，以及工程契约里的 Needs input / question card。不要把未写明的按钮猜成 OpenBot 那样的 Allow once / Deny。

### 1.c 用户能看到的任务状态与流转

**缺失（未参考源码）**：没有官方的「任务列表 / 执行中 / 等审批 / 重启后恢复」四态产品截图可引用（Quickstart 写明逐步截图尚未发布）。下列状态只来自 README 与文档用词，不以源码或未发布截图补全。

公开产品文档里的看板状态（用户用词）：

| 状态 | 含义 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- |
| In progress | Comma 正在做。 | [Tasks](https://comma.surf/docs/agent/tasks) | hive-3 `running`、hive-2「进行」 |
| Needs Review | 等你做决定。决策属于你时，任务在此等待；也可让 Comma 只起草。 | [Tasks](https://comma.surf/docs/agent/tasks)、[Agent](https://comma.surf/docs/agent)、[README](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md) | 对照 hive-3 `blocked` / hive-2「卡住」，但对方是「等你拍板」，不一定等于验证码接管 |
| Done | 已交付。Comma **自己**把任务标成 Done。人也可以在 Needs Review 的详情里选 Done。 | [Tasks](https://comma.surf/docs/agent/tasks) | hive-3 `done`、hive-2「完成」 |

工程契约（pin `66ab3dc`）里还有另一套用词，**和产品文档不完全一致**，对照时不要混用：

| 契约用词 / 客户端展示 | 含义 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- |
| `escalated` → 客户端显示 **Needs input** | 「升级」表示被堵住的工作需要发起人，而不是例行审批。人回答期间为 `escalated`，恢复工作后为 `active`。 | [tasks-background-execution.md @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/salix/tasks-background-execution.md) | hive-3 `blocked`、hive-4 接管 |
| `active` | 工作恢复或正在进行。 | 同上 | hive-3 `running` |
| Messages never reopen Tasks | 发消息不会重开已结束的任务。失败写入要如实报告；抑制同状态重试；重开要解释。 | 同上 | 待定 |
| Archive / restore | 归档只读，保留消息、历史和资源；`archived_from_status` 与 `archived_at` 用于恢复；恢复不会重新绑定 Schedule。本地卡片不等于归档状态。 | 同上「Archive」 | 待定 |
| 非 `active` 的任务会跳过定时窗口，但保留 Schedule；只有已结束任务或未绑定的 Schedule 才会删掉它 | 定时与任务状态的关系。 | 同上「Continuation and containment」 | hive-4 定时 |
| Station 内部 stage（`vm_not_ready`、`station_offline`、`running`、`idle_no_reply` 等） | 主要是运行时监视，文档未说明这些名字是否直接画在任务卡上。 | 同上「Station Tasks」 | 未说明（界面是否展示） |

产品文档站的「Needs Review」与契约里的「Needs input」是否为同一界面文案，**未说明**，清单里应并列引用，不要擅自合并。

robroz 的 `queued` 在 Comma 公开文档里没有对等列名，**未说明**。

### 1.d 交互

| 交互 | 文档怎么说 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- |
| 创建任务 | 在 Home / 聊天里提出请求。简单问题就地回答；更大的工作变成 Task，Comma 自己检查并决定何时完成。 | [Quickstart](https://comma.surf/docs/quickstart)、[Agent](https://comma.surf/docs/agent)、[README](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md) | hive、hive-3 `POST .../tasks` |
| 从 Linear / 聊天应用发任务 | 把 Linear issue 指派给 Comma 或在评论里 @Comma；需要实做时才开 Task。也可从 Signal / Telegram 交接；Telegram 可保留多份任务列表。 | [Linear](https://comma.surf/docs/agent/linear)、[Chat apps](https://comma.surf/docs/agent/chat-apps) | 待定 |
| 需要你拍板 | 任务进入 Needs Review；Comma 也会在聊天里先写给你并建议下一步。Routines 在你设定的时间给简报。 | [Agent](https://comma.surf/docs/agent)、[Routines](https://comma.surf/docs/agent/routines)、[README](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md) | hive-2、hive-4 |
| 批准 / 拒绝 | 在 Needs Review 详情里选 Done。工程契约另有 Needs input + question card，人回答后恢复 active。Telegram review 卡只能由 owner review 完成。**拒绝按钮、超时策略的界面文案未说明**。 | [Tasks](https://comma.surf/docs/agent/tasks)、[tasks-background-execution.md](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/salix/tasks-background-execution.md) | hive-4；robroz 卡住只留「去私聊接管」，不要照抄「详情里点 Done」的交互 |
| 只起草 | 若你更希望，可让 Comma 只 draft。如何在界面里打开该偏好 **未说明**。 | [Agent](https://comma.surf/docs/agent) | 待定 |
| Linear 里停止 | 在 session 按 Stop：进行中的 Task 结束，已做的工作保留。把 issue 标 Done / Canceled 或改派走，也会结束进行中的 Task。 | [Linear](https://comma.surf/docs/agent/linear) | 待定 |
| 本机操作许可 | 连接 Mac 后默认只读；打开 Allow operations 后才能改文件、跑命令、把工作交给 Codex / Claude Code 等。可随时撤销设备。 | [README](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md)、[Devices](https://comma.surf/docs/salix/devices) | hive-4（写外部资产先拦住）；本机桌面控制不在第一期 |
| 重启 / 续跑（运行时承诺，界面展示未写清） | 某一部分失败会重启，其他 agent 继续。Salix 已接受的消息在重启后仍在。**「中断的、会改东西的动作不会自己再跑一遍」**。刷新会恢复该对话的规范历史。Home 最多跟随 2 条 Task feed 显示 Worker 状态。任务视图会立刻显示已知属性。 | [README「Built with Elixir and Lean」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md)、[clients.md](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | hive-3 / hive-4 的 resume 与防重放；**「重启后恢复」在界面上长什么样未说明** |
| Loop 的 resume | 对 Loop：显式 resume 再给 15 分钟；进程重启不会给；Pause 保留待办，删除会丢弃。这是 Loop 契约，**未说明**是否画成任务卡上的 Resume 按钮。 | [tasks-background-execution.md「Background loops」](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/salix/tasks-background-execution.md) | 待定 |
| 实时更新 | Apple 客户端前台 SSE（最多 2 路）；后台关闭。可见 Home 跟随 Task feed。动态卡片在消息挂载时读取，实时线程会提前加载。 | [clients.md](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | hive-3 事件流 |
| 定时简报（Routines） | 在设定时间列出各应用里需要你的事项；点一项会把提示放进输入框，待发送。需要你「现在」处理时，会在聊天里先写并建议下一步。 | [Routines](https://comma.surf/docs/agent/routines)（该页仍是占位提纲） | hive、hive-2 简报角标、hive-4、hive-5 第 5 节 |
| 回复消息 | 菜单，或 iOS 右滑，收集该消息的 thread。 | [product-features.md](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md) | 待定 |
| 通知 | Review / escalation / completion / failure 警报**省略**标题、内容、Workspace、身份；可退出，退出后仍保留 Activity 标题。 | [clients.md](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | 待定 |
| 移动端 | 有 iPhone、iPad、Watch、Android。文档说明了 Home / Tasks / review、Task sheet、附件先下载再打开。**未说明**手机上审批按钮的完整布局，也没有官方逐步截图（Quickstart 写 screenshots coming soon）。 | [Quickstart](https://comma.surf/docs/quickstart)、[clients.md](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md) | 待定 |
| Android 0.2.0 (311) 发布说明里的新聊天卡、凭据填写卡 | 源 SHA 是 `faf55ed`，**晚于**本笔记锁定的 `66ab3dc`，不作为 pin 版本的界面依据。 | [Releases](https://github.com/AFK-surf/Comma/releases) | — |

### 1.e 公开截图与演示

**缺失（未参考源码）**：该 pin 下没有官方四态产品截图。仓库在该版本只找到 README 横幅图；演示视频未按帧拆解。

| 资源 | 说明 |
| --- | --- |
| [README banner @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/.github/assets/readme-banner.webp) | 仓库在该 pin 下仅找到这一张 README 资源图。 |
| [演示视频 1](https://github.com/user-attachments/assets/cefa2954-f29c-4434-9246-a82cb19a51fe)、[演示视频 2](https://github.com/user-attachments/assets/486cde78-d927-4ff0-85de-00c46edc498e) | README「See Comma in action」。本文**没有**按帧拆视频来补字段，避免把未写进文档的 UI 写成事实。 |
| [Quickstart](https://comma.surf/docs/quickstart) | 写明逐步截图即将提供，当前没有。 |
| 文档站各页的 `og-image` | 营销图，不是产品状态截图。 |

### 1.f 附录：Bridge For Teams Dashboard（同仓库手册，另一产品）

出处：[manual_zh.tex @ 66ab3dc](https://raw.githubusercontent.com/AFK-surf/Comma/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/user_manual/bridge_for_teams/manual_zh.tex)（文档版本 1.5，2026-07-19，适用 [teams.bridge.surf](https://teams.bridge.surf)）。

| 项目 | 手册内容 | 对应 robroz 节点 |
| --- | --- | --- |
| 导航 | 左侧：组织切换器；Workspace（Overview、Agent Swarms、Fin、按角色显示的 Health）；当前 Agent Swarm（Overview、Tasks、Agents、Devices、Settings）；Configure（Plugins、Skills、Connections）；Administration（Members、Organization plugins、Settings）。窄屏把同一组导航放进抽屉。 | 待定（这是团队 Dashboard，不是 robroz 第一期客户端） |
| 创建任务 | Tasks → New task，填写目标与必要背景；Run on（Salix / Cloud VM / 已连接设备）；Model。提交后打开任务详情看进展；需要补充信息时在详情里继续发消息。 | hive-3 派任务；详情续聊对照 hive-5 消息流 |
| 计划 | 在任务详情中添加和编辑。Tasks 右上角筛选选 Scheduled，可看全部定时计划并删除。 | hive-4 定时 |
| Agent 详情 | 右侧打开：角色、状态、模型、运行时 ID、系统提示词。Configure 选模型。Start new session 会停止当前工作，原会话历史仍可查看。 | 不在第一期（Bot 详情页） |
| Fin / Runner 列表字段 | 状态、容量、主机信息、版本、最近在线时间。 | 不在第一期 |
| 「任务没有继续执行」 | 打开任务详情，检查是否在等补充信息；确认 Agent 未被归档；若用 External Codex，确认设备与 Runner 可用。 | 对照 hive-4 恢复，但手册没写「重启后不重放」的界面 |

BFT 手册**没有**单独的审批卡字段表，也没有 In progress / Needs Review 这种个人版看板用语。

---

## 2. OpenBot（CopilotKit/OpenBot）

- 仓库：[https://github.com/CopilotKit/OpenBot](https://github.com/CopilotKit/OpenBot)（本会话 transcript 未能回放 Panda 当时贴出的原链接，按公开项目与房间上下文认定为此仓）。
- 文档：[README](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/README.md)、[docs/README.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/README.md)、[How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md)、以及 [open.bot](https://open.bot/)、[changelog](https://open.bot/changelog)。读取的是当前 `main` / 文档站，**没有**像 Comma 那样锁 SHA。
- 许可提醒（只作背景，不是界面）：应用源码为 MIT，**CopilotKit Intelligence 另行授权**。见 README License 节。

OpenBot 的产品模型不是「一块任务看板」，而是「对话里给 Bot 工作，Bot 在对话里汇报、出结果卡、在动作前问你」。八件套：Conversations、Routines、Memory、Skills、Apps、Computer、Approvals、Activity。[How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md)

### 2.a 页面 / 屏幕

| 页面或入口 | 文档里怎么说 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- |
| 前端形态 | Mac、Windows、web、iOS、Android、Slack、Teams 连到同一工作区；对话、交互结果、computer view、审批会按端适配。open.bot 上 iOS / Android / Slack / Teams 仍是 Early access。 | [README Architecture](https://github.com/CopilotKit/OpenBot)、[open.bot](https://open.bot/) | 待定（第一期只要 iced 桌面） |
| 应用壳：channels、Bot chat、live screen、settings、admin | `app` 负责这些页面。 | [architecture.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/architecture.md) | 会话与聊天：hive-5；live screen：不在第一期 |
| 侧边栏 / Home | 可从侧边栏菜单重命名自己开的对话。Bot 正在工作时，侧边栏会显示，Home 上有 Open。 | [Changelog 1.0.1 / 1.0.0](https://open.bot/changelog) | hive-5 会话列表 |
| 对话（Conversations） | 在这里给 Bot 工作、听它汇报。可写、说、发文件。一个对话可有多个 Bot。旧名 channel / group / chat 不再当独立地方。 | [How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md) | hive、hive-5 |
| 主菜单：Bots、Routines、Skills、Connections | 个人版主菜单这四项；名字下有 Admin、OpenBot for Business、Settings。 | [personal-edition.md「What the app shows」](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/personal-edition.md) | Bots 列表对照「拉 Bot」；Routines 对照定时；其余待定。**Bot 详情页不在第一期** |
| `/bots`、Bot 页 | 可 pin（Pinned 区）。Bot 页展示「正在做什么」和「等你处理的事」，旁边是 Activity；然后按固定顺序：Routines、Skills、Apps、Computer、Approvals、Setup（名称、说明、把工作交给谁）。个人版没有 Memory 行。 | [coworkers.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/coworkers.md)、[personal-edition.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/personal-edition.md)、[How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md) | 进行中 / 等待你：hive-2、hive-5；Setup / 详情：不在第一期 |
| Sharing | 团队版 Bot 的 Sharing 页。个人版没有 Sharing / Visibility 等「关于其他人」的项。 | [coworkers.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/coworkers.md)、[personal-edition.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/personal-edition.md) | 不在第一期（多租户 / 团队） |
| Routines（`/routines`、`/routines/new`、Bot 的 `/bots/…/routines`） | 一份列表，含 routine / goal / check-in。New routine 表单字段见下。对话输入框旁的时钟也可打开同一屏幕。 | [routines.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/routines.md)、[How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md) | hive、hive-4、hive-5 第 5 节 |
| Memory | 主菜单有 Memory。个人版**尚未提供**，无屏幕。团队版 Memory 页里，来自其他应用、OpenBot 没有行记录的条目显示为「Kept in Intelligence」，可编辑或遗忘，不能暂停。 | [How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md)、[architecture.md「Personal memory」](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/architecture.md)、[personal-edition.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/personal-edition.md) | 待定 |
| Skills、Teach a Bot | Skills 列出你写的、学会的技能（标明来源）；「Waiting for your review」里批准或拒绝；Learn now 可马上跑一轮。Teach a Bot 从 Skills 打开，可 Record / Stop，窗口内看到标签、地址栏和实时页面。Try it 会打开 Home 并备好待发送的任务。 | [personal-edition.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/personal-edition.md)、[Changelog 1.0.0](https://open.bot/changelog) | 待定 |
| Connections（`/connections`） | 两段：Your apps、Add an app（Connect / Add / Set up）。点开某应用进 `/connections/…`：工具、帐号、关闭。Vault 在网格下方，进入 Settings → Vault。 | [personal-edition.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/personal-edition.md) | 待定（连接器）；凭证不进第一期公开界面设计也可以后做 |
| Settings | 三组：**You**（General、Notifications、Vault）；**Every Bot**（Computer、Approvals、Activity）；**OpenBot**（Setup、Components gallery）。桌面 Settings 显示当前 AI 提供方与模型，并可 Change AI connection；可关遥测。 | [personal-edition.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/personal-edition.md)、[Changelog 1.0.1](https://open.bot/changelog)、[How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md) | 待定 |
| Settings → Approvals | 含 Extra vigilant、Auto-review、Commands on your computer；Always allow 的规则可在此删除。团队另有 Admin → Approvals。 | [approvals.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/approvals.md) | hive-4（策略本身）；设置页待定 |
| Settings → Activity / Admin → Activity | Bot 做过、被拒绝、失败、你批准过的记录。可 Show older events。团队版可问审计「上周某 routine 改过 Gmail」等。 | [How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md)、[Changelog 1.0.0 / 1.0.1](https://open.bot/changelog) | 待定 |
| Settings → Computer / live screen | Bot 有带浏览器、命令行和跨任务保留文件的 computer。页面需要你时（通行密钥、手机验证码），你接管做完再交还。营销页有 Live screen / Activity 面板。 | [How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md)、[open.bot](https://open.bot/) | **不在第一期**（桌面流 / 终端 / 文件树） |
| Admin | 按八件套排页，另有 Vault 与团队本身。个人版部分页面写明「仅团队适用」。含 What would happen、Boundaries、Providers 等。 | [How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md)、[personal-edition.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/personal-edition.md) | 不在第一期（多租户管理） |
| 首次使用 starter tour | 在与 Personal Assistant / picked-harness 的第一次新对话里给建议，无关闭控件；建议留在该对话。建议包括 Chart Hacker News、Browse Google Maps、Triage my inbox、Build a trip budget 等。 | [first-use-tour.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/testing/first-use-tour.md) | 待定 |

### 2.b 卡片上能看到的字段

| 卡片类型 | 文档写明的可见内容 | 未说明 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- | --- |
| 审批卡 | 用自然语言描述动作，例如「Send email on Gmail」，而不是工具名或原始数据；会点出要使用的帐号；Always allow 只覆盖该帐号。按钮：**Allow once**、**Allow for this conversation**、**Always allow here**、**Deny**。开启「Allow for this conversation」后，输入框上方会显示当前允许项，并有 **Turn off**。 | 卡片上是否还有页面 URL、元素名等——文档没写完整字段表 | [approvals.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/approvals.md)、[Changelog 1.0.0](https://open.bot/changelog) | hive-2 / hive-4。robroz 卡住只留「去私聊接管」，不要把这四个按钮搬进第一期群卡片 |
| 凭据 / 自己来做 | 改网站密码或凭据时，卡上写明这是你来做，按钮是 **I did it myself** 和 **Don't do it**。团队若设成「Members do it themselves」，同样是这两枚按钮。 | — | [approvals.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/approvals.md) | hive-4「不代填凭据，人到私聊接管」 |
| 连接应用卡（对话内） | Bot 需要未连接的应用时出卡。要登录的应用：**Connect**（开新标签登录，连上后对话继续）；无需登录或登录方式要先打开才知道的：**Turn on**；要填密钥的：**Set up**（打开 Connections 下对应页）。Gmail 分拣示例：先出 Gmail connect card，任何邮件被读取之前。 | — | [personal-edition.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/personal-edition.md)、[first-use-tour.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/testing/first-use-tour.md) | 待定 |
| 结果卡 | 默认可读 Markdown；只有更有用时才画 chart / table / card。图还在加载时写「Drawing the chart…」。预算类控件可在本地改数字并重算，不必再请求模型。长链接或路径会换行，不被裁切。 | 各类结果卡的完整字段 | [Changelog 1.0.0 / 1.0.1 / 1.0.2](https://open.bot/changelog)、[README 演示说明](https://github.com/CopilotKit/OpenBot) | hive-2 交付卡片（robroz 只要摘要 + 相对路径） |
| 技能保存卡 | 显示 command、title、完整 instruction；人按下按钮后才写入。 | 按钮文案 | [architecture.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/architecture.md) | 待定 |
| Activity 行 | 同样用自然语言描述动作；对话级允许的行会标 **Allowed for this conversation**。 | 完整列 | [approvals.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/approvals.md)、[Changelog 1.0.0](https://open.bot/changelog) | 待定 |
| 任务 / 状态卡 | OpenBot 文档**没有** Comma 那种三列看板任务卡。进行中通过侧边栏「正在工作」+ Home 的 Open、以及对话内活动来表现。 | 独立任务卡字段 | [Changelog 1.0.0](https://open.bot/changelog) | 对照 hive-2：robroz 仍用消息流里就地更新的状态卡，不要改成 OpenBot 的纯对话样式就算完成 |

营销页 [open.bot](https://open.bot/) 的演示对话里还能看到步骤行（读文档、读频道、登录门户、统计结果）和结果表，那是演示文案，**不作为字段契约**。

### 2.c 用户能看到的状态与流转

OpenBot 公开文档**没有** queued / In progress / Needs Review / Done 这样的看板列。用户能看到的是：

| 可见状态或情况 | 含义 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- |
| Bot 正在工作 | 侧边栏标明，Home 提供 Open。 | [Changelog 1.0.0](https://open.bot/changelog) | hive-3 `running`、hive-2「进行」 |
| 等待审批 | 对话里出现审批卡，或凭据卡要你自己做。 | [approvals.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/approvals.md)、[README](https://github.com/CopilotKit/OpenBot) | hive-3 `blocked`、hive-4 |
| 需要你接管页面 | 通行密钥或手机验证码等，你接管再交还。 | [How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md) | hive-4 私聊接管 |
| Routine / goal 进度 | 有结果要达成的 routine（goal）会显示进度。列表里正在工作的（on、active、未暂停）排在前面。 | [How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md)、[routines.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/routines.md) | hive-4；进度展示待定 |
| 以列表方式写的 routine | 会做到每一项完成、被堵住（blocked）或跳过（skipped）。 | [Changelog 1.0.1](https://open.bot/changelog) | 待定 |
| 由 check-in 启动的任务 | 无论其他规则如何，每次变更前都要问。 | [approvals.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/approvals.md) | hive-4 |
| 技能待审 | Skills 页「Waiting for your review」，或自动批准（默认开）。 | [personal-edition.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/personal-edition.md) | 待定 |

robroz 的四态（queued / running / blocked / done）在 OpenBot 文档里没有一一对应的用户文案，不要硬翻译。

### 2.d 交互

| 交互 | 文档怎么说 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- |
| 创建工作 | 在对话里向 Bot 写、说或发文件。说「每周一…」会变成 routine。也可在 Routines 页按 What / Starts / Does / Until 新建。New routine 字段顺序：Bot、What to do、It should（Act / Only tell you）、Starts（定时 / 事件 / 现在一次）、Done when（可选）、Posts in、Name。 | [How OpenBot works](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/how-openbot-works.md)、[routines.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/routines.md) | hive、hive-3、hive-4 |
| 请求审批 | 删除、发给别人、分享、发帖、付款、改密码或帐号设置，默认每次都问（可用一次 / 本对话 / 总是允许改）。其余按规则与 Extra vigilant。付款类现在是询问，不再一律「交给你做」（1.0.1 变更）。 | [approvals.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/approvals.md)、[Changelog 1.0.1](https://open.bot/changelog) | hive-4 |
| 批准 / 拒绝 | 四按钮见上。Allow once 只这一次；Allow for this conversation 限同一 Bot、工具、地点、帐号、本对话；Always allow here 写入 Settings → Approvals；Deny 则不做，并告诉 Bot 你拒绝了。 | [approvals.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/approvals.md) | 第一期群里不要做四按钮；对照「去私聊接管」 |
| 本机文件夹 | 桌面应用在你电脑的文件夹里每次编辑或命令都会问；Settings → Approvals 的 Commands on your computer 可直接禁止命令。 | 同上 | hive-4；本机文件夹 UI 待定 |
| 刷新 / 多标签续看 | 进行中的 run 会从头回放，所以刷新或第二个标签能看到全过程。事件经 Intelligence 实时网关推到应用。回到对话时，Bot 做到一半的查找不应再丢失（1.0.1 修复）。 | [architecture.md「Runtime flow」](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/architecture.md)、[Changelog 1.0.1](https://open.bot/changelog) | hive-3 事件流；「回放」是为了把进行中的过程再画出来，不等于把已做的外部改动再执行一遍——后者在公开文档里 **未作为用户可见开关说明** |
| 错过的 routine 不补跑 | 停机一个月回来，会把时间戳往前拨而不把错过的每一次都跑掉；正在进行时重启，最多丢掉那一次，下一次仍按时。 | [routines.md「Missed windows are skipped, not replayed」](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/routines.md) | hive-4（不重放）；界面是否提示「已跳过」**未说明** |
| Routine 疲劳规则 | 连续失败只在首次失败和第十次关掉时各发一条消息；关掉后要人再打开。 | [routines.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/routines.md) | 待定 |
| 实时更新 | Intelligence realtime gateway；非官方应用客户端用 join token。 | [architecture.md](https://raw.githubusercontent.com/CopilotKit/OpenBot/main/docs/architecture.md) | hive-3 |
| 输入与模型 | 输入框有麦克风（桌面用本机语音识别）。选模型后关闭选择器，光标回到输入框。 | [Changelog 1.0.2](https://open.bot/changelog) | 待定 |
| 移动端 | 前端会适配审批与 computer view；iOS / Android 在官网仍是 Early access。手机上如何完成发任务和审批的逐步说明 **未写全**。 | [README](https://github.com/CopilotKit/OpenBot)、[open.bot](https://open.bot/) | 待定 |

### 2.e 公开截图与演示

| 资源 | 说明 |
| --- | --- |
| [README 演示视频](https://github.com/user-attachments/assets/c60f07da-8b6d-46eb-a573-17fc1e65a644) | 说明文字：Bot 使用浏览器和工具、以交互结果作答、发送前等待批准、需要时由你接管。本文未按帧拆视频补字段。 |
| [open.bot](https://open.bot/) 首屏演示 | 对话 + Live screen + Activity + 结果表的营销演示，不是字段契约。 |
| 仓库 docs 中未发现单独的产品截图目录 | 界面依据以 README / docs / changelog 的文字为准。 |

---

## 3. OpenMausBot（milind-soni/OpenMausBot）

### 许可（仅来自 LICENSE、LICENSING.md、enterprise/LICENSE 与仓库元数据）

对照版本已在文首固定为 `ecabd15caa08f674877e155d03c553c30a35a63b`。下列许可结论在该 SHA 上核对过，与当时 main 上的文本一致。`enterprise/` **不参考、不使用**。

- 仓库：[https://github.com/milind-soni/OpenMausBot](https://github.com/milind-soni/OpenMausBot)
- GitHub 仓库许可元数据：`Apache License 2.0`（SPDX `Apache-2.0`）。[API](https://api.github.com/repos/milind-soni/OpenMausBot)
- 根目录 [LICENSE](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/LICENSE) 为 **Apache License 2.0**。README 写明 © 2026 Milind Soni and OpenMausBot contributors。
- **例外**：`enterprise/` 不在 Apache 2.0 下，而是 source-available 的 [OpenMausBot Enterprise License](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/enterprise/LICENSE)：可阅读、构建、评估，开发和测试免费；生产启用企业功能需要组织许可密钥；不得再分发或给第三方托管（除非另有合作协议）。删掉该目录后，剩余部分即开源版。详见 [LICENSING.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/LICENSING.md)。
- 打包的 Cua Driver 等第三方组件保留各自许可；商标权利不随 Apache 授予。OpenMausBot 是受 Grok Bot 启发的独立开源项目，与 xAI 无隶属关系。

产品站点：[openmausbot.com](https://www.openmausbot.com)；文档站：[docs.openmausbot.com](https://docs.openmausbot.com/docs/features)。仓库内另有详细 `docs/*.md`。读取的是当前 `main` / 文档站。

产品形状：聊天应用里的一组 Bot（联系人式侧边栏），每个 Bot 有个性、模型、电脑和已连接应用。旧称 Task 的对话现在叫 **Thread**。

### 3.a 页面 / 屏幕

| 页面或入口 | 文档里怎么说 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- |
| 桌面聊天壳 | macOS / Windows / Ubuntu 桌面；侧边栏是 Bot 通讯录。Chat UI、model picker、computer panel。 | [README Features / How it works](https://github.com/milind-soni/OpenMausBot)、[Feature overview](https://docs.openmausbot.com/docs/features) | hive-5 会话列表与消息流 |
| 侧边栏 Inbox | 侧边栏最上一行。三档：Needs you（问题与审批）、For your info（报告、提及、消息）、Done。手机上以 sheet 打开。Chief 私聊里有一张总览卡：多少件需要你、多少件仅供参考、最近几项带回答按钮、以及 Open inbox。 | [Threads, groups, and collaboration](https://docs.openmausbot.com/docs/features/chat-and-teams) | 对照 hive-2「卡住要显眼」；独立 Inbox 是否第一期做：待定 |
| Bot 及其 Threads | 展开 Bot → New thread，或 All threads 找旧对话。每个 thread 有自己的进度、审批和 Stop。同一 Bot 最多 3 个直接 thread 同时跑（含等待审批的）。 | [Bots, folders, and threads](https://docs.openmausbot.com/docs/features/bots-and-tasks) | hive-5；Stop / 多线程：待定 |
| 文件夹 | Bot 下可选的组织方式，无模型或权限设置。 | 同上 | 待定 |
| Groups（原 channels / rooms） | 多 Bot 共享上下文。可 stop @Bot / Resume；@everyone stop / resume。Hold 在重启后仍保留。 | 同上 | hive「建群、多 Bot」；暂停/恢复：待定 |
| New bot / Templates / Set me up / Bot settings | New bot 用默认值立即创建并打开聊天。Templates 先出可编辑草稿：profile、model、permissions、memory、skills、routines，再 Create bot。App settings → General → Defaults for new bots。 | 同上 | 拉 Bot：hive；Bot 设置页：不在第一期 |
| App Settings | 粘贴密钥（界面只显示已配置与否）；主题；API keys；Phone & devices；AI → Routing and extraction；Connections 等。公开截图：`docs/screenshots/app-settings.png`。 | [README「Keys once」](https://github.com/milind-soni/OpenMausBot)、[Feature overview](https://docs.openmausbot.com/docs/features)、[docs/screenshots](https://github.com/milind-soni/OpenMausBot/tree/main/docs/screenshots) | 待定 |
| Settings → Engines | 把引擎指到自定义 CLI。 | [README「Why」](https://github.com/milind-soni/OpenMausBot) | 待定 |
| Settings → Phone & devices | 桌面显示二维码给 iPhone 配对；可选 Keep this computer awake。 | [ios-companion.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/ios-companion.md) | 待定 |
| Computer panel | 打开后云桌面自行拉起；实时屏幕预览；「Open desktop」在浏览器里接管；也可指向 This Mac。公开截图：`computer-panel.png`、`ubuntu-computer-panel.png`、`docs-computer-panel.png`。 | [README「Every bot gets a computer」](https://github.com/milind-soni/OpenMausBot)、[Feature overview](https://docs.openmausbot.com/docs/features) | **不在第一期** |
| Inspector | 需要比对话更多细节时用：工具调用、重连、引擎输出、长任务。 | [Approvals and inspector](https://docs.openmausbot.com/docs/features/approvals-and-inspector) | 不在第一期（偏终端/日志） |
| Bot Settings → Memory | 明文记忆，可查看、改正、删除；用量条（80% 琥珀、超出红色）；编辑器不会覆盖 Bot 正在写的内容；变更日志可一键撤销；可关「Let this bot use memory」；有 Open in Obsidian / Show in Finder（仅本机）。 | [memory.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/memory.md)、[Voice, dictation, and memory](https://docs.openmausbot.com/docs/features/voice-and-memory) | 待定 |
| Automations / Routines | 有 tasks-routines、docs-automations 等公开截图。含一次性与工作日 routines、webhook。可暂停或删除「Keep an eye on my day」。 | [Feature overview](https://docs.openmausbot.com/docs/features)、[proactive-routines.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/proactive-routines.md)、[docs/screenshots/tasks-routines.png](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/tasks-routines.png) | hive-4 定时 |
| Connected apps / marketplace | Composio 市场；OAuth 一次。截图：`marketplace.png`、`docs-connected-apps.png`、`composio-multi-account.png`。 | [README](https://github.com/milind-soni/OpenMausBot) | 待定 |
| Teams → Import、分享卡 | 从 Markdown / GitHub URL / 文件夹导入团队；导入前有审查屏。分享卡见下。 | [README「Install a complete team」「Share a bot」](https://github.com/milind-soni/OpenMausBot) | 待定 |
| 配对 iOS / Android 伴侣 | 聊天、审批、搜索、transcript 控件、可选 computer viewing。凭证与敏感配置留在电脑上。 | [Feature overview](https://docs.openmausbot.com/docs/features)、[ios-companion.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/ios-companion.md) | 待定 |
| 独立的「任务看板」页 | 文档明确：以前叫 Tasks 的是 Threads，组织在侧边栏 Bot 下面，**不是** Comma 那种三列板。 | [Bots, folders, and threads](https://docs.openmausbot.com/docs/features/bots-and-tasks) | 对照时用 robroz 消息流卡片，不要改成看板才算齐 |

### 3.b 卡片上能看到的字段

| 卡片类型 | 文档写明的可见内容 | 未说明 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- | --- |
| 审批 / 权限卡 | 内联卡，动作为 shell 命令、文件编辑、提问等。可读「请求的动作和范围」，并与请求及 agent 的解释放在一起。按钮：**Allow / Deny**；以及 **Allow once**、**Allow for this session**（按钮就这样写，因为不落盘）；对已配置的自定义 MCP 只读工具另有 **Always allow**。👍 允许、👎 拒绝，与点卡相同；即使收回表情，决定仍保留。通话中用语音 yes/no 决定的卡会标 **by voice**，决策日志里也有。公开截图：`docs/screenshots/approval-card.png`。过期卡有 iOS 截图 `moca-282-expired-card-ios.jpg`，截图像素内容本文未当作字段表。 | 除「动作与范围」外的完整 schema（参数、路径、diff 等） | [README「Bots ask before they act」](https://github.com/milind-soni/OpenMausBot)、[Approvals and inspector](https://docs.openmausbot.com/docs/features/approvals-and-inspector)、[approval-levels.md「Answering a request」](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/approval-levels.md)、[chat-and-teams](https://docs.openmausbot.com/docs/features/chat-and-teams)、[voice-and-memory](https://docs.openmausbot.com/docs/features/voice-and-memory) | hive-2 / hive-4；第一期群卡不要搬 Allow once / session / always |
| 问题卡 | 任何审批级别都不会替你回答问题，都会留下 question card。Claude 可一次多个问题（CLI 上限 4，`ask_user` 最多 6）。选项如何排布 **部分未说明**。 | 选项字段的完整表 | [approval-levels.md「Asking you questions」](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/approval-levels.md) | hive-2 卡住；群里应用「带选项、点一下就回」来自 UI 姐对 Grok Bot 的建议，对照此卡，样式仍走 hive-2 |
| 凭据 / 连应用卡 | Bot 等你连接应用或输入凭据时，语音会让你在聊天里做（不要把凭据说出来）。iOS 可用 Password AutoFill 完成受支持的 credential-request cards。 | 桌面卡字段 | [voice-and-memory](https://docs.openmausbot.com/docs/features/voice-and-memory)、[ios-companion.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/ios-companion.md) | hive-4 不代填凭据 |
| Bot 自改一行 + Undo | 例如「Clive set up the team: added @Mira」，带 **Details**（全文）和 **Undo**；撤销后行末为 **Undone**。若此后又改过，或新 Bot 已被使用，Undo 会说明并什么都不做。删除 Bot 无 Undo。 | — | [Approvals and inspector](https://docs.openmausbot.com/docs/features/approvals-and-inspector) | 待定 |
| 群里的工作摘要行 | 工作中的笔记折成回复上方的 **Worked for…** 行；若没有最终消息，显示 finished without a summary，并再要一次短结果。 | — | [Bots, folders, and threads](https://docs.openmausbot.com/docs/features/bots-and-tasks) | hive-2 交付摘要 |
| 工具活动 chips | 流式回复带 tool-run activity chips；Bot 工作截图可折进 transcript。 | chip 上具体字段 | [README「Also in the box」](https://github.com/milind-soni/OpenMausBot)、[Feature overview](https://docs.openmausbot.com/docs/features) | 待定（不要做成终端） |
| 用量 chip | 聊天标题栏：可显示费用或 input tokens；悬停见 uncached / cached input、output、最近一条用量。群标题同样。 | — | [chat-and-teams](https://docs.openmausbot.com/docs/features/chat-and-teams) | 待定 |
| 每日简报卡 | 固定聊天里**就地编辑**的一张卡（无 AI 生成）。仅在有需要你的内容时才发。分段： **Needs you**；**Your day**（会议带 Join）；**Important emails**（一行 why，以及 Draft a reply、Open in Gmail、Dismiss）；**Routines**（当天其他 routine 结果）。另有会前准备卡、10 分钟提醒卡。 | — | [proactive-routines.md「What the person sees」](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/proactive-routines.md) | hive、hive-2 简报角标、hive-4 |
| 分享团队卡 | 一张卡：bots、说明、skills、图片、群聊、routines（暂停状态）、Chief of Staff、所用应用的**名称**。不含 chats、memory、keys、连接地址。按钮：**Publish**、**Save folder…**。 | — | [README「Share a bot or a department」](https://github.com/milind-soni/OpenMausBot) | 待定 |
| 通话记录行 | 「Call with」+ Bot 名；时长；为你做过的请求；步骤名称；提出的权限请求及如何被决定。不含语音本身。旧手机应用可能只显示「Call with Ada · 1:42」。 | — | [voice-and-memory](https://docs.openmausbot.com/docs/features/voice-and-memory) | 待定 |
| Inbox / Chief 总览卡 | 需要你的数量、仅供参考的数量、最近几项及回答按钮、Open inbox。 | — | [chat-and-teams](https://docs.openmausbot.com/docs/features/chat-and-teams) | 待定 |

独立「任务卡 / 交付卡」字段表：**未说明**（工作在 thread 里发生，交付表现为最终回复 + Worked for… / 附件 / 简报卡）。

### 3.c 用户能看到的状态与流转

| 可见状态或情况 | 含义 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- |
| Thread 正在工作 | 有进度与 Stop；可另开 thread；对忙着的 thread 发的消息会排队，部分引擎支持中途 steer。 | [bots-and-tasks](https://docs.openmausbot.com/docs/features/bots-and-tasks)、[chat-and-teams](https://docs.openmausbot.com/docs/features/chat-and-teams) | hive-3 `running` |
| 等待审批 | 内联审批卡；计入「最多 3 个直接 thread（含等待审批）」；Inbox 的 Needs you；应用图标与浏览器标签上的数字。 | 同上及 [chat-and-teams](https://docs.openmausbot.com/docs/features/chat-and-teams) | hive-3 `blocked`、hive-2 卡住要显眼 |
| 问题 / 缺凭据 | 任何级别都要人回答。 | [approval-levels.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/approval-levels.md) | hive-4 |
| 群内 Bot 被 hold | 当前回合结束，被 skip，直到 @Bot、说 resume 或按 Resume。跨重启保持。 | [bots-and-tasks](https://docs.openmausbot.com/docs/features/bots-and-tasks) | 待定 |
| 审批级别（composer / Bot settings） | **Ask for approval**、**Auto-accept edits**、**Approve for me**、**Full access**（Antigravity 下 Full access 叫 Auto）；Codex 另有 Custom。默认个人安装是 Full access；托管公司工作区默认 Ask。可在 Bot 工作时改级别。Composer 上有级别 chip。 | [approval-levels.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/approval-levels.md) | 待定（策略比第一期粗的「卡住就接管」更细） |
| 发送代操作 | Sending on your behalf：Ask every time 或 Send without asking。 | 同上 Full access 行 | hive-4 |
| Routine 行 | 例如「reads and drafts only」「checked … , nothing new」。失败会通知 owner。 | [proactive-routines.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/proactive-routines.md) | hive-4 |
| Inbox：Needs you / For your info / Done | 在聊天里回答后，也会离开 Needs you。 | [chat-and-teams](https://docs.openmausbot.com/docs/features/chat-and-teams) | 待定 |
| 模型回退一行 | 若 thread 自己的模型跑不了，改用 Bot 的模型，并在 thread 里写一行说明。 | [bots-and-tasks](https://docs.openmausbot.com/docs/features/bots-and-tasks) | 待定 |

没有与 Comma 看板或 robroz 四态一一对应的列名，不要硬翻。

### 3.d 交互

| 交互 | 文档怎么说 | 出处 | 对应 robroz 节点 |
| --- | --- | --- | --- |
| 创建工作 | 像给联系人发消息。New thread 开新对话。也可由 routine / webhook 以全新 task context 跑，并留下 run receipt。 | [README](https://github.com/milind-soni/OpenMausBot)、[Feature overview](https://docs.openmausbot.com/docs/features)、[bots-and-tasks](https://docs.openmausbot.com/docs/features/bots-and-tasks) | hive、hive-3 |
| 请求审批 | 由引擎自己的许可体系提出；OpenMausBot 用权限经纪把它变成你做的决定。云电脑与本机一样。My Cloud 上客人会话或 webhook 跑的别人的 routine，每次改动都要等 Cloud 主人。 | [README](https://github.com/milind-soni/OpenMausBot)、[Approvals and inspector](https://docs.openmausbot.com/docs/features/approvals-and-inspector) | hive-4 |
| 批准 / 拒绝 | Allow / Deny；Allow once；Allow for this session；部分 MCP 只读工具 Always allow（在 Bot 的 Access settings 可见、可撤销）。拒绝后控制交回 agent，让它走更安全的路。Inbox 里同一套 Approve / Deny，答了这边那边也算答了。 | [approval-levels.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/approval-levels.md)、[chat-and-teams](https://docs.openmausbot.com/docs/features/chat-and-teams) | 第一期群卡仍走「去私聊接管」 |
| 👍 / 👎 当答案 | 对「Shall I send it?」一类问题，👍 即继续，👎 即否。Bot 还在工作时，答案会等它；若在 Bot 看到之前撤回表情，答案一并撤回。审批卡上则撤回也不改决定。 | [chat-and-teams](https://docs.openmausbot.com/docs/features/chat-and-teams) | 待定 |
| 停止 / 恢复 | 每条 thread 有 Stop。群里 Stop Bot / Resume，或 stop @Bot / resume。 | [bots-and-tasks](https://docs.openmausbot.com/docs/features/bots-and-tasks) | 待定 |
| 刷新、重连、重启 | 应用折叠一条 SSE。iOS：可恢复的 SSE、流式回复、reconnect hydration。群里的 hold 跨重启保留。Rewind / 编辑旧消息会开新分支，**不会**去恢复已经丢掉的旧 provider 会话。简报卡在重启后按 id 从 Gmail 把发件人与主题读回。短暂的流失败会重试。 | [README「How it works」](https://github.com/milind-soni/OpenMausBot)、[ios-companion.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/ios-companion.md)、[bots-and-tasks](https://docs.openmausbot.com/docs/features/bots-and-tasks)、[chat-and-teams「Rewind」](https://docs.openmausbot.com/docs/features/chat-and-teams)、[proactive-routines.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/proactive-routines.md)、[Approvals and inspector「When a run disconnects」](https://docs.openmausbot.com/docs/features/approvals-and-inspector) | hive-3 事件流；重连把过程再画出来。**「已做的外部改动不重放」没有单独的用户可见开关说明**（与 Comma README 那句运行时承诺不同） |
| Routine 补跑 | 电脑无论多晚醒来，错过的 checks **合并成一次**，不会把每一次错过的都跑掉。 | [proactive-routines.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/proactive-routines.md) | hive-4 不重放错过的定时 |
| 实时更新 | 一条 SSE 事件流；工具活动直播；通话中每 30 秒给语音一条安静的状态说明。 | [README](https://github.com/milind-soni/OpenMausBot)、[Feature overview](https://docs.openmausbot.com/docs/features)、[voice-and-memory](https://docs.openmausbot.com/docs/features/voice-and-memory) | hive-3 |
| 移动端 | 配对 iOS：Bot / room 列表、分页 transcript、发送、中断、未读、审批与问题（含有限的 always allow）、可选电脑画面。后续才追求与桌面完全对齐的 thread 创建/切换/重命名/删除等。Android 有 composer 截图与 Play 安装说明。关闭应用后的推送，文档写明还不在这一版。 | [ios-companion.md](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/ios-companion.md)、[Feature overview](https://docs.openmausbot.com/docs/features)、[docs/screenshots](https://github.com/milind-soni/OpenMausBot/tree/main/docs/screenshots) | 待定；说明了手机上**可以**做聊天与审批 |
| 本机电脑控制 | 需显式 opt-in；Ubuntu Wayland 目前关闭。 | [README](https://github.com/milind-soni/OpenMausBot)、[Feature overview](https://docs.openmausbot.com/docs/features) | 不在第一期 |
| 语音批准 | 通话中可口头 yes/no 批权限；卡仍留在屏幕上。改其他 Bot 或团队的提案只能在屏幕上决定。 | [voice-and-memory](https://docs.openmausbot.com/docs/features/voice-and-memory) | 待定 |

端到端回路（README Status）：message → agent → streamed reply → tools → approvals → computer use。

### 3.e 公开截图与演示

官方截图目录：[https://github.com/milind-soni/OpenMausBot/tree/main/docs/screenshots](https://github.com/milind-soni/OpenMausBot/tree/main/docs/screenshots)（只列图片，未根据像素发明字段）。与界面参照最相关的包括：

- 审批：[approval-card.png](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/approval-card.png)
- 电脑面板：[computer-panel.png](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/computer-panel.png)、[docs-computer-panel.png](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/docs-computer-panel.png)
- 设置 / 模型 / 新 Bot / 已连接应用 / 自动化：[app-settings.png](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/app-settings.png)、[docs-model-picker.png](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/docs-model-picker.png)、[docs-fresh-bot.png](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/docs-fresh-bot.png)、[docs-connected-apps.png](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/docs-connected-apps.png)、[tasks-routines.png](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/tasks-routines.png)
- 资料与花名册：[agent-profile-desktop.png](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/agent-profile-desktop.png)、[agent-profile-ios.png](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/agent-profile-ios.png)
- iOS 活动与停：[activity-full-bubbles-ios.jpg](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/activity-full-bubbles-ios.jpg)、[moca-148-stop-ios.jpg](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/moca-148-stop-ios.jpg)、[moca-282-expired-card-ios.jpg](https://raw.githubusercontent.com/milind-soni/OpenMausBot/main/docs/screenshots/moca-282-expired-card-ios.jpg)

文档站部分功能页还带 og 图（如 [approvals-and-inspector/image.png](https://docs.openmausbot.com/og/docs/features/approvals-and-inspector/image.png)），属文档配图，不是字段表。

---

## 对照时请注意的差异（给并进清单用）

1. **任务在哪里**：Comma 是「聊天 + 三列看板」；OpenBot 与 OpenMausBot 是「通讯录式 Bot + 对话 / Thread」，没有同一块 In progress / Needs Review / Done 板。robroz 已冻结的是**消息流里就地更新的状态卡**（hive-2），不要为了对齐 Comma 改成看板，也不要把 OpenBot 的纯对话当成可以取消状态卡。
2. **审批出现的地方**：Comma 公开文档强调看板 Needs Review + 详情里选 Done；OpenBot / OpenMausBot 是对话内审批卡，OpenMausBot 还加了 Inbox，审批可在聊天或 Inbox 完成。robroz 第一期群里卡住只留「去私聊接管」。
3. **重启后续跑**：Comma README 明确写了「中断的变更动作不会自己再跑一遍」。OpenBot / OpenMausBot 文档强调的是刷新或重连时**把进行中的过程再画出来**、以及错过的定时**不补跑**；不要把「SSE 回放过程」写成「重放已做的外部副作用」。界面上「重启后已恢复、未重放」的专门提示，三家都 **未说明**。
4. **第一期不做**：文件树、终端、桌面流 / live screen / computer panel、Bot 详情页（hive、hive-5）。这些在三家里都很显眼，清单里应标「后面再做」，不要挤进第 6 节客户端。
5. **记忆**：Comma 在 Drive `/memory`；OpenBot 个人版还没有 Memory 页；OpenMausBot 有可检查、可改的明文 Memory 面板。robroz 第一期未列 Memory 页，标待定。
6. **视觉**：一律按 hive-2 原生控件与已定色（排队灰、进行深、完成绿、卡住琥珀），不抄三家的主题、气泡或配色。

---

## 未能获取或未使用的来源

- **缺失（未参考源码）**：Comma Quickstart 写明的逐步截图尚未发布，因此没有官方的「任务列表 / 执行中 / 等审批 / 重启后恢复」四态产品截图可引用。
- Comma、OpenBot 的 README 演示视频未按帧拆解，避免把视频里未写入文档的控件写成事实。
- **缺失（未参考源码）**：Comma 的 dynamic UI「卡片种类与字段」指向 card contract 源文件，**未打开**；该项保持未说明。
- Comma 文档站多页仍是占位提纲（尤其 Routines、Chat apps 的设置步骤）。
- 本执行会话的 transcript 搜索未能回放 Panda 最早分享的 OpenBot 链接；按公开仓库认定为 CopilotKit/OpenBot。若实际另有 fork，需要文档哥核对。
- OpenBot 仓库 docs 中未发现系统的产品截图目录；open.bot 首屏是营销演示。
- OpenMausBot 的 `docs/screenshots/` 已列出并引用关键文件 URL，但没有把每张图的像素文案逐一抄进字段表（字段以文档文字为准）。部分子目录（如 `triggers-v2/`、`automations-journey/`）未逐一打开。
- 未打开任何一方的源码，也未打开 OpenMausBot `enterprise/FEATURES` 等非许可、非文档文件。

## 许可提醒

- Comma 的原创代码是 AGPL-3.0-only。修改后通过网络提供服务，要向使用者提供对应源码。robroz 不拷贝、不翻译它的代码。
- OpenBot 应用源码是 MIT，但 CopilotKit Intelligence 另行授权；robroz 不使用 Intelligence，也不把它写进实现依赖。
- OpenMausBot 主体是 Apache-2.0：若直接复用其开源代码，须保留许可证、NOTICE 和署名。`enterprise/` 使用独立的 OpenMausBot Enterprise License，生产启用要组织授权，那一部分标为**不参考 / 不使用**。即便 Apache-2.0 比 AGPL 宽松，robroz 仍然只用 Rust 自研，本清单只参考功能和设计，不把对方代码搬进仓库。

# 参考功能清单

Project Hive 的目标是做出一个开源版的 Grok Bot：用 Rust 自研、单机可私有化的多智能体协同。本清单只记录参考产品**用户能看见、用得着的功能和行为**，供 Panda 勾选后按节点排期。写 Rust 和客户端的人都只照这份清单实现，不对照参考产品的源码。

固定版本（只读其 README、`docs/`、许可证和公开预览/截图）：

| 来源 | 固定版本 | 说明 |
| --- | --- | --- |
| [Comma](https://github.com/AFK-surf/Comma) | [`66ab3dc92475005aea9afd123df36a28e4ef86ab`](https://github.com/AFK-surf/Comma/tree/66ab3dc92475005aea9afd123df36a28e4ef86ab) | AGPL-3.0-only |
| [CopilotKit OpenBot](https://github.com/CopilotKit/openbot) | [`8d18f5d88be67334678c7f18c54c3b4797976a20`](https://github.com/CopilotKit/openbot/tree/8d18f5d88be67334678c7f18c54c3b4797976a20) | 应用 MIT；CopilotKit Intelligence 另行授权，Hive 不使用 |
| [OpenMausBot](https://github.com/milind-soni/OpenMausBot) | [`ecabd15caa08f674877e155d03c553c30a35a63b`](https://github.com/milind-soni/OpenMausBot/tree/ecabd15caa08f674877e155d03c553c30a35a63b) | 主体 Apache-2.0；`enterprise/` 为独立商业许可，**不参考、不使用** |
| Grok Bot 用户能看到的行为 | — | 只写用户看得见的产品行为，不写内部提示词或配置 |

「第一期？」按 Hive 已冻结的范围填写：单机私有化，跑通「说话、建群、拉 Bot、本机 Docker 隔离运行、做完休眠」，外加一次定时简报和一次私聊接管。文件树、终端、桌面流、Bot 详情页、Webhook、MCP 网关、浏览器自动化、多租户等不在第一期。勾选列留给 Panda。

## 目的与规则

- 只参考功能和行为，用来对照 Hive 已有节点（节点 1 契约、节点 2 宿主机编排、节点 3 沙盒和共享卷、节点 4 权限、节点 5 定时、节点 6 客户端、节点 7 联调）。对不上的标「无对应节点（新增？）」。
- 不看、不抄、不翻译对方的应用源码。Elixir 或 TypeScript 逐行译成 Rust 同样算衍生作品。
- 每项在「来源」列注明出处，链接到该版本的 README 或文档章节。没在文档或公开预览里核到的不写，或标「待核」。
- OpenMausBot 的 `enterprise/` 以及仅在该目录/商业许可下出现的能力（白标、公司 SSO、托管登录适配、许可制月度支出上限、按模型售价出账等）一律不列入功能行。

表头：功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选

## 持久任务与执行

| 功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 请求变成可持久任务 | 简短提问可在对话里直接回答；较大的工作成为独立任务，在「进行中 / 待审 / 完成」一类状态里流转。同一任务只改状态，不另开一张卡。 | [Comma README「What Comma does」](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does)；[Comma 产品概念：Task lifecycle](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/architecture/PRODUCT_MODEL.md#核心概念) | 节点 2、节点 6 | 是（四态为 queued / running / blocked / done，没有单独的「待审」看板） | [ ] |
| 后台执行，用户可继续聊天 | 长任务放到后台跑，聊天不卡住。进行中用状态卡原地更新。 | **Grok Bot 用户能看到的行为**；[Comma README：Tasks 是持久执行状态](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#philosophy)；[OpenBot README：在频道里看它工作](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#main-surfaces)；[OpenMausBot README：Status 环路含流式回复与工具](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#status) | 节点 2、节点 3、节点 6 | 是（状态卡排队 / 进行 / 卡住 / 完成） | [ ] |
| 关键节点汇报进度 | 执行中在关键里程碑推一次进度，而不是不停刷屏。 | **Grok Bot 用户能看到的行为** | 节点 6 | 待定（第一期已有四态原地更新；更细的里程碑文字需先对契约） | [ ] |
| 收到任务先短确认 | 先回一句短确认再开始干活，之后只在关键节点更新。 | **Grok Bot 用户能看到的行为**（界面建议进第一期） | 节点 6 | 待定（可用文本消息，但契约未要求执行器必须先发确认） | [ ] |
| 卡住时提醒需要输入 | 任务需要人决定时停在「需要输入 / 卡住」，并给出下一步入口。 | [Comma 任务文档：escalated 显示为 Needs input](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/salix/tasks-background-execution.md#canonical-lifecycle)；Hive 卡住只留「去私聊接管」 | 节点 4、节点 6 | 是 | [ ] |

## 审批与权限

| 功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 写外部资产或遇到验证码则停下 | 不代填凭据。任务变成卡住，删掉容器，人到私聊接管；恢复后挂回原来的共享卷。 | Hive 已冻结的权限契约；[OpenBot README：登录墙或 2FA 时把控制权交给用户](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features) | 节点 4 | 是 | [ ] |
| 不可逆操作先出审批卡 | 对外发消息、删除、提交、付款等不能撤回的操作，先出审批卡，用户点了才执行。拒绝或过期就是最终结果，不能自动重试。 | **Grok Bot 用户能看到的行为**；[OpenBot docs/approvals.md](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/approvals.md)；[OpenMausBot docs/approval-levels.md「Sending on your behalf」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/approval-levels.md)；[Comma README：Needs Review](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does) | 无对应节点（新增？）。第一期接近的是节点 4 的卡住 + 私聊接管 | 否（群内审批卡不在当前卡片契约里；要做需先改契约） | [ ] |
| 审批级别与当场作答 | 每个 bot 有审批级别（每次询问、自动接受编辑、代为批准、完全放行等）。会话里的审批卡可「允许一次」「本次会话允许」；对可信只读工具可给长期允许，并在该 bot 的访问设置里撤销。同一房间里的另一个 bot 仍会再问。问题卡任何级别都不会被自动代答。 | [OpenMausBot docs/approval-levels.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/approval-levels.md)；[OpenMausBot README「Bots ask before they act」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features) | 无对应节点（新增？） | 否 | [ ] |
| 选项卡，一点就能回复 | 需要用户拍板时发带选项的卡片，可点选项，也可自己写一句；不必为了选一项去打长文。 | **Grok Bot 用户能看到的行为**；[OpenMausBot 问题卡文档与截图](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/approval-levels.md#asking-you-questions)；[OpenBot：Bot 可停下来问人](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/architecture.md#asking-a-person) | 无对应节点（新增？） | 否 | [ ] |
| 对外邮件和消息先出草稿卡 | 要发出去的邮件或消息先以草稿卡呈现，用户自己点发送。 | **Grok Bot 用户能看到的行为**；[Comma 产品功能：草稿只经用户动作发出](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md#chat-and-tasks) | 无对应节点（新增？） | 否 | [ ] |
| 外部内容只当数据 | 网页、邮件、工具返回的内容只当数据处理，里面写的指令一律不执行，也不能凭这些内容自行授权操作。 | **Grok Bot 用户能看到的行为**；[Comma docs/verification.md：Data, forwarded text, and untrusted pages do not authorize a request](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/verification.md#ifc-authorization-contract) | 节点 4 | 待定（权限契约尚未写明这条，应对齐） | [ ] |

## 重启续跑与防重放

| 功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 中断后从原工作区接着跑 | 进程或容器没了，任务身份还在；恢复后挂回原来的共享卷，历史文件还在。 | [Comma README：Accepted work is not silently lost](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#built-with-elixir-and-lean-verified-with-lean-and-tla)；Hive `POST /api/tasks/:id/resume` | 节点 4 | 是 | [ ] |
| 被打断的修改不自动再跑一遍 | 已经做过或已被打断的改动操作，重启后不会自行重放。拒绝或过期的审批也不能自动重试；过期提案要重新出卡。 | [Comma README：An interrupted mutating action does not run again on its own](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#built-with-elixir-and-lean-verified-with-lean-and-tla)；**Grok Bot 用户能看到的行为**；[OpenMausBot 过期日程卡需重新提案](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/screenshots/moca-282-expired-card-ios.jpg) | 节点 4 | 待定（第一期已跳过已接管的阻塞行；一般写操作的防重放尚未写成契约） | [ ] |

## 记忆

| 功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 长期记忆按类型存放 | 用户资料、工作习惯、项目记录分开存，可查看、可改、可忘掉。Bot 形成的条目要用户确认后才收下。 | **Grok Bot 用户能看到的行为**；[OpenBot `/capabilities/memory`](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#main-surfaces)；[OpenMausBot docs/memory.md：Memory 面板、About me、日记](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/memory.md)；[Comma Drive 中的 memory / instructions](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md#files-and-navigation) | 无对应节点（新增？） | 否 | [ ] |
| 项目日志 | 按日或按项目留下工作记录，人可以打开看、可以撤销改动。 | **Grok Bot 用户能看到的行为**；[OpenMausBot：每日日志与变更日记](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/memory.md) | 无对应节点（新增？） | 否 | [ ] |

## 实时推送 / 流式

| 功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 同一张状态卡原地更新 | 一个任务一张卡，状态变化就地改，不另起一张。 | Hive 卡片规范；[Comma：Cards only project status](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/salix/tasks-background-execution.md#canonical-lifecycle) | 节点 6 | 是 | [ ] |
| 流式回复与活动提示 | 回复边生成边出现；工具或步骤用活动提示标出。客户端从一条事件流收更新。 | [OpenMausBot README「Also in the box」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[OpenMausBot README「How it works」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#how-it-works)；[OpenBot：看屏幕与 Activity 列表](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features) | 节点 6 | 是（第一期用 `GET /api/groups/:id/events` 轮询，不另开通道）；WebSocket 桌面流为否 | [ ] |

## 多 agent 与群聊

| 功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 建群、拉多个 Bot、指定执行者 | 一个群里可以有多个人和一个或多个 Bot；派任务时指定执行的 Bot。 | Hive 节点 2；[OpenBot：To 里放两个以上 Bot 即开群](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#main-surfaces)；[OpenMausBot 频道与可编辑 roster](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features) | 节点 2 | 是 | [ ] |
| 群聊里多个 Bot 分工 | 被点到的 Bot 用自己的名字回答，不由另一个 Bot 代述。可把工作交给队友，按接收方自己的权限跑。 | **Grok Bot 用户能看到的行为**；[OpenBot docs/architecture.md「One Bot handing work to another」](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/architecture.md#one-bot-handing-work-to-another)；[OpenMausBot 审批文档中的队友交接](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/approval-levels.md) | 无对应节点（新增？）。第一期只支持群内指定 assignee，没有 Bot 互派 | 否 | [ ] |
| 本地 agent 按 bot 接入 | 侧栏里每个 bot 是一台本地 agent（本机已登录的 Claude / Codex / Grok 命令行，或设置里接入的兼容引擎）。本机 127.0.0.1 上的 harness 托管这些进程；应用发命令、收一条事件流。可在对话中途换模型。数据和密钥默认留在本机目录。 | [OpenMausBot README「Why」「Pick a brain per bot」「How it works」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#why)；[OpenMausBot docs/custom-engines.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/custom-engines.md) | 无对应节点（新增？）。第一期执行器是本机 Docker 里的 Rust runner，不接外部 CLI | 否 | [ ] |
| 把 Bot 当联系人管理 | 侧栏像聊天列表：置顶、未读、改资料、复制、隐藏、删除；可暂停某个 bot、整个部门或全部 bot，恢复后数据和例行任务还在。 | [OpenMausBot README「Manage bots like chats」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[OpenMausBot docs/pausing-bots.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/pausing-bots.md) | 节点 2、节点 6（第一期只有群列表，没有 Bot 详情页和暂停） | 否 | [ ] |
| 频道 / 房间组织 | 工作、日常、项目分频道，各有自己的记录、共用说明、工作目录、应答规则和可编辑的 bot 名单。未点名的消息可由决策模型选一个合适的 bot，拿不准就交给房间负责人。 | [OpenMausBot README「Channels」「Fast decisions with Jev」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[OpenMausBot docs/decision-model.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/decision-model.md) | 节点 2（群） | 待定（分群是；自动选 Bot 和频道级说明为否） | [ ] |
| 整队导入与分享 | 用一份说明文件安装一组 bot、频道和建议例行任务；连接默认关闭，例行任务默认暂停，不携带会话、记忆、密钥或电脑权限。分享时只带角色、技能和要用的应用名。 | [OpenMausBot README「Install a complete team」「Share a bot or a department」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features) | 无对应节点（新增？） | 否 | [ ] |

## 定时与事件触发

| 功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 定时任务，到点出简报 | 宿主机按计划拉起任务，写下产物，发一张标了「简报」的交付卡。睡着的容器不能自己醒来。错过的窗口不补跑。 | Hive 节点 5；**Grok Bot 用户能看到的行为**；[Comma README：Routines 在设定时间给简报](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does)；[OpenBot docs/routines.md](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/routines.md) | 节点 5、节点 6 | 是 | [ ] |
| 事件触发 | 收件箱、仓库或已连接应用有约定变化时才叫醒 bot；安静时段不跑大模型。 | **Grok Bot 用户能看到的行为**；[Comma README：Loops](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does)；[OpenMausBot README「Triggers: schedules and webhooks」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#triggers-schedules-and-webhooks) | 无对应节点（新增？）。Webhook 不在第一期 | 否 | [ ] |
| 没变化时不打扰 | 没有新情况就不推送、不刷存在感。例行任务连续失败应停下并说明，而不是反复打扰。 | **Grok Bot 用户能看到的行为**；[Comma README：A quiet hour uses no large-model tokens](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does)；[OpenBot：疲劳规则与错过窗口跳过](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/routines.md)；[OpenMausBot：空简报不发出](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/proactive-routines.md#what-the-person-sees) | 节点 5（错过不补跑） | 待定（完整「无变化不推送」还需产品规则） | [ ] |
| 例行任务列表，可看可暂停 | 单独的列表里能看到所有定时和触发项、运行历史，并能暂停、恢复、立刻跑一次。对话里创建时出一张可编辑 / 暂停的卡片；过期或已变更的提案要重新出卡，不能再点旧卡。 | **Grok Bot 用户能看到的行为**；[OpenBot `/routines`](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#main-surfaces)；[OpenMausBot README 与 Tasks & routines 截图](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#triggers-schedules-and-webhooks) | 无对应节点（新增？）。契约目前只有 `POST /api/schedules` | 否 | [ ] |

## 工具 / 连接器

| 功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 连接器 | 把常用应用接到 bot。OAuth 由用户完成；授权范围按 bot 授予。未授权的应用不能用。 | **Grok Bot 用户能看到的行为**；[OpenBot `/connections` 与 MCP 目录](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#main-surfaces)；[OpenMausBot README「Connected apps」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[Comma 插件与连接请求卡](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/tools-integrations.md#request-cards) | 无对应节点（新增？）。MCP 网关不在第一期 | 否 | [ ] |
| 技能库 | 可复用的技能说明，对话里启用；保存或授予前要用户确认。技能本身不等于对外系统的权限。 | **Grok Bot 用户能看到的行为**；[OpenBot README：Skills are instructions, not capabilities](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features)；[Comma Drive 中的 skills](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md#files-and-navigation) | 无对应节点（新增？） | 否 | [ ] |
| 公开网页检索并注明出处 | 搜索或阅读公开网页，并引用 URL。检索查询按用户可见的「搜索网页 / 阅读页面」呈现。 | [OpenBot docs/web-research.md](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/web-research.md) | 无对应节点（新增？）。第一期只有按任务计数的出网 GET | 否 | [ ] |

## 沙箱电脑与浏览器

| 功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 本机隔离沙箱，做完休眠 | 每个任务在隔离环境里跑，做完删掉容器，群共享目录里的文件还在，交付卡上给出相对路径。 | Hive 节点 3；[Comma coding sandboxes](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/compute-devices.md#coding-sandboxes)；[OpenBot：每人/每 Bot 一台电脑](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features) | 节点 3 | 是（本机 Docker，不是微型虚拟机或第三方沙箱云） | [ ] |
| 按 bot 分配电脑 | 每个 bot 可单独选择运行处：云桌面、本机隔离的 Local VM、这台电脑，或关闭。电脑面板显示该 bot 的屏幕；云电脑未配置时说明如何补上密钥。选「这台电脑」需明确开启，并受平台限制。 | [OpenMausBot README「Every bot gets a computer」](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[Computer 面板截图](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/screenshots/docs-computer-panel.png)；[docs/computer-use-integration.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/computer-use-integration.md) | 无对应节点（新增？）。第一期是按群挂共享卷，不是按 bot 分虚拟机 | 否 | [ ] |
| 云端电脑和浏览器代办 | 看它的桌面或浏览器，必要时由用户接管（登录、验证码）。第一期不做桌面流和浏览器自动化。 | **Grok Bot 用户能看到的行为**；[OpenBot README：Watch / Take the wheel](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features)；[OpenMausBot：实时预览与「Open desktop」接管](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#features)；[Comma：Agent browser / Salix](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#what-comma-does) | 节点 4（私聊接管，无屏幕） | 否（浏览器、Playwright、桌面流均不在第一期） | [ ] |
| 文件树、终端、桌面流 | 在卡片或侧栏里浏览文件树、打开终端、看实时桌面。 | [OpenBot：屏幕与 Activity](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features) | 明确不在本期 | 否 | [ ] |

## 产出页面

| 功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 交付卡片 | 摘要加一条不能点开的等宽相对路径。定时简报仍是同一张交付卡，角上标「简报」。 | Hive 卡片规范 | 节点 6 | 是 | [ ] |
| 可发布的页面或小应用 | 用户让 agent 做一个可打开的预览页，确认后再发布；对外行动仍要在产品界面里点一下。 | [Comma docs/product-features.md Mini-apps](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/product-features.md#mini-apps) | 无对应节点（新增？） | 否 | [ ] |
| 对话里的通用组件 | 除状态卡和交付卡之外，用可交互组件代替纯文字（例如画廊、选项板）。 | [OpenBot README：Components instead of prose](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/README.md#features)；[Comma Chat dynamic UI](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/docs/clients.md#chat-dynamic-ui) | 无对应节点（新增？）。第一期只做状态卡和交付卡 | 否 | [ ] |

## 部署与自托管

| 功能 | 行为描述 | 来源 | Hive 节点 | 第一期? | 勾选 |
| --- | --- | --- | --- | --- | --- |
| 单机私有化，模型自备 | 在自己的机器上跑完整产品，不依赖对方账号；模型用自己的密钥或本机已有登录。 | Hive 第一期范围；[Comma README：Self-host with Docker Compose](https://github.com/AFK-surf/Comma/blob/66ab3dc92475005aea9afd123df36a28e4ef86ab/README.md#self-host-with-docker-compose)；[OpenBot Personal Edition](https://github.com/CopilotKit/openbot/blob/8d18f5d88be67334678c7f18c54c3b4797976a20/docs/personal-edition.md)；[OpenMausBot README：本机优先，桌面包内嵌 harness](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/README.md#why) | 节点 7 | 是 | [ ] |
| 用量可见、可设上限 | 能看到用量；可为整台机器或单个 bot 设每日 / 每月上限。到上限默认只拦住无人值守的例行任务，本人发的消息仍可走。80% 只改变用量条颜色，不弹窗。 | [OpenMausBot docs/usage-caps.md](https://github.com/milind-soni/OpenMausBot/blob/ecabd15caa08f674877e155d03c553c30a35a63b/docs/usage-caps.md)（开源版即有；许可制月度支出上限属 enterprise，不参考） | 无对应节点（新增？） | 否 | [ ] |

## 界面（前端补充）

@吊炸天前端专家 将根据 Comma、OpenBot、OpenMausBot 的 README、文档和公开截图，把页面、卡片字段和状态整理后补进本节。下面先记下 OpenMausBot 根目录公开预览页的**卡片种类**，以及截图/文档里已经能看见的字段（用自己的话转述，不抄标记）。字段不全的标「待核」，留给前端补。Hive 的卡片样式仍按已定规范做，不照搬对方视觉。

OpenMausBot 固定版本 `ecabd15` 根目录的 `*-preview.html` 是公开设计预览入口（页面本身只挂了预览标题，可见字段来自 `docs/screenshots/` 与文档，没有去读应用源码）：

| 预览 | 已看见的字段与行为 | 对照 Hive |
| --- | --- | --- |
| 问题卡（`question-cards-preview.html`，README 审批截图、`docs/question-card-cutoff.png`） | 标题（如「你最想先得到哪类帮助？」「Your bot has a question」）；一段说明或正文；带字母编号的选项；「自己写答案」输入框；关闭。进行中可把回答送进当前这一轮。任何审批级别都不会代答问题。 | 第一期没有选项卡。若后续要做，需先扩充卡片契约。 |
| 审批卡（README「Bots ask before they act」，文档「Answering a request」） | 内联卡，针对命令、改文件等：允许一次、本次会话允许；对可信只读工具可长期允许（可在该 bot 的访问设置里撤销）；拒绝。同一房间的其他 bot 仍会再问。 | 第一期用「卡住 + 去私聊接管」，群里不出审批卡。 |
| 电脑询问 / 电脑面板（`computer-ask-preview.html` 标题为「Turn On Your Computer」；`docs/screenshots/docs-computer-panel.png`） | 面板标题带该 bot 的名字和「屏幕」；未配置云电脑时的说明；可选的云电脑密钥；「运行于」四选一：Cloud、Local VM、This computer、Off。 | 第一期无电脑面板和桌面流。 |
| 房间卡（`room-card-preview.html`） | 预览页存在。频道在 README 里有各自的记录、共用说明、工作目录、应答规则和 bot 名单，但房间卡上具体排了哪些字段待核。 | 节点 6 目前是群列表 + 消息流。 |
| 用量表（`usage-meter-preview.html`，`docs/usage-caps.md`「What a person sees」） | 设置里的用量条；接近上限变提示色，达上限变强调色并写明何时恢复；不弹审批卡。 | 无对应节点。 |
| 日程 / 例行任务卡（Tasks & routines 截图、过期卡截图） | 列表页含 Routines / Webhooks 分页、日历、筛选、「新建」；说明一次任务与可复用日程的区别。对话里的日程提案卡有标题、将要创建的例行任务说明；若提案之后例行任务已变，卡片标过期，要求重新提案。 | 第一期只有定时简报交付卡，没有例行任务列表。 |
| 其他预览（欢迎流程、主页、电话连接、取消方案、云购买、吉祥物动作） | 仅确认有这些预览页，字段待核。取消方案、云购买若只服务于商业订购，前端补录时请排除 enterprise 范围。 | — |

## 许可提醒

- Comma 的原创代码是 AGPL-3.0-only。修改后通过网络提供服务，要向使用者提供对应源码。Hive 不拷贝、不翻译它的代码。
- OpenBot 应用源码是 MIT，但 CopilotKit Intelligence 另行授权；Hive 不使用 Intelligence，也不把它写进实现依赖。
- OpenMausBot 主体是 Apache-2.0：若直接复用其开源代码，须保留许可证、NOTICE 和署名。`enterprise/` 使用独立的 OpenMausBot Enterprise License，生产启用要组织授权，那一部分标为**不参考 / 不使用**。即便 Apache-2.0 比 AGPL 宽松，Hive 仍然只用 Rust 自研，本清单只参考功能和设计，不把对方代码搬进仓库。


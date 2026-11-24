# 概念和术语

robroz 里的叫法、任务状态和卡片是什么意思。接口字段的精确定义见 [api.md](api.md)；本页只讲概念，和 api.md 冲突时以 api.md 和代码为准。

## 术语表

| 叫法 | 是什么 | 在哪里用 |
| --- | --- | --- |
| bro | 一个 agent，也就是一个 bot。新建一个 agent 叫「引入一个 bro」。 | 只用在界面上。客户端现在的样子：会话列表标题是「聊天」，新建按钮是「引入一个 bro」（`crates/robroz-ui/src/conversations.rs` 的 `TITLE`、`NEW_BRO`），列表空时显示「还没有 bro，引入第一个」。点私聊顶部的名字可以改名。bro 资料页还没做，排在第二期（「把 Bot 当联系人管理」），见 [roadmap-phase2.md](roadmap-phase2.md)。 |
| 默认名 | 新建时自动填的名字：第一个叫 `bro`，之后依次是 `bro 2`、`bro 3`，取第一个没被占用的。用户随时可以改名。 | 只在新建时填一个默认值，不新增字段。 |
| `botId` / `botName` | 契约里冻住的字段名。 | 代码、接口和数据库里照旧用这两个名字，不改成 bro。 |
| 群 | 一个会话，可以有多个人和多个 bro。 | 界面、代码和接口都用这个叫法。 |
| 私聊 | 只有你和一个 bro 的群，不是另一种概念。 | 界面上叫私聊，代码里就是一个群。消息、任务、事件、实时推送和未读数都直接可用。 |
| 群目录 | 每个群一个目录 `ROBROZ_VOLUMES/{groupId}`，挂到容器里的 `/workspace`。容器删掉后目录还在。 | 交付卡的 `path` 是相对这个目录的路径。 |
| 控制目录 | 每个任务一个目录 `ROBROZ_CONTROL/{taskId}`，只有宿主机写，只读挂到容器的 `/control`。 | 放 `prompt.txt`、`reply.txt`、接管标记 `takeover`。见 [architecture.md](architecture.md)。 |
| 例行任务 | 定时任务。到点由宿主机新建一个普通任务。 | 界面按钮叫「例行任务」。接口 `/api/schedules`。 |
| 简报 | 例行任务成功后的交付卡，`briefing` 为 true。不是新卡种。 | 客户端角上标「简报」（`robroz_card::briefing_badge`）。 |

## bro 的名字

bot 的名字是全局的：同一个 `botId` 在所有群里叫同一个名字。`/say` 里的 `@名字` 匹配的就是这个全局名字。`POST /api/groups/{id}/members` 给已经存在的 bot 传 `name` 不会改名，改名用 `PATCH /api/bros/{id}`。

## 任务状态

`state` 只有四种，冻住，不会加第五种：`queued`（排队）、`running`（在跑）、`blocked`（卡住，要人）、`done`（结束）。

「失败」不是一种状态，用下面两种方式表达：

- **运行失败**：容器非 0 退出、没写出结果、或者容器中途消失（宿主机或 Docker 重启、容器被删）。任务结束成 `done`，`failure` 为「没写出结果」，没有交付卡。不重新排队，不再起容器，`resume` 返回 409「这次不能恢复」。不加第五种状态。
- **没做成**：任务 `done`，但带 `failure`。见 [permissions.md 的「没做成」](permissions.md#没做成failure)。

## 状态卡的几个字段

一个任务恰好一张状态卡，`id` 等于 `taskId`，按 id 原地更新，`createdAt` 不变、`updatedAt` 跟着变化。

| 字段 | 含义 |
| --- | --- |
| `containerUp` | 只有容器还在跑的 `running` 是 true。排队、完成、卡住、运行失败都是 false。 |
| `label` | 状态那句话，来自 `robroz_card::status_line(state, containerUp)`：排队、进行、没写出结果、卡住、完成。 |
| `failure` | 可选。`done` 但没做成时的原因。 |
| `waitingFor` | 可选。`blocked` 时等什么：`approval`（去答审批卡）或 `takeover`（去私聊接管）。 |
| `progress` | 可选。`running` 且容器在跑时，bro 正在干什么；任务因 MCP 工具等审批而 `blocked` 时，是「在用 {server}.{tool}」。 |

颜色和动作也来自 `robroz-card`（`appearance`、`palette`），客户端不自己定：排队、运行失败、等批准、没做成是灰色；卡住（等接管）是琥珀色，动作「去私聊接管」。

### progress

- 宿主机自己报关键节点：开跑时「开始干活」（接了模型时是「在想怎么回」），沙箱起来后「在沙箱里跑」，调用工具时「在用 <工具名>」（MCP 工具用展示名 `{server}.{tool}`）。
- bro 自己报：prompt 或模型回复里写一行 `robroz-progress:文字`。runner 把最后一行进度写到群目录 `tasks/{taskId}/progress`，宿主机在任务运行时读；模型回复里的进度行开跑时就用上。进度行不进 `result.txt`。
- 规范成一行：换行和连续空白变一个空格，去掉首尾空白，最多 60 个字（超了截到 59 个字加「…」），不报错。
- 文字没变就不改 `updatedAt`，不多推一次。
- 任务结束、卡住、容器没了时清掉；重新开跑从头报。例外：MCP 工具等审批的 `blocked` 保留「在用 {server}.{tool}」这一行，答卡后清掉。
- 代码：`host/src/progress.rs`。

## 卡片（事件）

事件流里每一项是一个 `Event`，按 `type` 区分。完整字段见 [api.md 的「事件类型」](api.md#事件类型)。

| `type` | 是什么 | 谁发 |
| --- | --- | --- |
| `message` | 一条消息，`kind` 是 `text`、`image` 或 `file`。只落库，不写文件。 | 人（`/say`、`/messages`） |
| `status` | 任务状态卡，一个任务一张。 | 宿主机 |
| `delivery` | 交付卡：摘要加群目录里的相对路径，现在是 `tasks/{taskId}/result.txt`。 | 宿主机，任务写出结果时 |
| `choice` | 选项卡：在 2 到 4 个固定选项里选，可单选或多选，`primary` 是 bro 的推荐。 | bro，经接口 |
| `approval` | 审批卡：删除、发送或花钱前问人。任务碰到对外写入时宿主机也会替它发一张（带 `taskId`）。 | bro 或宿主机 |
| `confirm` | 确认卡：bro 提议记下一条记忆或收下一项技能，人决定。 | 宿主机，任务成功后 |

规则：

- 卡片再变化时用同一个 `id` 再出现，客户端按 id 原地替换。
- 客户端碰到不认识的 `type` 要跳过，以后新增事件时旧客户端才不会出错。
- 不同类型事件之间没有约定先后，不要依赖。

## 已读和未读

每个群一份已读位置（现在是单用户）。计入未读的只有 bro 发来、需要你看的东西：已读位置之后的交付卡、选项卡、审批卡、确认卡，以及变成卡住的任务。你自己发的消息和排队、进行、完成这类状态变化不计。规则细节见 [api.md 的「未读」](api.md#未读)。

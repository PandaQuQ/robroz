# 记忆和技能

bro 记住的关于你的事（记忆），和 bro 会的做法（技能）。两者都只是拼进模型 system 提示的文字，不开放任何权限；权限只由 [permissions.md](permissions.md) 里的策略决定。接口字段和报错见 [api.md 的「记忆」「技能」「确认卡」](api.md#记忆)，客户端页面见 [client.md](client.md#memories-page)。

## 记忆

- 三种 `kind`：`profile`（你是谁）、`habit`（你的工作习惯）、`project`（项目记录）。
- `scope`：`user` 所有 bro 都能用，`bot:<botId>` 只给这个 bro（bro 必须存在）。
- `text` 去掉首尾空白后非空，最多 2000 个字符。
- 你自己记的（`POST /api/memories`）直接是 `kept`，`source=user`。bro 提的先是 `pending`，`source=bot`，要在确认卡上点了才算数。
- 「忘掉」是软删：记下忘掉的时间，以后任何列表都不返回。忘掉 `pending` 的记忆时，它那张没回答的确认卡一起关掉，记成 `denied`，`answeredBy` 为 `null`。
- `status` 只能通过确认卡改，`PATCH` 里带了也不理会。
- 代码：`host/src/memories.rs`。

## 技能

- 一项技能是 `slug` + `title` + `instruction`。`slug` 是身份，小写字母、数字和单个连字符，不能以连字符开头或结尾，最多 64 个字符，建了以后不能改。`title` 最多 80 个字符，`instruction` 最多 8000 个字符。
- `scope` 和记忆一样。
- 生命周期同记忆：人写的直接 `kept`，bro 提的先 `pending`。同一个 `slug` 没忘掉时不能再建（409），忘掉以后可以再写一条。
- 技能还要**按群启用**（`PUT /api/groups/{id}/skills/{slug}`）才会进提示。`kept` 但没在这个群启用的技能，这个群里的任务用不到。
- 代码：`host/src/skills.rs`。

## 拼进提示

接了模型时，宿主机给执行这个任务的 bro 拼 system 提示（`host/src/pump.rs:system_extra`）：

```
<固定开场白 robroz_model::SYSTEM_PROMPT>

你记得这些：
[habit] 周报用中文
[profile] …

你会这些技能：
### 写周报 (weekly-report)
先列三点
```

| 段 | 选哪些 | 排序 | 上限 | 代码 |
| --- | --- | --- | --- | --- |
| 记忆 | `scope` 为 `user` 或 `bot:<执行者>`，`status=kept`，没忘掉 | `updatedAt` 从新到旧 | 整段 4000 个字符 | `memories::kept_for`、`prompt_section` |
| 技能 | 同上，并且**已在该群启用** | `updatedAt` 从新到旧 | 整段 8000 个字符 | `skills::kept_for`、`prompt_section` |

- 超过上限时整条丢掉最旧的，不截半条。
- 一条都没有时不加这一段；没有记忆时技能段直接接在开场白后面。
- `pending`、`rejected` 和忘掉的都不进去。
- 没接模型时（没设 `ROBROZ_MODEL_KEY`）不拼，也不影响回显。

## bro 怎么提议

bro 不能直接写记忆或技能，只能在 prompt 或模型回复里写一行提议：

```
robroz-memory:{"kind":"habit","text":"周报用中文","scope":"user"}
robroz-skill:{"slug":"weekly-report","title":"写周报","instruction":"先列三点","scope":"user"}
```

- 记忆的 `kind`、`text` 必填；技能的 `slug`、`title`、`instruction` 必填；`scope` 都可省，默认 `bot:<执行这个任务的 bro>`。
- runner 把它们写进群目录 `tasks/{taskId}/proposals.jsonl`，每行 `{"type":"memory",…}` 或 `{"type":"skill",…}`。这些行不进 `result.txt`。
- 任务成功跑完（`done` 且没有 `failure`）后宿主机读这个文件，读完就删。没跑完、卡住、带 `failure` 的任务提的都不收。
- 不收（直接跳过、不报错、不影响任务）：JSON 不对、`type` 不认识、必填字段不对或超长、`scope` 写法不对或 bro 不存在；已有同 `scope`、同 `kind`、同样文字（连续空白算一个）且没忘掉的记忆，不论状态；已有同 `slug` 且没忘掉的技能，不论状态。
- 一个任务最多收前 20 条，记忆和技能合计（`PROPOSALS_PER_TASK_MAX`）。
- 每收一条存一条 `pending` 的记忆或技能，再发一张 `confirm` 卡。代码：`host/src/proposals.rs`。

## 确认卡

- 回答用审批卡的接口：`POST /api/approvals/{confirmId}/approve` 是「记下／保存」，变成 `kept`；`/deny` 是「不用」，变成 `rejected`，记录留着但不进提示，bro 也不会再提同一条。只能答一次，再答 409「已经答过了」。
- 列表里 `pending` 条目带 `confirmId`，用它调这两个接口，聊天里那张卡会跟着变。
- 卡上的内容（记忆的 `kind`、`text`、`scope`；技能的 `slug`、`title`、`instruction`、`scope`）是条目**现在**的样子，`pending` 时 `PATCH` 改了会再推一次。
- 存在和审批卡同一张表，但只以 `confirm` 事件出现，不影响状态卡的 `waitingFor`。新卡算一条未读。
- 技能卡点了「保存」只是变成 `kept`，还要在群里启用才会进提示。

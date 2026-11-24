# 接口参考

宿主机 `robroz-host` 的全部 HTTP 接口，以 `main` 上的代码为准。路由表在 `host/src/lib.rs:router_from`，请求体和返回体的类型在 `crates/robroz-contract/src/lib.rs`。

## 通用约定

- **地址**：默认 `http://127.0.0.1:8790`，见 [configuration.md](configuration.md) 的 `ROBROZ_ADDR`。下文示例用 `B=http://127.0.0.1:8790`。
- **令牌**：宿主机设了 `ROBROZ_TOKEN` 或 `ROBROZ_TOKEN_FILE` 时，**每个** `/api` 请求都要带 `Authorization: Bearer <你的令牌>`，本机访问也要。缺了或错了返回 401 `{"error":"缺少或错误的令牌"}`，响应头带 `WWW-Authenticate: Bearer`（`host/src/auth.rs:check_bearer`）。没设令牌时不鉴权。下文示例省略这个头。
- **格式**：请求和返回都是 JSON，字段名 camelCase。发 JSON 时带 `Content-Type: application/json`。
- **报错**：宿主机自己的报错是 `{"error":"原文"}`（`host/src/lib.rs:error_response`）。数据库出错一律 500 `{"error":"数据库错误"}`。
- **框架报错**：请求体不是合法 JSON、缺必填字段、枚举值不认识时，多数接口由 axum 直接拒绝，返回 400 / 415 / 422 和一段**纯文本**（不是 `{"error"}`），例如 422 `Failed to deserialize the JSON body into the target type: missing field ...`。下文标了「自定义」的接口把这类情况改成了 `{"error"}`。
- **id**：服务端生成的 id 都是 UUID v4 字符串。`botId` 由调用方给，任意非空字符串。
- **时间**：RFC 3339，UTC，毫秒精度，结尾 `Z`，例如 `2026-11-21T02:35:48.932Z`。
- **租户**：单租户，`tenantId` 固定是 `local`。
- **字段冻结**：已有字段不改，只会新增可选字段。客户端遇到不认识的字段和事件 `type` 要忽略或跳过，见 [AGENTS.md](../AGENTS.md)。

## 接口总表

| 方法 | 路径 | 用途 | 代码位置 |
| --- | --- | --- | --- |
| POST | `/api/groups` | 建群 | `host/src/lib.rs:create_group` |
| GET | `/api/groups` | 列群 | `host/src/lib.rs:list_groups` |
| POST | `/api/groups/{id}/members` | 把 bro 拉进群 | `host/src/lib.rs:add_member` |
| POST | `/api/bros` | 新建 bro 和它的私聊 | `host/src/bros.rs:create_bro` |
| GET | `/api/bros` | 列 bro | `host/src/bros.rs:list_bros` |
| PATCH | `/api/bros/{id}` | 改名 | `host/src/bros.rs:rename_bro` |
| POST | `/api/bros/{id}/dm` | 取或建私聊 | `host/src/bros.rs:open_dm` |
| POST | `/api/groups/{id}/messages` | 存一条消息 | `host/src/lib.rs:post_message` |
| POST | `/api/groups/{id}/say` | 说一句话，按需派任务 | `host/src/say.rs:say` |
| POST | `/api/groups/{id}/tasks` | 直接派任务 | `host/src/lib.rs:create_task` |
| POST | `/api/tasks/{id}/resume` | 接管后恢复卡住的任务 | `host/src/lib.rs:resume_task` |
| GET | `/api/groups/{id}/events` | 拉群里的全部卡片 | `host/src/lib.rs:list_events` |
| GET | `/api/groups/{id}/stream` | SSE 实时推送卡片 | `host/src/stream.rs:stream_events` |
| GET | `/api/unread` | 所有群的未读 | `host/src/unread.rs:list_unread` |
| GET | `/api/groups/{id}/unread` | 一个群的未读 | `host/src/unread.rs:group_unread` |
| POST | `/api/groups/{id}/read` | 标记已读 | `host/src/unread.rs:mark_read` |
| POST | `/api/groups/{id}/choices` | bro 发选项卡 | `host/src/choices.rs:create_choice` |
| POST | `/api/choices/{id}/answer` | 回答选项卡 | `host/src/choices.rs:answer_choice` |
| POST | `/api/groups/{id}/approvals` | bro 发审批卡 | `host/src/approvals.rs:create_approval` |
| GET | `/api/approvals/{id}` | 读一张审批卡 | `host/src/approvals.rs:get_approval` |
| POST | `/api/approvals/{id}/approve` | 同意审批卡 / 确认卡 | `host/src/approvals.rs:approve` |
| POST | `/api/approvals/{id}/deny` | 拒绝审批卡 / 确认卡 | `host/src/approvals.rs:deny` |
| POST | `/api/schedules` | 建例行任务 | `host/src/lib.rs:create_schedule` |
| GET | `/api/groups/{id}/schedules` | 列群的例行任务 | `host/src/routines.rs:list_schedules` |
| PATCH | `/api/schedules/{id}` | 暂停 / 恢复 | `host/src/routines.rs:update_schedule` |
| GET | `/api/memories` | 列记忆 | `host/src/memories.rs:list_memories` |
| POST | `/api/memories` | 记一条 | `host/src/memories.rs:create_memory` |
| PATCH | `/api/memories/{id}` | 改记忆 | `host/src/memories.rs:update_memory` |
| DELETE | `/api/memories/{id}` | 忘掉 | `host/src/memories.rs:forget_memory` |
| GET | `/api/skills` | 列技能 | `host/src/skills.rs:list_skills` |
| POST | `/api/skills` | 写一条技能 | `host/src/skills.rs:create_skill` |
| PATCH | `/api/skills/{slug}` | 改技能 | `host/src/skills.rs:update_skill` |
| DELETE | `/api/skills/{slug}` | 忘掉技能 | `host/src/skills.rs:forget_skill` |
| GET | `/api/groups/{id}/skills` | 群里启用的技能 | `host/src/skills.rs:list_group_skills` |
| PUT | `/api/groups/{id}/skills/{slug}` | 在群里启用技能 | `host/src/skills.rs:enable_skill` |
| DELETE | `/api/groups/{id}/skills/{slug}` | 在群里关掉技能 | `host/src/skills.rs:disable_skill` |
| GET | `/api/settings/search` | 读搜索设置 | `host/src/search.rs:get_search` |
| PUT | `/api/settings/search` | 改搜索设置 | `host/src/search.rs:put_search` |
| POST | `/api/settings/search/test` | 试连搜索 | `host/src/search.rs:test_search` |
| GET | `/api/settings/model` | 读模型设置 | `host/src/model_settings.rs:get_model` |
| PUT | `/api/settings/model` | 改模型设置 | `host/src/model_settings.rs:put_model` |
| POST | `/api/settings/model/test` | 试连模型 | `host/src/model_settings.rs:test_model` |
| GET | `/api/settings/review-rules` | 列已确认的放行规则 | `host/src/review/rules.rs:list_rules` |
| POST | `/api/settings/review-rules` | 提议加一条放行规则（发确认卡） | `host/src/review/rules.rs:create_rule` |
| DELETE | `/api/settings/review-rules/{id}` | 提议删一条放行规则（发确认卡） | `host/src/review/rules.rs:delete_rule` |
| GET | `/api/mcp/servers` | 列 MCP 服务器 | `host/src/mcp.rs:list_servers` |
| POST | `/api/mcp/servers` | 加 MCP 服务器 | `host/src/mcp.rs:create_server` |
| PATCH | `/api/mcp/servers/{id}` | 改 MCP 服务器 | `host/src/mcp.rs:patch_server` |
| DELETE | `/api/mcp/servers/{id}` | 删 MCP 服务器 | `host/src/mcp.rs:delete_server` |
| GET | `/api/mcp/servers/{id}/tools` | 实时列工具 | `host/src/mcp.rs:list_tools` |
| POST | `/api/mcp/servers/{id}/test` | 试连 | `host/src/mcp.rs:test_connection` |
| POST | `/api/mcp/servers/{id}/approve-command` | 批准 stdio 启动命令 | `host/src/mcp.rs:approve_command` |
| GET | `/api/groups/{id}/mcp` | 群里启用的服务器 | `host/src/mcp.rs:list_group_mcp` |
| GET | `/api/groups/{id}/mcp/{serverId}` | 群里某台服务器 | `host/src/mcp.rs:get_group_mcp` |
| PUT | `/api/groups/{id}/mcp/{serverId}` | 在群里启用 | `host/src/mcp.rs:enable_group_mcp` |
| DELETE | `/api/groups/{id}/mcp/{serverId}` | 在群里关掉 | `host/src/mcp.rs:disable_group_mcp` |

---

## 群和成员

私聊就是只有一个 bro 的群（库里 `groups.dm_bot_id` 有值），所有按群的接口对私聊同样适用。

### POST /api/groups

建群。`host/src/lib.rs:create_group`

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `name` | string | 是 | 去掉首尾空白后不能为空。 |

- 201 `Group`：`{"id","name","tenantId"}`。
- 400「请填写名称」：`name` 为空白。
- 422（纯文本）：缺 `name`。

```sh
curl -s -X POST $B/api/groups -H 'content-type: application/json' -d '{"name":"官网"}'
# {"id":"<groupId>","name":"官网","tenantId":"local"}
```

### GET /api/groups

列出所有群，**包括私聊**，按创建先后。`host/src/lib.rs:list_groups`

- 200 `{"groups":[Group]}`，没有群时 `{"groups":[]}`。

```sh
curl -s $B/api/groups
```

### POST /api/groups/{id}/members

把 bro 拉进群。`host/src/lib.rs:add_member`

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `botId` | string | 是 | bro 的 id。不存在就新建这个 bot。 |
| `name` | string | 是 | 只在新建 bot 时作为它的名字。bot 已存在时**忽略**，沿用库里的名字。 |

- 201 `Member` `{"groupId","botId","name"}`：新加入。`name` 是库里存的名字。
- 200 `Member`：已经在群里，不重复记。
- 400「请填写名称」：`botId` 或 `name` 为空白。
- 404「群不存在」。
- 409「私聊里只能有一个 bro」：往私聊里加别的 bro。

注意：bot 的名字是全局的，群里 `@名字` 匹配的是这个全局名字（见 `/say`）。用 `PATCH /api/bros/{id}` 改名。通过这里新建的 bot 没有私聊，`dmGroupId` 为 `null`，直到调用 `POST /api/bros/{id}/dm`。

```sh
curl -s -X POST $B/api/groups/<groupId>/members -H 'content-type: application/json' \
  -d '{"botId":"bot-ada","name":"ada"}'
```

### POST /api/bros

新建一个 bro，同时建好它的私聊。`host/src/bros.rs:create_bro`

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `name` | string | 否 | 去掉首尾空白，非空，最多 64 个字符。不给就用默认名：`bro`，被占了就从 `bro 2` 往上取第一个空的（`robroz_contract::next_bro_name`）。名字可以重复。 |

请求体可以是 `{}`，也可以完全不带（同时不带 `Content-Type`）。带了 `Content-Type: application/json` 却是空体时，axum 返回 400 纯文本。

- 201 `Bro` `{"id","name","dmGroupId"}`。`id` 是 UUID。
- 400「请填写名称」/「名字太长」。

```sh
curl -s -X POST $B/api/bros -H 'content-type: application/json' -d '{"name":"bob"}'
# {"id":"<botId>","name":"bob","dmGroupId":"<groupId>"}
```

### GET /api/bros

列出所有 bot（包括只通过 `/members` 加进来的），按新建顺序。`host/src/bros.rs:list_bros`

- 200 `{"bros":[Bro]}`。没有私聊的 `dmGroupId` 为 `null`。

### PATCH /api/bros/{id}

改名，私聊的群名跟着改。`host/src/bros.rs:rename_bro`

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `name` | string | 是 | 规则同 `POST /api/bros`。 |

- 200 `Bro`。
- 400「请填写名称」/「名字太长」。
- 404「bro 不存在」。

```sh
curl -s -X PATCH $B/api/bros/<botId> -H 'content-type: application/json' -d '{"name":"Bob"}'
```

### POST /api/bros/{id}/dm

取这个 bro 的私聊，没有就建。无请求体。`host/src/bros.rs:open_dm`

- 201 `Bro`：新建了私聊。
- 200 `Bro`：已经有。
- 404「bro 不存在」。

```sh
curl -s -X POST $B/api/bros/<botId>/dm
```

---

## 消息和 /say

### POST /api/groups/{id}/say

客户端输入框发的每一句话都走这里。先存一条 `text` 消息，再决定要不要派任务，两者在同一个事务里。`host/src/say.rs:say`

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `body` | string | 是 | 存的是去掉首尾空白后的内容。 |

派不派任务（`robroz_contract::route_mention`）：

- 私聊：整句派给那个 bro，prompt 是整句。
- 群：只有以 `@名字` 或 `＠名字` 开头，名字是群成员的（全局）名字，且名字后面紧跟空白或句子结束时才派。多个名字都能匹配时取最长的（`@bro 2 …` 派给 `bro 2`）。prompt 是 `@名字` 后面去掉首尾空白的部分；为空就不派。
- 其他情况不直接派任务，但群里照样轮次（见下）。

**群轮次**（`host/src/turns.rs`）：群里（不是私聊）的每一条 `/say`，群里每个 bro 都轮一次，可以选择不说话。

- 顺序：被 `@` 点名的 bro 先（就是上面那个任务，跟以前完全一样：prompt 去掉 `@名字`，状态卡马上出现，在 `Said.task` 里返回），其余成员按入群顺序。没点名时 `Said.task` 仍是 `null`，所有成员按入群顺序轮。
- 一个接一个：前面所有轮次的任务都离开 `queued` / `running`（变成 `done` 或 `blocked`）后，宿主机才给下一个 bro 建任务。它的 prompt 里有原消息和前面 bro 交付的结果（每条最多 2000 字），并告诉它没话说就整条回复只写一行 `robroz-silent`（`robroz_host::turns::SILENT_REPLY`）。引用的内容去掉了所有以 `robroz-` 开头的行，所以别人的话不会替它触发对外写入、进度或沉默。
- 没点名的轮次在模型回复之前是**隐藏**的：不出状态卡、不算未读、不算 `lastActivityAt`。模型回复之后：
  - 回复是 `robroz-silent`（只有这一行，`robroz-progress:` 行不算）或空：任务直接 `done`，一直隐藏。客户端什么都看不到：没有消息、没有状态卡、没有交付卡、没有未读。
  - 模型调用失败，或没设置模型：同样当作不说话，不出「模型没回」卡（只写日志）。
  - 其他回复（包括要批 MCP 工具）：任务从这时起出现，状态卡的 `createdAt` 是这一刻，之后跟普通任务一样（进沙箱、交付卡、审批卡都照旧）。
  - 测试开关 `ROBROZ_MODEL=echo` 下每个轮次都会说话（回显 prompt）。
- **Bros waking bros** (`robroz_host::turns::open_bro_round`). A person's `/say` opens a round at depth `PERSON_DEPTH` (0). When a bro's task on a group turn (the bro named with `@name`, or a bro that chose to speak on its turn) delivers a clean reply, the host opens a follow-on round at the parent round's depth + 1 for the other bros. A clean reply is one with no `failure` or `needsSetup` that is not silence.
  - The speaker never gets a turn in the round it opened. A bro whose turn in the parent round has not started yet is also left out, because that turn already quotes the reply.
  - A leading `@name` of another member in the reply moves that bro to the front. The rest follow in join order. The named bro still takes a normal turn (hidden until it speaks). This is never a follow-up, and no task card appears up front.
  - Turns work as above: one at a time, quoting earlier replies, hidden until the bro speaks, and waiting while the bro is busy. The prompt names the speaking bro and tells the bro to reply `robroz-silent` unless it is asked directly, needs to correct something, or has something essential to add.
  - Cap: `robroz_host::turns::MAX_BRO_DEPTH` (2). A reply in a depth-2 round is delivered but opens nothing, so one person message leads to at most two bro-opened levels.
  - Nothing else opens a round: not `POST /api/groups/{id}/messages`, not `POST /api/groups/{id}/tasks` or schedules (their tasks have no turn), not silence, not failed deliveries. DMs never have rounds.
  - No contract change. Bro-opened rounds post no `message` events; their replies show up as normal status and delivery cards.
- 两条消息的轮次各走各的，互不等待。轮到某个 bro 时它已经不在群里，就跳过它。
- 轮到某个 bro 时，它在这个群里还有别的任务是 `queued` / `running`（自己的任务，或别的轮次），这一轮就等那个任务离开 `queued` / `running`（变成 `done` 或 `blocked`）再建，同一个 bro 不会同时在群的卷上跑两个任务。它后面的 bro 照旧排在它后面。

**追问**（`host/src/follow_ups.rs`）：私聊里的一句话，或群里 `@名字` 点名的一句话，要派给的那个 bro 在这个群（私聊）里还有没结束的任务（`queued` / `running` / `blocked`，不算隐藏的轮次任务）时，这条消息挂到它最新的那个任务上，不新建任务，也不出新的状态卡。

- 算追问的只有：私聊里那个 bro，或群里以 `@名字` 开头点名的 bro，自己在这里有没结束的任务。群里没点名的消息、点别的 bro 都不算。`done` 的任务不算没结束，这时照旧新建任务。
- 追问的内容（去掉 `@名字` 后的部分）在这个任务的**下一步**被看到：
  - `queued`：马上要跑的这一步的 prompt 末尾加上「追问：」和内容。
  - `running`：当前这一步照原样跑完、照常交付。要是成功写出结果，同一个任务回到 `queued` 再跑一步：prompt 里加上上一步的回复（去掉 `robroz-` 开头的行）和追问。所以一张状态卡会有两张交付卡。
  - `blocked`：还是等审批卡，追问不是 `resume`。主人批了以后，下一步（或批准的 MCP 调用那一步跑完之后的那一步）会看到追问；主人拒了，任务照常结束，追问没人答。
  - 这一步失败（「没写出结果」、出网超额等）：任务照常 `done` + `failure`，不重新排队，追问没人答。
- 追问在群里照样开一轮：被点名的 bro 这一轮就是它挂上去的那个任务，不再给它建新的隐藏轮次；其余 bro 等这个任务离开 `queued` / `running` 后按入群顺序轮，跟普通点名一样。
- 不出新卡、不新增未读；这条消息本身照常是一条 `message`。

返回：

- 201 `Said` `{"message":Message,"task":Task|null,"appendedToTaskId"?:string}`。`task` 只是被点名的那个任务；其余轮次的任务不在这里返回。是追问时 `task` 是挂上去的那个任务（`state` 是它现在的状态，`prompt` 是它原来的 prompt），并带上 `appendedToTaskId`（就是这个任务的 id）；不是追问时没有这个字段。

- 400「请填写内容」：空白。
- 404「群不存在」。

```sh
curl -s -X POST $B/api/groups/<groupId>/say -H 'content-type: application/json' \
  -d '{"body":"@ada 写一句欢迎语"}'
# {"message":{"id":"…","kind":"text","body":"@ada 写一句欢迎语","createdAt":"…"},
#  "task":{"id":"…","assigneeBotId":"bot-ada","prompt":"写一句欢迎语","state":"queued"}}
```

### POST /api/groups/{id}/messages

只存一条消息，不派任务，不写文件。`host/src/lib.rs:post_message`

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `kind` | `"text"` \| `"image"` \| `"file"` | 是 | 图片和文件这一期只存名字。 |
| `body` | string | 是 | 去掉首尾空白后不能为空。 |

- 201 `Message` `{"id","kind","body","createdAt"}`。
- 400「kind 只能是 text、image 或 file」：`kind` 不认识。
- 400「请填写内容」。
- 404「群不存在」。
- 422「请求格式不正确」（自定义）：缺字段或类型不对。

```sh
curl -s -X POST $B/api/groups/<groupId>/messages -H 'content-type: application/json' \
  -d '{"kind":"file","body":"报价.pdf"}'
```

---

## 任务和 pump

### POST /api/groups/{id}/tasks

不经过 `@`，直接给群里的 bro 派任务。`host/src/lib.rs:create_task`

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `assigneeBotId` | string | 是 | 必须是群成员。 |
| `prompt` | string | 是 | 去掉首尾空白后不能为空。 |

- 201 `Task` `{"id","assigneeBotId","prompt","state":"queued"}`。
- 400「请填写内容」（先检查，群不存在时也是这个）。
- 404「群不存在」。
- 400「这个 Bot 不在群里」。

```sh
curl -s -X POST $B/api/groups/<groupId>/tasks -H 'content-type: application/json' \
  -d '{"assigneeBotId":"bot-ada","prompt":"写一句欢迎语"}'
```

### POST /api/tasks/{id}/resume

人接管以后恢复一个 `blocked` 的任务。无请求体。`host/src/lib.rs:resume_task`

- 200 `{"id":"<taskId>","state":"queued"}`：把库里记下的阻塞行哈希追加到 `ROBROZ_CONTROL/{taskId}/takeover`，清掉卡住记录，任务回到 `queued`。之后由 pump 在原来的群目录上重新起容器，跳过这一行。如果这一行还有一张没回答的任务审批卡，它同时被记成 `approved`。
- 404「任务不存在」。
- 409「这次不能恢复」：任务不是 `blocked`（包括 `done`，以及 `failure` 为「没写出结果」的 `done`）。
- 409「请先回答审批卡」：任务是因为 MCP 工具调用而 `blocked`。这种卡住只能用下面的审批卡批准或拒绝，不能 `resume`。
- 500「恢复失败，找不到卡住记录」：库里没有阻塞行记录。任务保持 `blocked`。
- 500「恢复失败，无法写入接管标记」：写控制目录失败。任务保持 `blocked`。

```sh
curl -s -X POST $B/api/tasks/<taskId>/resume
```

### pump 做什么

pump 不是接口，是宿主机里每秒跑一次的线程（`host/src/main.rs:main` → `Host::tick_once`、`Host::pump_once` 和 `Host::watchdog_once`，实现在 `host/src/pump.rs` 和 `host/src/watchdog.rs`）。任务的状态变化都由它写回，客户端通过事件看到。模型调用和它发起的工具调用（包括 MCP 和联网搜索）在每个任务自己的线程上跑（`host/src/steps.rs`），pump 线程不等它们，接着起别的容器、收别的结果；回答好了，下一轮 pump 再起这个任务的容器。

1. `tick`：到点的例行任务新建一个 `queued` 任务（`briefing` 为 true）。上一次还在排队或在跑就跳过这一次，不补跑。
2. 取一个 `queued` 任务改成 `running`。接了模型时先调模型（带记忆和已启用技能、可用工具，以及该群已启用且 `enabled` 的 MCP 工具），把回答写进 `/control/reply.txt`。模型要调用需要批准的 MCP 工具时先变成 `blocked`，发出 `action` 为 `call` 的审批卡，这一步先不写 `reply.txt`、也不起容器；批准后再从暂停处继续。
3. `docker run`，容器 `--network none`，只挂群目录 `/workspace` 和只读的 `/control`。起不来就改回 `queued`。
4. 容器退出后：
   - 写出 `result.txt`：`done`，发一张 delivery。
   - prompt 里碰到没接管过的 `robroz-action:external`：`blocked`，宿主机发一张任务审批卡（`waitingFor:"approval"`）。
   - 碰到 `robroz-action:captcha`：`blocked`，等人接管（`waitingFor:"takeover"`）。
   - 出网超额或用了非 GET：`done` 带 `failure`，没有 delivery。
   - 非 0 退出或没写出结果、容器中途消失：`done`，`failure` 为「没写出结果」，没有 delivery，不重排队，不再起容器。写库是比较后写入（只在任务仍是 `running` 时生效）。
5. 成功完成的任务，读它的 `proposals.jsonl`，每条记忆或技能提议发一张确认卡。
6. Watchdog（`host/src/watchdog.rs`）：
   - `running` 且 `containerUp` false 的任务（旧版宿主机留下的运行失败）同样结束成 `done` + `failure`「没写出结果」，不重排队。
   - 模型和工具调用跑过 `ROBROZ_STEP_TIMEOUT_SECS`（默认 600 秒）：打日志，`progress` 写「模型和工具还没回，已等 N 分钟」。
   - 容器在跑但 `progress` 超过 `ROBROZ_STALL_SECS`（默认 900 秒）没变：打日志，`progress` 写「N 分钟没有新进度」，runner 写出新进度后换回新进度。
   - 这两条只提示，不改状态、不停容器、不重排队。`blocked` 是在等人，不算卡住。

完整的权限规则见 [permissions.md](permissions.md)，容器和模型调用的细节见 [architecture.md](architecture.md)。

---

## 事件和卡片

### GET /api/groups/{id}/events

群里的所有卡片。`host/src/lib.rs:list_events`，组装在 `host/src/lib.rs:group_events`。

| 查询参数 | 必填 | 说明 |
| --- | --- | --- |
| `since` | 否 | RFC 3339。只返回在这个时间**之后变过**的卡。空值等于不带。 |

- 200 `{"events":[Event]}`。按 `createdAt` 升序，同一时间再按 `id`。带 `since` 而没有变化时是 `{"events":[]}`。
- 400「since 不是合法时间」。
- 404「群不存在」。

「变过」的时间：状态卡用 `updatedAt`；选项卡、审批卡、确认卡用 `answeredAt`，没回答时用 `createdAt`；消息和交付卡用 `createdAt`。

一张卡变化后用**同一个 `id`** 再出现，客户端按 `id` 原地替换。

```sh
curl -s "$B/api/groups/<groupId>/events?since=2026-11-21T02:35:00Z"
```

### 事件类型

`Event` 按 `type` 区分（`robroz_contract::Event`）。下表「一定有」的键总会出现，值可能是 `null`；「可选」的键只在有值时出现。

| `type` | 一定有 | 可选 | 什么时候出现 |
| --- | --- | --- | --- |
| `message` | `id` `kind` `body` `createdAt` | — | `/say` 或 `/messages` 存的消息。 |
| `status` | `id` `taskId` `botId` `botName` `state` `label` `createdAt` `updatedAt` `containerUp` | `failure` `waitingFor` `progress` | 每个任务恰好一张，`id` 等于 `taskId`。 |
| `delivery` | `id` `summary` `path` `briefing` `createdAt` | `failure` `needsSetup` | 任务写出结果时一张。`summary` 通常是「已写入结果」；MCP 工具审批被拒绝时是「未执行」。`path` 是群目录里的相对路径 `tasks/{taskId}/result.txt`。`briefing` 为 true 表示例行任务的结果（简报）。 |
| `choice` | `id` `botId` `botName` `prompt` `options` `multi` `choice` `createdAt` `answeredAt` `answeredBy` | — | 选项卡。`options` 是 `[{"id","label","primary"}]`。`choice` 未回答时 `null`，回答后是选中的 `id` 数组。 |
| `approval` | `id` `botId` `botName` `action` `summary` `path` `decision` `createdAt` `answeredAt` `answeredBy` | `taskId` `mcpServer` `mcpTool` `reason` `expiresAt` | 审批卡。`action` 是 `delete` / `send` / `spend` / `call` / `rule_add` / `rule_delete`（后两个是放行规则的确认卡，`summary` 是规则原文）；客户端遇到不认识的值会解析成通用卡，不会整条失败。`decision` 是 `approved` / `denied` 或 `null`。宿主机为任务发的卡带 `taskId`。MCP 写工具的卡带 `mcpServer`、`mcpTool`，没有这两个键时是其它审批卡。`reason` 是被拦或已结算的原因，过期时固定为契约常量 `APPROVAL_EXPIRED_REASON`（「审批已过期」）；`expiresAt` 是过期时间（RFC 3339，格式同 `createdAt`）。两者都是可选字段，没有时省略，旧 JSON 不变。宿主机在任务审批卡、MCP 工具审批卡、bro 发的审批卡和放行规则确认卡上填 `expiresAt`；自动审核拦下的卡带 `reason`（审核模型给的一句话理由）。确认卡（`confirm`）和接管不过期。 |
| `confirm` | `id` `botId` `botName` `subject` `taskId` `decision` `createdAt` `answeredAt` `answeredBy` | 记忆：`memoryId` `kind` `text` `scope`；技能：`slug` `title` `instruction` `scope` | 确认卡，`subject` 是 `memory` 或 `skill`。显示的内容是这条记忆 / 技能**现在**的样子。 |

### 任务状态和标签

`state` 冻住，只有四种。`label` 来自 `robroz_card::status_line(state, containerUp)`：

| `state` | `containerUp` | `label` |
| --- | --- | --- |
| `queued` | false | 排队 |
| `running` | true | 进行 |
| `running` | false | 没写出结果（只出现在旧数据里：宿主机现在把运行失败直接结束成 `done` + `failure`「没写出结果」，Watchdog 也会把剩下的这种任务这样结束） |
| `blocked` | false | 卡住 |
| `done` | false | 完成（带 `failure` 时客户端显示成「没做成：原因」；`failure` 为「没写出结果」时原样显示「没写出结果」） |

### failure

状态卡只在 `done` 时可能带；有交付卡的情况下，交付卡带同一个值。任务再次开跑时清掉。值是原文：

| 情况 | `failure` | 有没有交付卡 | 代码 |
| --- | --- | --- | --- |
| 模型调用失败 | 「模型没回：<原因>」 | 有，交付卡也带同样的 `failure` | `host/src/pump.rs:write_reply`、`host/src/tools.rs` |
| 任务审批卡被拒绝 | 「你拒绝了：<prompt 第一行普通内容，最多 60 字>」 | 没有 | `host/src/approvals.rs:settle_task` |
| MCP 工具审批卡被拒绝 | 「你拒绝了：{server}.{tool}」 | 有，`summary` 为「未执行」，交付卡带同一个 `failure` | `host/src/mcp_tools.rs:finish_denied` |
| 任务审批卡或 MCP 工具审批卡过期 | 「审批已过期」（`APPROVAL_EXPIRED_REASON`） | 任务审批卡没有；MCP 卡有，`summary` 为「未执行」 | `host/src/review/mod.rs:expire_due` |
| 出网超额 | 「出网超额：已用 N 次，上限 M」 | 没有 | `crates/robroz-policy/src/lib.rs`（`OverQuota::failure`） |
| 出网用了非 GET | 「出网只允许 GET：用了 <方法>」 | 没有 | 同上 |
| 工具调用超过 8 轮 | 「工具调用超过 8 轮」 | 有，交付卡也带同样的 `failure`，结果里也写这句 | `host/src/tools.rs:ROUND_LIMIT_FAILURE` |
| 运行失败（容器非 0 退出、没写出结果、中途消失） | 「没写出结果」 | 没有 | `host/src/pump.rs`（`Outcome::Dead`）、`host/src/watchdog.rs`（`DEAD_RUN_FAILURE`） |

### waitingFor

只在 `blocked` 时带：

- `approval`：这个任务有一张**没回答**、且对应它当前阻塞行的审批卡（`robroz-action:external`，或 MCP 写工具的 `call` 卡）。回答那张卡即可：同意后重新排队并继续，拒绝后变成带 `failure` 的 `done`。
- `takeover`：没有这样的审批卡（例如 `robroz-action:captcha`），要人接管后调 `POST /api/tasks/{id}/resume`。MCP 写工具不能靠 `resume` 放行。

### progress

只在 `running` 且 `containerUp` 为 true 时带，或任务因 MCP 审批而 `blocked` 且库里还存着进度行时带，原地更新在状态卡上（`host/src/progress.rs`）：

- 任务在群里等一张发到主人私聊的审批卡时（见「审批卡发到哪」），这行固定是契约常量 `APPROVAL_SENT_TO_OWNER_DM_NOTICE`（「已发到主人私聊」），盖过库里存的进度行；卡回答、过期或还在自动审核时不显示。
- 宿主机自己报：开跑时「开始干活」（接了模型时是「在想怎么回」），容器起来后「在沙箱里跑」，调用工具时「在用 <工具名>」。MCP 工具用展示名 `{server}.{tool}`。等审批时这行仍显示。
- bro 报：prompt 或模型回复里一行 `robroz-progress:<文字>`，取最后一行。
- Watchdog 也可能写「模型和工具还没回，已等 N 分钟」或「N 分钟没有新进度」。只提示，不改状态，见上面「pump 做什么」。
- 规范成一行：连续空白变一个空格，去掉首尾空白，最多 60 个字符（超了截到 59 个加「…」）。文字没变不更新 `updatedAt`。

### needsSetup

交付卡的可选字段。现在有两种值，和 `failure` 互不依赖：

- `"search"`：任务调用了 `web_search` 但还没设置联网搜索（`web_fetch` 不会记）。一般情况下任务照常写出结果，只带 `needsSetup` 不带 `failure`。
- `"model"`：还没设置模型（也没有打开测试开关 `ROBROZ_MODEL=echo`）。结果文件是「还没设置模型」，不把 prompt 原样写成结果。

客户端应按这个字段引导用户去填对应的设置，不要匹配文字。搜索的客户端做法见 [client.md 的「联网搜索页」](client.md#联网搜索页)；模型见 [client.md 的「模型页」](client.md#模型页)。

---

## 实时推送

### GET /api/groups/{id}/stream

Server-Sent Events，`text/event-stream`。`host/src/stream.rs:stream_events`

| 输入 | 说明 |
| --- | --- |
| 请求头 `Last-Event-ID` | 断线重连用，只补推这个时间之后的变化。和 `since` 都带时以它为准。 |
| 查询参数 `since` | 同上。 |

- 每条推送：`event:` 是事件的 `type`，`id:` 是这张卡的变化时间，`data:` 是事件 JSON（和 `/events` 里的一样）。
- 连上时先推一遍（带了 `since` 就只推之后的），之后约每 250 毫秒查一次库，有变化才推；没变化时每 15 秒一条保活注释。
- 同一张卡再变化时用同一个 `data.id` 再推。不同类型的事件之间没有约定先后，按 `id` 去重、原地替换即可。
- 400「since 不是合法时间」；404「群不存在」。
- 轮询 `/events?since=` 是兜底。

```sh
curl -s -N $B/api/groups/<groupId>/stream
# event: status
# id: 2026-11-21T02:36:10.421Z
# data: {"type":"status","id":"…","state":"blocked","label":"卡住","waitingFor":"approval",…}
```

---

## 未读

每个群一个已读位置，单用户。`host/src/unread.rs`

`Unread`：`{"groupId","readAt","unread","lastActivityAt"}`。`readAt` 从没读过时为 `null`；`lastActivityAt` 群里没有任何活动时为 `null`。

`unread` 只数 `readAt` 之后 bro 发来、需要人看的东西：交付卡、选项卡、审批卡、确认卡（按 `createdAt`），以及 `updatedAt` 在之后、当前处于 `blocked` 的任务。还没说话的群轮次任务（见 `/say` 的「群轮次」）不算，也不算进 `lastActivityAt`。人自己的消息和排队、进行、完成这类状态变化不算。审批卡只算出了卡的（审核中和被自动审核放行的不算），而且只在卡所在的会话算：bro 在群里发、改发到主人私聊的卡只算私聊的未读，群里列着的那张提示不算。

`lastActivityAt` 取消息、任务（`updatedAt`）、交付卡、选项卡、审批卡和确认卡（已回答按 `answeredAt`）里最新的时间。改发到私聊的 bro 卡在原来的群里也算一次活动（群里列着它），但不算未读。

### GET /api/unread

- 200 `{"groups":[Unread]}`，包括私聊。按 `lastActivityAt` 从新到旧；没有活动的群排最后，按建群顺序。

### GET /api/groups/{id}/unread

- 200 `Unread`；404「群不存在」。

### POST /api/groups/{id}/read

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `readAt` | string（RFC 3339） | 否 | 不给就用当前时间。已读位置只往后走，更早的值被忽略。 |

请求体可以不带（同时不带 `Content-Type`）。

- 200 更新后的 `Unread`。
- 400「readAt 不是合法时间」。
- 404「群不存在」。

```sh
curl -s -X POST $B/api/groups/<groupId>/read -H 'content-type: application/json' -d '{}'
```

---

## 审批卡和选项卡

### POST /api/groups/{id}/choices

群里的 bro 问人在固定选项里选。`host/src/choices.rs:create_choice`

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `botId` | string | 是 | 必须是群成员。 |
| `prompt` | string | 是 | 去掉首尾空白后不能为空。 |
| `options` | `[{"id":string,"label":string,"primary"?:bool}]` | 是 | 2 到 4 个；`id`、`label` 去掉空白后非空；`id` 不重复；`primary` 最多一个。多选卡上 `primary` 一律变成 false。 |
| `multi` | bool | 否 | 默认 false。 |

- 201 `choice` 事件。
- 404「群不存在」；400「这个 Bot 不在群里」；400「请填写内容」。
- 400「选项需要 2 到 4 个」/「推荐项最多一个」/「选项不能为空」/「选项重复」。

```sh
curl -s -X POST $B/api/groups/<groupId>/choices -H 'content-type: application/json' \
  -d '{"botId":"bot-ada","prompt":"用哪个？","options":[{"id":"a","label":"A","primary":true},{"id":"b","label":"B"}]}'
```

### POST /api/choices/{id}/answer

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `choice` | string[] | 是 | 单选恰好一个；多选一个以上、不重复。保留请求顺序。 |
| `answeredBy` | string | 否 | 见下面的 `answeredBy`。 |

- 200 同一张 `choice` 卡，`choice`、`answeredAt`、`answeredBy` 已填。
- 404「选项卡不存在」。
- 409「已经选过了」：每张只能答一次。
- 400「只能选一个」/「至少选一个」/「选项重复」/「没有这个选项」。
- 400「answeredBy 只能一行，最多 40 个字」。

```sh
curl -s -X POST $B/api/choices/<choiceId>/answer -H 'content-type: application/json' \
  -d '{"choice":["a"],"answeredBy":"书房电脑"}'
```

### POST /api/groups/{id}/approvals

bro 在删除、发送或花钱之前问人。`host/src/approvals.rs:create_approval`

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `botId` | string | 是 | 必须是群成员。 |
| `action` | `"delete"` \| `"send"` \| `"spend"` | 是 | 不认识的值返回 400「不支持这个动作」。`call`、`rule_add`、`rule_delete` 只能由宿主机发出，用这个接口传会 400「这个动作只能由宿主机发出」。 |
| `summary` | string | 是 | 去掉首尾空白后非空，一行，最多 200 字。 |
| `path` | string | 否 | 只用来展示，去掉空白后为空当没填。 |

宿主机先自动审核（见「自动审核和过期」），审核在这个请求里做完才返回，所以有放行规则时可能要等审核模型几秒。201 的 body 永远是审核后的卡，形状和以前一样（不带 `taskId`）：

- 没有放行规则：`decision: null`，带 `expiresAt`，不带 `reason`。卡发到主人私聊（见「审批卡发到哪」）。
- 规则放行（allow）：`decision: "approved"`、`answeredAt` 已填、`answeredBy` 为 `null`、`reason` 是审核模型的理由，不带 `expiresAt`。不出卡（不进任何群或私聊的事件和未读），bro 可以直接做。之后 `GET /api/approvals/{id}` 能读到它，`approve` / `deny` 返回 409「已经答过了」。
- 拦下（block，包括审核出错重试后失败、没接模型）：`decision: null`，带 `reason` 和 `expiresAt`，卡发到主人私聊。
- bro 只看 `decision`：`approved` 就做，`null` 就等主人回答（读 `GET /api/approvals/{id}` 或 stream），`denied` 就不做。旧 bro 照旧把 201 当「卡已发出」也不会出错，只是放行的卡它会白等一张不会出现的卡——所以 bro 应先看 `decision`。

- 404「群不存在」；400「这个 Bot 不在群里」。
- 400「请填写要做什么」/「说明只能一行，最多 200 字」/「不支持这个动作」。

```sh
curl -s -X POST $B/api/groups/<groupId>/approvals -H 'content-type: application/json' \
  -d '{"botId":"bot-ada","action":"delete","summary":"删缓存","path":"target/"}'
```

### 审批卡发到哪

任务审批卡和 MCP 工具审批卡发到**主人和这个任务的 bro 的私聊**（`host/src/approvals.rs:card_group`），不进任务所在的群：

- 群里的状态卡照旧 `blocked` + `waitingFor:"approval"`，`progress` 是「已发到主人私聊」（`APPROVAL_SENT_TO_OWNER_DM_NOTICE`），没有按钮。旧客户端已经会把 `progress` 显示在「等你批准」下面，不用改客户端。
- 任务本来就在私聊里跑：卡留在这个私聊，不显示这行提示。
- bro 还没有私聊（用 `POST /api/groups/{id}/members` 直接按 `botId` 加进群的 bro）：宿主机先给它建一个私聊，和 `POST /api/bros/{id}/dm` 一样；找不到这个 bro 时卡留在任务所在的群。
- 同意、拒绝、过期、409 都不变；同意后任务在原来的群里重新排队、在原来的群目录跑完，交付卡也在群里。
- bro 用 `POST /api/groups/{id}/approvals` 自己发的卡（`delete` / `send` / `spend`）也走自动审核；被拦或没有放行规则时，卡发到主人和这个 bro 的私聊（`host/src/approvals.rs:bro_card_group`，没有私聊就先建一个），主人在私聊里点同意或拒绝。发卡的那个群的事件和 stream 里**也列着同一张卡**（同一个 `id`，内容一样，回答后两边一起变）。客户端在群里本来就把待批准的审批卡画成没有按钮、只写「已发到主人私聊」（#120），所以不用改客户端。这张卡只在私聊里算一条未读；群里列着的那张只是提示（跟任务卡的状态卡一样），不算群的未读，回答前后都不算。bro 在私聊里发的卡留在这个私聊，不另列。

### 任务审批卡

任务的 prompt 里有没接管过的 `robroz-action:external` 行时，宿主机替执行者发一张审批卡（`host/src/approvals.rs:gate_task`）：`action` 为 `send`，`summary` 为「要对外写入：<prompt 第一行普通内容，最多 60 字>」，带 `taskId`，发到主人私聊（见上），状态卡 `waitingFor` 为 `approval`。

- 同意：写接管标记，任务重新排队，跳过这一行跑完。
- 拒绝：任务变成 `done`，`failure`「你拒绝了：…」，没有交付卡。
- 任务已经通过 `/resume` 被接管走了：回答只记下，不再动任务。
- 自动审核和过期见下面「自动审核和过期」。

### MCP 工具审批卡

模型要调用未标 `readOnlyHint: true` 的 MCP 工具时，宿主机替执行者发一张审批卡（`host/src/approvals.rs:gate_mcp_task`）：`action` 为 `call`，`summary` 为「调用 {server}.{tool}」加上参数摘要（超过 `APPROVAL_SUMMARY_MAX` 会截断），带 `taskId`、`mcpServer`、`mcpTool`，发到主人私聊（见上），状态卡 `waitingFor` 为 `approval`，进度行是「已发到主人私聊」（卡在同一个私聊里时是「在用 {server}.{tool}」）。

- 同意：任务回到 `queued`，由 pump 执行这次调用，把结果交回模型继续；这次调用计一次出网 GET。写继续标记失败时 500「批准失败，无法写入继续标记」，卡保持未回答。
- 拒绝：不执行，任务变成 `done`，交付卡 `summary` 为「未执行」，`failure` 为「你拒绝了：{server}.{tool}」。
- 不要用 `POST /api/tasks/{id}/resume` 代替答卡（409「请先回答审批卡」）。
- 自动审核和过期同任务审批卡，见下面「自动审核和过期」。
- 只读 MCP 工具不出卡。行为说明见 [search-and-mcp.md](search-and-mcp.md#模型怎么调用)。

### 自动审核和过期

`host/src/review/`。设计见 [design/auto-review.md](design/auto-review.md)。

- **没有放行规则**：任务审批卡和 MCP 工具审批卡照旧直接出卡，不调审核模型，不带 `reason`，带 `expiresAt`。
- **有放行规则**：卡先不出（不在事件里，`approve` / `deny` 返回 404）。宿主机的审核线程把全部已确认规则的原文和这次操作交给审核模型，模型回 allow 或 block 加一句理由。不用正则或通配。
  - allow：不出卡，跟手动同意一样写标记、任务重新排队。库里记下 `decision` 为 `approved`、理由，不带 `answeredBy`。
  - block：出卡，带 `reason`（理由）和 `expiresAt`，`createdAt` 改成出卡时间。
  - 审核出错或超时：重试一次，还不行就 block，`reason`「自动审核没有成功，需要你批准」。
  - 没接模型：block，`reason`「还没设置模型，没法自动审核」。从不默认放行。
- **过期**：卡在 `expiresAt`（出卡后 `ROBROZ_APPROVAL_TTL_SECS` 秒，默认 24 小时）还没回答，宿主机把它记成 `decision: denied`、`reason: "审批已过期"`（`APPROVAL_EXPIRED_REASON`），`answeredBy` 留空。它挡着的任务变成 `done`，`failure`「审批已过期」。之后再 `approve` / `deny` 返回 409「已经答过了」。确认卡（记忆、技能）和接管（captcha）不过期。

任务审批卡、MCP 工具审批卡和 bro 发的卡都走上面的审核，被拦的卡发到主人私聊，见「审批卡发到哪」。bro 发的卡不经过审核线程，而是在 `POST /api/groups/{id}/approvals` 请求里当场审核（锁外调模型）；请求中途断掉、卡还在审核的，审核线程过 60 秒接手。被规则放行的卡（任务卡也一样）`GET /api/approvals/{id}` 能读到，`approve` / `deny` 返回 409「已经答过了」；还在审核的卡两处都是 404。未读只算出了卡的审批卡，审核中和被放行的不算；改发到私聊的 bro 卡只算私聊的未读，见「未读」。

### GET /api/approvals/{id}

读一张审批卡，返回和事件里一样的 `approval` 对象。`host/src/approvals.rs:get_approval`。设置页的放行规则确认卡只能从这里取到。

- 200 `approval` 对象。
- 404「审批卡不存在」：id 不存在、是确认卡（`confirm`），或这张卡还在自动审核。被自动审核放行的卡（没出过卡）也能读到，`decision` 为 `approved`、带 `reason`。

```sh
curl -s $B/api/approvals/<approvalId>
```

### POST /api/approvals/{id}/approve 和 /deny

回答审批卡，也用来回答确认卡（见「确认卡」）。`host/src/approvals.rs:approve`、`deny`，共同逻辑在 `decide`。

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `answeredBy` | string | 否 | 请求体整个可以不带，也可以是 `{}`。 |

- 200 同一张卡，`decision` 为 `approved` / `denied`，`answeredAt`、`answeredBy` 已填。
- 404「审批卡不存在」：id 不存在（确认卡也一样），或这张卡还在自动审核。
- 409「已经答过了」：每张只能答一次；已过期的卡也是这句。
- 同意 `rule_add` 卡时加上这条规则，同意 `rule_delete` 卡时删掉那条规则；拒绝或过期时规则列表不变。
- 400「answeredBy 只能一行，最多 40 个字」。
- 422「请求格式不正确」（自定义）：请求体不是合法 JSON。
- 500「批准失败，无法写入接管标记」：同意任务审批卡时写控制目录失败，卡保持未回答。
- 500「批准失败，无法写入继续标记」：同意 MCP 工具审批卡时写继续标记失败，卡保持未回答。

```sh
curl -s -X POST $B/api/approvals/<approvalId>/approve
curl -s -X POST $B/api/approvals/<approvalId>/deny -H 'content-type: application/json' -d '{"answeredBy":"书房电脑"}'
```

### answeredBy

选项卡、审批卡、确认卡共用。宿主机只有一个共享令牌，分不清是谁，所以存的是客户端自己报的设备名或人名，不做核实。去掉首尾空白，空值当没填；只能一行，最多 40 个字（`robroz_contract::clean_answered_by`）。没填时事件里是 `null`。

---

## 例行任务（定时）

cron 由宿主机按 **UTC** 计算（`croner`）。接受 5 段 `分 时 日 月 周` 或 6 段 `秒 分 时 日 月 周`；不接受年份字段、`@daily` 这类别名和永远不会触发的表达式（`host/src/schedule.rs:parse_cron`）。到点时新建普通任务，完成时交付卡 `briefing` 为 true。

### POST /api/schedules

`host/src/lib.rs:create_schedule`

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `groupId` | string | 是 | |
| `assigneeBotId` | string | 是 | 必须是群成员。 |
| `cron` | string | 是 | 见上。 |
| `prompt` | string | 是 | 去掉首尾空白后非空。 |

- 201 `Schedule` `{"id","groupId","assigneeBotId","cron","prompt","nextRunAt","lastTaskId"}`，`lastTaskId` 第一次运行前是 `null`。不含 `paused`。
- 404「群不存在」；400「这个 Bot 不在群里」；400「请填写内容」；400「cron 格式不对」。

```sh
curl -s -X POST $B/api/schedules -H 'content-type: application/json' \
  -d '{"groupId":"<groupId>","assigneeBotId":"bot-ada","cron":"0 9 * * 1","prompt":"写周报"}'
# nextRunAt 是 UTC：每周一 09:00 UTC
```

### GET /api/groups/{id}/schedules

`host/src/routines.rs:list_schedules`

- 200 `{"schedules":[ScheduleEntry]}`，按创建先后。`ScheduleEntry` 是 `Schedule` 的字段平铺再加 `paused`（bool）。
- 404「群不存在」。

### PATCH /api/schedules/{id}

`host/src/routines.rs:update_schedule`

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `paused` | bool | 是 | |

- 200 `ScheduleEntry`。暂停只阻止以后再建任务，已经排队或在跑的不受影响；重复暂停没有变化。恢复时 `nextRunAt` 推到当前时间之后的下一次，错过的不补跑。
- 404「例行任务不存在」。

```sh
curl -s -X PATCH $B/api/schedules/<scheduleId> -H 'content-type: application/json' -d '{"paused":true}'
```

---

## 记忆

`Memory`：`{"id","scope","kind","text","source","status","createdAt","updatedAt","confirmId"?}`。

| 字段 | 取值 |
| --- | --- |
| `scope` | `user`（所有 bro）或 `bot:<botId>`（只给这个 bro） |
| `kind` | `profile` / `habit` / `project` |
| `source` | `user`（人写的）/ `bot`（bro 提的） |
| `status` | `pending`（等确认）/ `kept`（在用）/ `rejected`（拒绝过，留记录不用） |
| `confirmId` | 可选。`pending` 且确认卡没回答时是那张卡的 id |

忘掉是软删，忘掉的永远不返回。代码在 `host/src/memories.rs`。

### GET /api/memories

| 查询参数 | 说明 |
| --- | --- |
| `kind` | 可选，筛类型。 |
| `scope` | 可选，只检查格式。 |
| `status` | 可选。不带时只返回 `kept` 和 `pending`。 |

空值等于不筛，不认识的参数忽略。

- 200 `{"memories":[Memory]}`，按 `updatedAt` 从新到旧。
- 400「kind 只能是 profile、habit 或 project」/「scope 只能是 user 或 bot:<botId>」/「status 只能是 pending、kept 或 rejected」。

```sh
curl -s "$B/api/memories?status=pending"
```

### POST /api/memories

人自己记一条，`source` 为 `user`，`status` 直接 `kept`。请求体格式错误也返回 `{"error"}`（自定义，400）。

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `scope` | string | 是 | `user` 或 `bot:<botId>`，bot 必须存在。 |
| `kind` | string | 是 | |
| `text` | string | 是 | 去掉首尾空白，非空，最多 2000 个字符。 |

- 201 `Memory`。
- 400「请填写内容」/「内容太长，最多 2000 个字」。
- 400「scope 只能是 user 或 bot:<botId>」/「这个 bro 不存在」。
- 400「kind 只能是 profile、habit 或 project」/「请求格式不正确」。

```sh
curl -s -X POST $B/api/memories -H 'content-type: application/json' \
  -d '{"scope":"user","kind":"habit","text":"周报用中文"}'
```

### PATCH /api/memories/{id}

只改给了的字段：`text`、`kind`、`scope`（规则同上）。`status` 不能改，带了也忽略。

- 200 `Memory`。
- 400「没有要改的内容」：三个都没给。
- 404「记忆不存在」：不存在或已忘掉。
- 其余 400 同 `POST`。

### DELETE /api/memories/{id}

- 204：忘掉；重复忘掉也是 204，不改第一次的时间。还在 `pending` 的，它那张没回答的确认卡同时记成 `denied`，`answeredBy` 为 `null`。
- 404「记忆不存在」：从没有过。

### 注入模型

接了模型时，宿主机把 `scope` 为 `user` 或 `bot:<执行者>`、`status=kept`、没忘掉的记忆拼进 system 提示：一段「你记得这些：」，每条一行 `[kind] 原文`，按 `updatedAt` 从新到旧，整段最多 4000 个字符，超了整条丢掉最旧的（`host/src/memories.rs:prompt_section`）。

---

## 技能

`Skill`：`{"id","scope","slug","title","instruction","source","status","createdAt","updatedAt","confirmId"?}`。`scope`、`source`、`status`、`confirmId` 同记忆。代码在 `host/src/skills.rs`。

- `slug`：小写字母、数字和单个连字符，不能以连字符开头或结尾，最多 64 个字符（`robroz_contract::parse_skill_slug`）。是技能的身份，不能改。
- `title`：最多 80 个字符；`instruction`：最多 8000 个字符。都去掉首尾空白，非空。
- 技能只是提示文字，不开放任何权限。

### GET /api/skills

查询参数 `scope`、`status`，规则同记忆。

- 200 `{"skills":[Skill]}`，按 `updatedAt` 从新到旧。
- 400「scope 只能是 user 或 bot:<botId>」/「status 只能是 pending、kept 或 rejected」。

### POST /api/skills

请求体 `{"scope","slug","title","instruction"}`，全部必填。格式错误返回 400「请求格式不正确」（自定义）。

- 201 `Skill`（`source` 为 `user`，`status` 为 `kept`）。
- 400「slug 只能是小写字母、数字和连字符」/「请填写标题」/「标题太长，最多 80 个字」/「请填写说明」/「说明太长，最多 8000 个字」。
- 400「scope 只能是 user 或 bot:<botId>」/「这个 bro 不存在」。
- 409「这个 slug 已经有了」：同 `slug` 的技能没忘掉。

```sh
curl -s -X POST $B/api/skills -H 'content-type: application/json' \
  -d '{"scope":"user","slug":"weekly-report","title":"写周报","instruction":"先列三点"}'
```

### PATCH /api/skills/{slug}

只改给了的字段：`title`、`instruction`、`scope`。

- 200 `Skill`。
- 400「没有要改的内容」；其余 400 同 `POST`。
- 404「技能不存在」：不存在、已忘掉或 `slug` 格式不对。

### DELETE /api/skills/{slug}

- 204：忘掉，同时从所有群关掉；重复忘掉也是 204。`pending` 的确认卡同时记成 `denied`。忘掉后同一个 `slug` 可以再写。
- 404「技能不存在」。

### GET /api/groups/{id}/skills

群里已启用、没忘掉的技能（任何 `status`）。

- 200 `{"skills":[Skill]}`，按 `updatedAt` 从新到旧。
- 404「群不存在」。

### PUT /api/groups/{id}/skills/{slug}

在群里启用。无请求体。

- 204；已启用再调也是 204。
- 404「群不存在」/「技能不存在」。

### DELETE /api/groups/{id}/skills/{slug}

在群里关掉。

- 204；本来没启用也是 204。
- 404「群不存在」/「技能不存在」。

```sh
curl -s -X PUT $B/api/groups/<groupId>/skills/weekly-report
```

### 注入模型

接了模型时，执行者在这个群里能用的技能（**已在该群启用**、`scope` 为 `user` 或 `bot:<执行者>`、`status=kept`、没忘掉）拼成一段「你会这些技能：」，每条 `### 标题 (slug)` 加说明，加在记忆段后面，整段最多 8000 个字符，超了整条丢掉最旧的（`host/src/skills.rs:kept_for`、`prompt_section`）。

---

## 确认卡

bro 不能直接写记忆或技能，只能提议，人在确认卡上决定。代码在 `host/src/proposals.rs`。

- **怎么提**：prompt 或模型回复里一行 `robroz-memory:{"kind":"habit","text":"周报用中文","scope":"user"}`，或 `robroz-skill:{"slug":"weekly-report","title":"写周报","instruction":"先列三点","scope":"user"}`。`scope` 可省，默认 `bot:<执行者>`。runner 把它们写进群目录 `tasks/{taskId}/proposals.jsonl`，这些行不进 `result.txt`。
- **什么时候收**：任务 `done` 且没有 `failure` 时，宿主机读完这个文件就删。每条存成 `source=bot`、`status=pending` 的记忆或技能，并发一张 `confirm` 卡。
- **不收的**：JSON 不对、`type` 不认识、字段不合法、`scope` 不对或 bro 不存在；同 `scope`、`kind`、文字（连续空白算一个）的记忆已存在且没忘掉（任何状态）；同 `slug` 的技能已存在且没忘掉。一个任务最多收 20 条（`PROPOSALS_PER_TASK_MAX`）。
- **回答**：`POST /api/approvals/{confirmId}/approve`（变成 `kept`）或 `/deny`（变成 `rejected`），请求体、`answeredBy`、409「已经答过了」、404「审批卡不存在」都和审批卡相同。这是改 `status` 的唯一途径。
- 确认卡和审批卡存在同一张表，但只以 `confirm` 事件出现，不影响状态卡的 `waitingFor`。新卡算一条未读。
- `pending` 时用 `PATCH` 改了记忆或技能，确认卡会带着新内容再推一次。

```sh
curl -s -X POST $B/api/approvals/<confirmId>/approve -H 'content-type: application/json' -d '{"answeredBy":"书房电脑"}'
```

---

## 搜索设置

模型可用的 `web_search` / `web_fetch` 工具用的搜索服务。设置存在 SQLite 的 `settings` 表，和环境变量的优先级见 [configuration.md 的「搜索」](configuration.md#搜索)：**设置接口优先，按字段退回环境变量**。代码在 `host/src/search.rs`。

`SearchSettings`：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `provider` | `"brave"` \| `"searxng"` \| null | 没生效时 `null`。 |
| `url` | string \| null | Brave 用默认地址时为 `null`。 |
| `hasKey` | bool | 是否有密钥。密钥本身永不返回。 |
| `keyLast4` | string \| null | 密钥末 4 位。 |
| `source` | `"settings"` \| `"env"` | 可选。生效时才有：设置接口里填过任意一项为 `settings`，全部来自环境变量为 `env`。 |

### GET /api/settings/search

- 200 `SearchSettings`。

```sh
curl -s $B/api/settings/search
# 未设置：{"provider":null,"url":null,"hasKey":false,"keyLast4":null}
```

### PUT /api/settings/search

三个字段都可选：**不传**表示不改，传 `null` 表示清掉设置接口里的这一项（之后退回环境变量），传字符串表示设置。

| 请求字段 | 类型 | 说明 |
| --- | --- | --- |
| `provider` | string \| null | `brave` 或 `searxng`，不分大小写。 |
| `url` | string \| null | 必须以 `http://` 或 `https://` 开头。 |
| `key` | string \| null | 非空，不能有空白。 |

- 200 更新后的 `SearchSettings`。
- 400「请求体不是 JSON」/「请求体无效」（自定义）。
- 400「provider 只能是 brave 或 searxng」/「请填写地址」/「地址要以 http:// 或 https:// 开头」/「密钥不能为空」/「密钥里不能有空白」。
- 400「请填写 SearXNG 地址」：合并后 provider 是 `searxng` 却没有地址。

```sh
curl -s -X PUT $B/api/settings/search -H 'content-type: application/json' \
  -d '{"provider":"brave","key":"<你的 Brave 密钥>"}'
# {"provider":"brave","url":null,"hasKey":true,"keyLast4":"…","source":"settings"}
```

### POST /api/settings/search/test

用当前生效的配置真的搜一次 `robroz`。无请求体。总是 200。

- `{"ok":true}`。
- `{"ok":false,"error":"还没设置联网搜索"}`：没生效，或 provider 是 `brave` 却没有密钥。
- `{"ok":false,"error":"连不上：<原因>"}`：原因里的密钥会被替换成 `***`。

```sh
curl -s -X POST $B/api/settings/search/test
```

---

## 模型设置

模型和搜索一样，可以用环境变量，也可以用设置接口。设置接口优先，逐个字段比较。只支持 OpenAI 兼容的 `/chat/completions`，后端不维护服务商列表。密钥只以 `hasKey` 和后 4 位返回，永不回显。改完立即生效，不用重启宿主机。详见 [configuration.md 的「模型」](configuration.md#模型)。

### GET /api/settings/model

读当前生效的模型配置。`host/src/model_settings.rs:get_model`

没配的字段是 `null`。`source` 在设置接口填过任意一项时是 `"settings"`，三项都没填、全部来自环境变量时是 `"env"`，什么都没配时不出现这个字段。

```sh
curl -s $B/api/settings/model
```

```json
{"baseUrl":"https://api.openai.com/v1","model":"gpt-4o-mini","hasKey":true,"keyLast4":"9f3a","source":"env"}
```

### PUT /api/settings/model

改设置接口里的模型配置。`host/src/model_settings.rs:put_model`

请求体是 JSON 对象：`{baseUrl?, model?, key?}`。某个字段不传表示不改；传 `null` 表示清除该项，之后退回环境变量。`baseUrl` 必须以 `http://` 或 `https://` 开头，末尾的 `/` 会去掉。成功时返回和 `GET` 相同的对象。非法的 `baseUrl` 或空值返回 400。

```sh
curl -s -X PUT $B/api/settings/model -H 'content-type: application/json' \
  -d '{"baseUrl":"https://api.openai.com/v1","model":"gpt-4o-mini","key":"<你的密钥>"}'
```

### POST /api/settings/model/test

用当前生效的配置发一条最短的 `/chat/completions` 请求试连。无请求体。总是 200。超时 10 秒。`host/src/model_settings.rs:test_model`

- `{"ok":true}`。
- `{"ok":false,"error":"还没设置模型"}`：地址、模型名没有配齐。密钥可选。
- `{"ok":false,"error":"密钥不对或没填"}`：服务商返回 HTTP 401 或 403。此句是契约常量 `MODEL_TEST_BAD_KEY`，前端按 `error == MODEL_TEST_BAD_KEY` 判断。
- `{"ok":false,"error":"连不上：<原因>"}`：其他失败。原因里的密钥会被替换成 `***`。

```sh
curl -s -X POST $B/api/settings/model/test
```

### 放行规则

自动审核的放行规则，每条是主人写的一句话。`host/src/review/rules.rs`。鉴权同其他 `/api/settings/*`。`POST` 和 `DELETE` 都不直接改列表：只建一张确认卡（`approval` 对象，`action` 为 `rule_add` / `rule_delete`，`summary` 为规则原文，带 `expiresAt`）。这张卡就显示在设置页里，不发到任何群或私聊，也不算未读：`botId` 是契约常量 `SETTINGS_APPROVAL_BOT_ID`（`"settings"`），`botName` 是 `SETTINGS_APPROVAL_BOT_NAME`（「设置」），没有 `taskId`。没有任何 bro 也能用。客户端用返回的 `approvalId` 调 `GET /api/approvals/{id}` 取卡，用 `POST /api/approvals/{id}/approve` / `deny` 回答；同意后才生效，拒绝或过期（24 小时）就作废。bro 调这两个接口也只能建卡。

#### GET /api/settings/review-rules

只列已确认的规则，按加入时间排。

```sh
curl -s $B/api/settings/review-rules
# {"rules":[{"id":"<ruleId>","text":"允许合并 CI 全绿的 PR","createdAt":"2026-11-24T10:00:00.000Z"}]}
```

#### POST /api/settings/review-rules

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `text` | string | 是 | 去掉首尾空白后非空，一行，最多 200 字（`REVIEW_RULE_MAX`）。 |

- 202 `{"approvalId":"<approvalId>"}`：确认卡已发出，规则还没加。
- 400「请填写规则」/「规则只能一行，最多 200 字」。

```sh
curl -s -X POST $B/api/settings/review-rules -H 'content-type: application/json' -d '{"text":"允许合并 CI 全绿的 PR"}'
```

#### DELETE /api/settings/review-rules/{id}

- 202 `{"approvalId":"<approvalId>"}`：确认卡已发出，规则在同意前仍在列表里。
- 404「规则不存在」：id 不是已确认的规则。

```sh
curl -s -X DELETE $B/api/settings/review-rules/<ruleId>
```

---

## MCP 服务器配置

宿主机作为 MCP 客户端，支持 stdio（本机进程）和 Streamable HTTP。配置在 `host/src/mcp.rs`，连接在 `crates/robroz-mcp`，连接超时 10 秒。任务开始时，会把该群已启用且 `enabled` 的服务器上的工具注册进模型循环（`host/src/mcp_tools.rs`），见下面的「MCP 工具调用」。

stdio 服务器会在宿主机上以宿主机用户身份启动 `command`。只填可信的命令。未批准前，`/test`、`/tools` 以及模型循环里的注册都不会启动进程；先调用 `POST /api/mcp/servers/{id}/approve-command` 批准当前的 `command` 和 `args`。http 服务器不需要这项批准。

`McpServer`：

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | string | |
| `name` | string | 唯一，一行，最多 80 个字符。 |
| `transport` | `"stdio"` \| `"http"` | |
| `command` | string | 可选，stdio 才有。 |
| `args` | string[] | 可选，stdio 才有。 |
| `url` | string | 可选，http 才有。 |
| `secrets` | `[{"name","in","hasKey","keyLast4"?}]` | `in` 是 `env`（stdio 的环境变量）或 `header`（HTTP 请求头）。密钥值永不返回。 |
| `enabled` | bool | |
| `toolCount` | number | 可选，最近一次成功连接看到的工具数。 |
| `createdAt` / `updatedAt` | string | |
| `commandApproved` | bool | 可选，stdio 才有。当前 `command` 和 `args` 是否已批准。新建和迁移后的已有服务器都是 `false`。http 服务器不返回这个字段。 |
| `commandApprovedAt` | string | 可选，已批准时才有，是批准时间。 |

密钥输入 `McpSecretInput`：`{"name","in"?,"key"?}`。`name` 非空、无空白、最多 64 个字符；`in` 不给时 stdio 默认 `env`、http 默认 `header`；`key` 不传表示不改（只在 `PATCH` 里有意义），`null` 表示删掉这条密钥，字符串表示设置。

请求体格式错误一律 400「请求格式不正确」（自定义）。

### GET /api/mcp/servers

- 200 `{"servers":[McpServer]}`，按 `updatedAt` 从新到旧。

### POST /api/mcp/servers

| 请求字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `name` | string | 是 | |
| `transport` | string | 是 | `stdio` 或 `http`。 |
| `command` | string | stdio 必填 | |
| `args` | string[] | 否 | |
| `url` | string | http 必填 | `http://` 或 `https://` 开头。 |
| `secrets` | McpSecretInput[] | 否 | 新增的密钥必须给 `key`。 |
| `enabled` | bool | 否 | 默认 true。 |

- 201 `McpServer`。
- 409「这个名字已经有了」。
- 400「请填写名称」/「名称最多 80 个字」/「名称只能一行」/「transport 只能是 stdio 或 http」/「请填写启动命令」/「启动命令无效」/「参数无效」/「请填写地址」/「地址要以 http:// 或 https:// 开头」。
- 400「请填写密钥名称」/「密钥名称最多 64 个字」/「密钥名称里不能有空白」/「新增密钥时请填写值」/「密钥不能为空」。

```sh
curl -s -X POST $B/api/mcp/servers -H 'content-type: application/json' \
  -d '{"name":"docs","transport":"http","url":"https://<MCP 服务器地址>/mcp","secrets":[{"name":"Authorization","key":"Bearer <你的令牌>"}]}'
```

### PATCH /api/mcp/servers/{id}

字段同 `POST`，都可选，只改给了的。改 `transport` 会清掉另一种传输的字段；`command`、`url` 可传 `null`。改完后 stdio 必须有 `command`、http 必须有 `url`。如果改完后的 `command` 或 `args` 和批准时记下的不一致（包括改成另一种传输），`commandApproved` 自动变回 `false`，需要再批一次。

- 200 `McpServer`。
- 404「MCP 服务器不存在」；409「这个名字已经有了」；400 同 `POST`。

### DELETE /api/mcp/servers/{id}

- 204，同时从所有群关掉。
- 404「MCP 服务器不存在」。

### GET /api/mcp/servers/{id}/tools

实时连一次，列出工具，成功时更新 `toolCount`。stdio 服务器必须先批准启动命令，否则不启动进程。

- 200 `{"tools":[{"name","description"?,"needsApproval"}]}`。`needsApproval` 只有工具标注了 `readOnlyHint: true` 时为 false，否则 true。
- 404「MCP 服务器不存在」。
- 409 `{"error":"command_not_approved","command","args"}`：stdio 命令尚未批准。`command` 和 `args` 是当前配置。
- 502 `{"error":"<原因>"}`：连不上，例如「连不上 MCP 服务器：…」。原因里的密钥会被替换成 `***`。

### POST /api/mcp/servers/{id}/test

- 200 `{"ok":true,"toolCount":N}` 或 `{"ok":false,"error":"<原因>"}`。
- 404「MCP 服务器不存在」。
- 409 `{"error":"command_not_approved","command","args"}`：与 `GET …/tools` 相同，未批准时不启动进程。

### POST /api/mcp/servers/{id}/approve-command

把这台 stdio 服务器当前的 `command` 和 `args` 标为已批准，并记下批准时的指纹和 `commandApprovedAt`。无请求体。重复批准仍是 200，会更新批准时间。

- 200 `McpServer`，`commandApproved` 为 true。
- 404「MCP 服务器不存在」。
- 400「只有 stdio 服务器需要批准启动命令」/「请填写启动命令」。

```sh
curl -s -X POST $B/api/mcp/servers/<serverId>/approve-command
```

### GET /api/groups/{id}/mcp

- 200 `{"servers":[McpServer]}`：这个群启用的服务器（包括 `enabled` 为 false 的）。
- 404「群不存在」。

### GET /api/groups/{id}/mcp/{serverId}

- 200 `McpServer`。
- 404「群不存在」/「MCP 服务器不存在」/「这个群还没启用这台服务器」。

### PUT /api/groups/{id}/mcp/{serverId} 和 DELETE

在群里启用 / 关掉。无请求体。

- 204；重复调用也是 204。
- 404「群不存在」/「MCP 服务器不存在」。

```sh
curl -s -X PUT $B/api/groups/<groupId>/mcp/<serverId>
```

### MCP 工具调用

任务开始时，宿主机把该群已启用、并且服务器本身也是 `enabled` 的 MCP 工具注册进这一次模型循环（`host/src/mcp_tools.rs:register_tools`）。某台服务器连接失败就跳过，任务照常进行。stdio 服务器还没批准启动命令的，同样跳过，不会启动进程。群里没启用或服务器被关掉的，不注册。

- 交给模型的名字是 `{safe_server}__{safe_tool}`（只保留字母、数字、`_`、`-`，最长 64 个字符；重名时加短哈希）。进度行和审批卡仍用 `{server}.{tool}`。
- `readOnlyHint: true`：按一次出网 GET 计次，直接调用，不出审批卡。
- 其余：先出上面的 MCP 工具审批卡；批准后执行（同样计一次出网 GET）并把结果交回模型；拒绝则不执行。
- 调用出错时模型拿到「调用失败：{原因}」，任务不因此失败。
- 工具结果和其它宿主工具一样，超过 16000 个字符会截断，末尾加上「…（已截断）」。
- 契约只加了可选字段和新类型：`ApprovalAction::Call`（JSON `"call"`），以及 `Event::Approval` 上可选的 `mcpServer`、`mcpTool`。现有字段和任务状态不变。

行为说明见 [search-and-mcp.md](search-and-mcp.md#模型怎么调用) 和 [permissions.md](permissions.md#mcp-工具)。

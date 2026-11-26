# 联网搜索和 MCP

宿主机替 bro 访问外部的两条路：联网搜索（`web_search` / `web_fetch`）和 MCP 服务器。都在宿主机上调用，沙箱仍然不联网。接口字段和报错见 [api.md 的「搜索设置」](api.md#search-settings)、[「MCP 服务器配置」](api.md#mcp-server-settings)和 [「MCP 工具调用」](api.md#mcp-tool-calls)，环境变量见 [configuration.md 的「搜索」](configuration.md#search)。

## 联网搜索

宿主机给模型提供两个工具（`host/src/search.rs`）：

| 工具 | 做什么 |
| --- | --- |
| `web_search` | 用选好的服务搜索公开网页。 |
| `web_fetch` | 用 GET 读一个页面，转成纯文本。只允许 http / https，拒绝解析到内网、回环和链路本地地址的目标。结果最多 16000 个字符，超出截断加 `…(truncated)`。 |

- 权限归计数（Metered）一级：和其他出网一样按任务计次，上限 `ROBROZ_GET_LIMIT`。超额时任务变成带 `failure`「出网超额：已用 N 次，上限 M」的 `done`，没有交付卡。见 [permissions.md](permissions.md#permission-tiers)。
- 服务由 robroz 用户自己选、自己填，密钥不必交给别人：[Brave Search](https://brave.com/search/api/) 或自建 [SearXNG](https://docs.searxng.org/)。
- 还没设置时 `web_search` 仍可调用，返回「还没设置联网搜索」（`host/src/search.rs:NOT_CONFIGURED`），并给任务记上 `needsSetup: "search"`。这句话会加在结果文件开头，交付卡带可选字段 `needsSetup: "search"`。`web_fetch` 不用搜索服务，不会记这个字段。客户端按这个字段给出「去设置」，不匹配文字：只要带这个字段就会显示按钮，不要求同时有 `failure`；没有 `failure` 时路径下面再加一行灰字「这次没联网搜索：还没设置」。见 [client.md 的「联网搜索页」](https://github.com/PandaQuQ/robroz-client/blob/main/docs/client.md#web-search-page)。
- 设置存在本机 SQLite 的 `settings` 表。密钥不进日志、不进报错，接口只返回 `hasKey` 和 `keyLast4`。

### 设置和环境变量谁优先

设置接口优先，按 provider、地址、密钥三项逐项退回环境变量；改设置不用重启宿主机。`GET` / `PUT` 的返回体在已生效时带可选字段 `source`：设置接口里填过任意一项是 `settings`，全部来自环境变量是 `env`，完全没生效时没有这个字段。详见 [configuration.md](configuration.md#search)。

### 怎么填搜索服务

三种方式选一种：客户端的「联网搜索」页（见 [client.md](https://github.com/PandaQuQ/robroz-client/blob/main/docs/client.md#web-search-page)，背后就是下面的设置接口）、设置接口、环境变量。

**用设置接口**（宿主机设了令牌时加 `-H "Authorization: Bearer <你的令牌>"`）：

```sh
# Brave：在 Brave Search API 后台拿订阅密钥
curl -X PUT http://127.0.0.1:8790/api/settings/search \
  -H 'Content-Type: application/json' \
  -d '{"provider":"brave","key":"<你的 Brave 密钥>"}'

# SearXNG：填实例地址，不用密钥
curl -X PUT http://127.0.0.1:8790/api/settings/search \
  -H 'Content-Type: application/json' \
  -d '{"provider":"searxng","url":"https://<你的 SearXNG 地址>"}'

# 试连：真的发一次查询，超时 10 秒。返回 {"ok":true}，或 {"ok":false,"error":"原因"}
curl -X POST http://127.0.0.1:8790/api/settings/search/test

# 只换密钥就只传 key；传 null 清掉设置接口里的密钥（之后退回环境变量）
curl -X PUT http://127.0.0.1:8790/api/settings/search \
  -H 'Content-Type: application/json' -d '{"key":null}'
```

`PUT` 成功返回的就是 `GET` 的内容，只有 `hasKey` 和 `keyLast4`。不合法的值返回 400，原文见 api.md。通过设置接口填的地址必须以 `http://` 或 `https://` 开头。

**用环境变量**：启动宿主机前设 `ROBROZ_SEARCH_PROVIDER=brave` 加 `ROBROZ_BRAVE_KEY`，或 `ROBROZ_SEARCH_PROVIDER=searxng` 加 `ROBROZ_SEARXNG_URL`。环境变量里的地址启动时**不检查格式**，写错了要等试连或搜索时才发现。

注意：

- 密钥只放在宿主机本机或你自己的设置里，不要提交进仓库，也不要发到群里。
- 宿主机请求 SearXNG 的 `/search?format=json`（地址末尾写不写 `/search` 都行）。实例要在 `settings.yml` 的 `search.formats` 里打开 `json`，否则试连会失败。
- 搜索和 `web_fetch` 都算进每个任务的出网次数。

## MCP

宿主机作为 MCP 客户端，可以接本机用 stdio 启动的服务器，或用 Streamable HTTP 访问的服务器。沙箱不直接连这些服务器。代码：配置在 `host/src/mcp.rs`，连接在 `crates/robroz-mcp`（stdio 连接池在 `crates/robroz-mcp/src/pool.rs` 和 `managed.rs`），超时 10 秒；把工具接进模型循环在 `host/src/mcp_tools.rs`、`host/src/tools.rs` 和 `host/src/pump.rs`。

- 每台服务器名字唯一，传输方式是 `stdio`（填 `command` 和 `args`）或 `http`（填 `url`）。
- 可以存多条密钥：stdio 作为环境变量传给进程，HTTP 作为请求头。接口只返回 `{name, in, hasKey, keyLast4}`，不回显密钥。`PATCH` 时某条密钥的 `key` 不传表示不改，传 `null` 表示**删掉这一整条密钥**（连名字一起）。密钥不进日志、不进报错。
- `GET /api/mcp/servers/{id}/tools` 实时连一次，每项带 `name`、`description`、`needsApproval`：工具标了 `readOnlyHint: true` 时为 false，否则为 true。stdio 服务器必须先经 `POST /api/mcp/servers/{id}/approve-command` 批准当前的 `command` 和 `args`，否则不启动进程，返回 409 `{"error":"command_not_approved","command","args"}`；`POST …/test` 同样。服务器列表项可带 `toolCount`（最近一次成功连接看到的工具数）和 `commandApproved`（stdio 才有）。
- 按群启用：`PUT` / `DELETE /api/groups/{id}/mcp/{serverId}`，`GET /api/groups/{id}/mcp` 列出这个群启用的服务器。
- 配置存在本机 SQLite 的 `mcp_servers` 和 `group_mcp` 表。

### 模型怎么调用

任务开始时，宿主机把「该群已启用、并且服务器本身也是 `enabled`」的工具注册进这一次的 `ToolRegistry`。某台服务器连接失败就跳过它，任务照常进行。stdio 服务器还没批准启动命令的，同样跳过，不会启动进程。群里没启用、或服务器被关掉（`enabled` 为 false）的，不会注册。

交给模型的名字是 `{safe_server}__{safe_tool}`：只保留字母、数字、`_`、`-`，最长 64 个字符；重名时加短哈希。进度行和审批卡仍用原来的 `{server}.{tool}`，例如「在用 notes.write_note」。映射写在 `host/src/mcp_tools.rs`。

权限跟 [permissions.md](permissions.md#permission-tiers) 对齐：

- `readOnlyHint: true`：计数（Metered），按一次出网 GET 计次，直接调用，不出审批卡。
- 其余：接管（Takeover），先出审批卡（`action` 为 `call`），卡上带可选字段 `mcpServer`、`mcpTool` 和参数摘要，以及 `taskId`。批准、拒绝走现有的 `POST /api/approvals/{id}/approve` 和 `/deny`，答过一次再答是 409。等待期间任务是 `blocked`（`waitingFor` 为 `approval`），不加新状态。不要用 `POST /api/tasks/{id}/resume` 代替答卡，那样会 409「请先回答审批卡」。
- 批准后任务回到 `queued`，接着执行这次调用，把结果交回模型继续；这次调用也计一次出网 GET。拒绝则不执行，任务 `done`，交付卡 `summary` 为「未执行」，带 `failure`「你拒绝了：{server}.{tool}」，不是成功交付。
- 工具结果和其它宿主工具一样，超过 16000 个字符会截断，末尾加上 `…(truncated)`。
- 模型循环里的 stdio 服务器走连接池，见下面的「stdio 连接池」：进程在任务之间保留，不再每个任务、每次调用都重新启动。HTTP 服务器不进池，每次列工具和每次调用都单独连一次，超时 10 秒（`robroz_mcp::MCP_TIMEOUT`）。还没批准启动命令的服务器在注册时跳过，不会启动进程。
- 密钥：`hasKey` 为 true 的每条密钥在列工具和调用时都会带上（stdio 作环境变量，HTTP 作请求头）；没填值的那条不带。`keyLast4` 只用于显示，不影响调用。
- 调用出错不会让任务失败：模型拿到「调用失败：{原因}」接着回答，原因里的密钥会被抹掉。
- `needsApproval`（`GET …/tools` 的返回）和调用时的分级用的是同一条规则：没标 `readOnlyHint: true` 就要批准。
- 客户端里的 MCP 页（增删改、试连、工具列表、按群启用、批准 stdio 命令）见 [client.md 的「MCP 页」](https://github.com/PandaQuQ/robroz-client/blob/main/docs/client.md#mcp-page)。对 `call` 审批卡的专门显示还没合入 `main`。

安全：

- **stdio 启动的是宿主机上的本机进程**，以宿主机用户身份运行。只能由用户通过这些接口配置，模型不能增删服务器。不要填来历不明的命令。
- stdio 的 `command` 和 `args` 必须先经 `POST /api/mcp/servers/{id}/approve-command` 批准，才会启动进程。新建和迁移后的已有服务器都是未批准。`PATCH` 改了 `command` 或 `args`（或改成另一种传输）后自动变回未批准，需要再批一次。http 服务器不需要这项批准。
- 未批准时，`POST …/test`、`GET …/tools` 都不启动进程，返回 409 `{"error":"command_not_approved","command","args"}`。模型循环注册工具时跳过未批准的 stdio 服务器。批准之后这些路径才会真正启动命令（例如已批准的 `npx -y <包名>` 仍会下载并运行这个包），所以只批准可信的命令。
- 客户端收到这个 409 时，按返回体里的 `command` 和 `args` 展开批准区（等宽字，一个参数一行），「批准运行」才调 `approve-command`，之后自动重新试连。客户端不会在没有用户点击的情况下批准命令。

### stdio 连接池

模型循环里，已批准的 stdio 服务器由 `robroz_mcp::McpPool` 管理。池子的 key 是「服务器 id + 配置哈希 + `group_id`」：同一台服务器在每个群里各留一个进程，同一个群里的多个任务共用这一个。bro 的私聊也是一个群，所以也有自己的进程。两个群不会共用进程。

- **什么时候换进程**：配置哈希按「启动命令、参数、环境变量（含密钥的值）」计算，和进程绑在一起。`PATCH` 改了其中任何一项（包括换密钥），下一次使用时按新配置启动新进程；旧进程等手上的请求做完再停。`PATCH`（不管改了什么，包括 `enabled: false`）和 `DELETE` 成功后，宿主机会马上停掉这台服务器在所有群里的进程。以后删群时，可以按 `group_id` 停掉这个群的进程（`evict_group`，这条接口还没有）。哈希只在内存里比较，不写日志，也不写进库。
- **工具列表缓存**：同一个进程活着、配置没变时，任务开始时列工具直接用缓存，不再发 `tools/list`。换了进程（配置变了、进程退出、超时被停）就重新列一次。服务器在运行中改了工具列表（`notifications/tools/list_changed`）时，池子不会自动刷新，要等进程换掉或闲置被停。
- **健康检查和重连**：每次使用前检查进程和管道还在。子进程退出或管道断了，下一次使用时自动启动新进程。`tools/list` 在复用的进程上遇到断管，会在新进程上重试一次；`tools/call` 不重试，因为写入类工具可能已经执行过，模型拿到「调用失败：…」，下一次调用再重连。
- **超时**：启动进程加 MCP 握手 10 秒；之后每个请求 10 秒；连接忙时排队等空位也最多 10 秒。请求超时的进程会被停掉，和以前每次单独启动时一样，卡住的服务器不会一直留着。
- **并发**：每个进程同时最多处理 `ROBROZ_MCP_MAX_IN_FLIGHT` 个请求（默认 4），多出来的排队。设成 `1` 就是同一台服务器的请求一个一个来。
- **闲置回收**：超过 `ROBROZ_MCP_IDLE_SECS` 秒（默认 600，即 10 分钟）没用过、手上也没有请求的进程会被停掉，下次用时再启动。设成 `0` 关掉连接池，回到每次列工具、每次调用都启动一次进程的旧行为。见 [configuration.md](configuration.md)。
- **退出**：宿主机收到 SIGTERM（`docker stop`）或 Ctrl-C 时，先停掉池里所有进程（关 stdin，等它退出，不退再终止，最多等 5 秒）再退出。宿主机被强杀时，子进程也带着 kill-on-drop，不会残留。
- **不变的部分**：审批、计次和以前完全一样，都在调用进池子之前做完：只读工具按一次出网 GET 计次，其余工具先出审批卡，批准后才调用。密钥仍然只作为环境变量传给进程，报错里的密钥照旧抹成 `***`。
- `GET /api/mcp/servers/{id}/tools` 和 `POST …/test` 不走连接池，仍然每次实时启动一次，用来确认当前配置真的能连上。

注意：同一个群里，同一台服务器的进程会在这个群的多个任务之间共用。服务器自己在内存里存的状态（例如某些「记忆」类服务器）会被这个群后面的任务看到；以前每个任务都是新进程，看不到。不同群、以及 bro 私聊之间不会共用进程。

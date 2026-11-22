# 联网搜索和 MCP

宿主机替 bro 访问外部的两条路：联网搜索（`web_search` / `web_fetch`）和 MCP 服务器。都在宿主机上调用，沙箱仍然不联网。接口字段和报错见 [api.md 的「搜索设置」](api.md#搜索设置)和 [「MCP 服务器配置」](api.md#mcp-服务器配置)，环境变量见 [configuration.md 的「搜索」](configuration.md#搜索)。

## 联网搜索

宿主机给模型提供两个工具（`host/src/search.rs`）：

| 工具 | 做什么 |
| --- | --- |
| `web_search` | 用选好的服务搜索公开网页。 |
| `web_fetch` | 用 GET 读一个页面，转成纯文本。只允许 http / https，拒绝解析到内网、回环和链路本地地址的目标。结果最多 16000 个字符，超出截断加「…（已截断）」。 |

- 权限归计数（Metered）一级：和其他出网一样按任务计次，上限 `ROBROZ_GET_LIMIT`。超额时任务变成带 `failure`「出网超额：已用 N 次，上限 M」的 `done`，没有交付卡。见 [permissions.md](permissions.md#三级权限)。
- 服务由 robroz 用户自己选、自己填，密钥不必交给别人：[Brave Search](https://brave.com/search/api/) 或自建 [SearXNG](https://docs.searxng.org/)。
- 还没设置时 `web_search` 仍可调用，返回「还没设置联网搜索」（`host/src/search.rs:NOT_CONFIGURED`），并给任务记上 `needsSetup: "search"`。这句话会加在结果文件开头，交付卡带可选字段 `needsSetup: "search"`。`web_fetch` 不用搜索服务，不会记这个字段。客户端按这个字段给出「去设置」，不匹配文字；目前只在带 `failure` 的交付卡上显示，见 [client.md 的「联网搜索页」](client.md#联网搜索页)。
- 设置存在本机 SQLite 的 `settings` 表。密钥不进日志、不进报错，接口只返回 `hasKey` 和 `keyLast4`。

### 设置和环境变量谁优先

设置接口优先，按 provider、地址、密钥三项逐项退回环境变量；改设置不用重启宿主机。`GET` / `PUT` 的返回体在已生效时带可选字段 `source`：设置接口里填过任意一项是 `settings`，全部来自环境变量是 `env`，完全没生效时没有这个字段。详见 [configuration.md](configuration.md#搜索)。

### 怎么填搜索服务

三种方式选一种：客户端的「联网搜索」页（见 [client.md](client.md#联网搜索页)，背后就是下面的设置接口）、设置接口、环境变量。

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

宿主机作为 MCP 客户端，可以接本机用 stdio 启动的服务器，或用 Streamable HTTP 访问的服务器。沙箱不直接连这些服务器。代码：配置在 `host/src/mcp.rs`，连接在 `crates/robroz-mcp`，连接超时 10 秒。

**`main` 上只做了配置和列工具，bro 还不会调用 MCP 工具。**

- 每台服务器名字唯一，传输方式是 `stdio`（填 `command` 和 `args`）或 `http`（填 `url`）。
- 可以存多条密钥：stdio 作为环境变量传给进程，HTTP 作为请求头。接口只返回 `{name, in, hasKey, keyLast4}`，不回显密钥。`PATCH` 时某条密钥的 `key` 不传表示不改，传 `null` 表示**删掉这一整条密钥**（连名字一起）。密钥不进日志、不进报错。
- `GET /api/mcp/servers/{id}/tools` 实时连一次，每项带 `name`、`description`、`needsApproval`：工具标了 `readOnlyHint: true` 时为 false，否则为 true。服务器列表项可带 `toolCount`，是最近一次成功连接看到的工具数。
- 按群启用：`PUT` / `DELETE /api/groups/{id}/mcp/{serverId}`，`GET /api/groups/{id}/mcp` 列出这个群启用的服务器。
- 配置存在本机 SQLite 的 `mcp_servers` 和 `group_mcp` 表。

安全：

- **stdio 启动的是宿主机上的本机进程**，以宿主机用户身份运行。只能由用户通过这些接口配置，模型不能增删服务器。不要填来历不明的命令。
- `POST …/test` 和 `GET …/tools` 会**真的启动** stdio 命令（例如 `npx -y <包名>` 会去下载并运行这个包）。试之前先确认命令可信。

# 权限、审批和接管

bro 能做什么、什么时候停下来问人、人怎么接手。分级逻辑在 `crates/robroz-policy`，执行在 `host/src/pump.rs`、`host/src/approvals.rs`、`host/src/tools.rs`、`host/src/mcp_tools.rs`。接口细节见 [api.md](api.md)。

## 三级权限

`robroz_policy::tier(SideEffect)` 把每种副作用分到三级之一：

| 级别 | 副作用 | 怎么处理 |
| --- | --- | --- |
| 放行 `Allow` | 读写群目录（`WorkspaceRead` / `WorkspaceWrite`） | 不通知人。成功时写 `tasks/{taskId}/result.txt`，任务完成。 |
| 计数 `Metered` | 出网（`Outbound`），包括 `web_search` / `web_fetch`，以及标了 `readOnlyHint: true` 的 MCP 工具 | 只允许 GET，按任务计数，上限 `ROBROZ_GET_LIMIT`（默认 8）。MCP 只读工具每调用一次计一次出网 GET，直接执行，不出审批卡。用满上限仍算正常；超限或用了非 GET：删掉容器，任务 `done` 并带 `failure`，没有交付卡，不改成 `blocked`。 |
| 接管 `Takeover` | 对外写入（`ExternalWrite`，包括未标 `readOnlyHint: true` 的 MCP 工具）、验证码（`Captcha`） | 不代填凭据。任务变成 `blocked`。prompt 里的对外写入另发一张任务审批卡；MCP 写工具另发一张 `action` 为 `call` 的审批卡（见下面）；验证码要人去私聊接管，再 `resume`。 |

计数细节：出网次数记在群目录 `tasks/{taskId}/outbound.txt`，由 `robroz_policy::meter` 按任务计。第 N+1 次（N 是上限）任务结束，`failure` 为「出网超额：已用 N+1 次，上限 N」，例如默认上限时是「出网超额：已用 9 次，上限 8」；用了非 GET 是「出网只允许 GET：用了 POST」。

## 阻塞行

prompt 里的 `robroz-action:external` 和 `robroz-action:captcha` 行是阻塞行，任务跑到这里就停。任务会不会卡住只看 prompt 里有没有这种行，和模型回答了什么无关。

停在哪一行由宿主机自己算，不信容器：

1. runner 停下时在群目录写 `tasks/{taskId}/intent`。这个文件只表示「卡住了」，宿主机从不读它的内容。
2. 容器退出后，宿主机用库里存的 prompt 和 `ROBROZ_CONTROL/{taskId}/takeover` 里的哈希，按顺序找第一行 sha256 不在 takeover 里的阻塞行（`robroz_policy::blocking_line`，runner 决定在哪停用的也是这个函数）。
3. 算出来的类型、原文和哈希存进这个任务的 `intent_kind` / `intent_line` / `intent_hash` 三列，然后删掉群目录里的 `intent`（下次起容器前也会再清一次）。
4. 有 `intent` 但算不出这样一行（容器说卡住了，其实没有没接管过的阻塞行）：按运行失败处理：`done`，`failure` 为「没写出结果」，不算卡住。

接管标记 `takeover`：

- 在控制目录里，每行一个哈希，只有宿主机写。容器只拿到自己任务的那一个，只读挂在 `/control`。
- runner 只读 `/control/takeover`，跳过哈希在里面的阻塞行，跳过的行不写进 `result.txt`；碰到别的阻塞行照样卡住，再接管一次就再追加一个哈希。
- `/workspace` 里任何地方的 takeover 文件（包括 bro 自己伪造的）一律不认。
- 同一行原文在 prompt 里出现多次时，接管一次就都放行。库里存的 prompt 不改写。
- 旧数据库打开时自动加上 `intent_*` 三列，原有数据不动。

## 恢复（resume）

`POST /api/tasks/{id}/resume` 只接受 `blocked`。成功时把库里的 `intent_hash` 追加到 `takeover`，清掉卡住记录，状态回到 `queued`，`containerUp` false。resume 不读群目录，也不启动容器；之后由 pump 在原来的群目录上再起容器。容器仍然 `--network none`，不挂 docker.sock。状态码和原文报错见 [api.md 的 resume](api.md#post-apitasksidresume)。

## 审批卡

### bro 自己发

`POST /api/groups/{id}/approvals`，`action` 是 `delete`、`send` 或 `spend`。只是问人，回答不会自动让宿主机做任何事。字段和报错见 [api.md](api.md#post-apigroupsidapprovals)。

### 任务自动发

任务跑到 `robroz-action:external` 时，宿主机替它发一张审批卡，不再等人去私聊接管：

- `action` 是 `send`，`summary` 是「要对外写入：要做的事」，`botId` 是任务的执行者，带可选字段 `taskId`（bro 自己发的卡不带）。
- 发卡时任务是 `blocked`，`waitingFor` 为 `approval`，算一条未读。
- 同意：宿主机替你记下接管（追加哈希），任务重新排队，接着跑完。
- 拒绝：任务直接 `done`，不执行那一步，没有交付卡，状态卡带 `failure`「你拒绝了：…」。
- 任务已经通过 `/resume` 被接管走了，再回答这张卡只记下回答，不再动任务。
- 验证码（`robroz-action:captcha`）不发审批卡，因为得有人去填，还是走「卡住，去私聊接管」。

### MCP 工具

模型要调用未标 `readOnlyHint: true` 的 MCP 工具时，宿主机先出一张审批卡（`host/src/mcp_tools.rs`、`host/src/approvals.rs:gate_mcp_task`），不执行这次调用：

- `action` 是 `call`，`summary` 是「调用 {server}.{tool}」加上参数摘要，带可选字段 `mcpServer`、`mcpTool` 和 `taskId`。
- 发卡时任务是 `blocked`，`waitingFor` 为 `approval`，进度行是「在用 {server}.{tool}」。
- 同意：任务回到 `queued`，由 pump 接着跑：执行这次调用，把结果交回模型继续。这次调用同样计一次出网 GET，和只读工具共用 `ROBROZ_GET_LIMIT`。
- 拒绝：不执行，任务 `done`，交付卡 `summary` 为「未执行」，带 `failure`「你拒绝了：{server}.{tool}」。
- 答过一次再答是 409。不要用 `POST /api/tasks/{id}/resume` 代替答卡，那样会 409「请先回答审批卡」。
- 只读 MCP 工具不出卡，直接调用，按一次出网 GET 计次。
- 调用出错（连不上、超时、服务器报错）不会让任务失败：模型拿到「调用失败：{原因}」接着回答，原因里的密钥会被抹掉。

详见 [search-and-mcp.md](search-and-mcp.md#模型怎么调用)。

### 处理人 answeredBy

宿主机只有一个共享令牌，分不清是谁在操作。`answeredBy` 存的是客户端回答时自己报的设备名或人名，宿主机不核实。审批卡、选项卡、确认卡都有这个字段，事件里一定出现，没填是 `null`。规则见 [api.md 的 answeredBy](api.md#answeredby)，客户端怎么显示见 [client.md](client.md#处理人)。

## 等什么（waitingFor）

只在 `blocked` 时带：

- `approval`：有一张没回答、且对应当前阻塞行的任务审批卡（`robroz-action:external`，或 MCP 写工具的 `call` 卡）。回答它即可。
- `takeover`：没有这样的审批卡（比如验证码），要人去私聊接管，再 `resume`。MCP 写工具不能靠 `resume` 放行。

`waitingFor` 只改这张状态卡怎么显示，不另出卡。

## 没做成（failure）

`done` 但活没做成时，状态卡带 `failure`；有交付卡的情况下交付卡带同一个值。正常结果不带这个键。任务再次开跑时清掉。

| 情况 | `failure` | 交付卡 |
| --- | --- | --- |
| 模型调用失败 | 「模型没回：原因」，结果里也写这句 | 有 |
| 任务审批卡被拒绝 | 「你拒绝了：…」 | 没有 |
| MCP 工具审批卡被拒绝 | 「你拒绝了：{server}.{tool}」 | 有，`summary` 为「未执行」，交付卡带同一个 `failure` |
| 出网超额 | 「出网超额：已用 N 次，上限 M」 | 没有 |
| 出网用了非 GET | 「出网只允许 GET：用了 POST」 | 没有 |
| 工具调用超过 8 轮 | 「工具调用超过 8 轮」，结果里也写这句 | 有 |
| 运行失败（容器非 0 退出、没写出结果、中途消失） | 「没写出结果」 | 没有 |

客户端看到 `failure` 就显示灰色「没做成：原因」，不显示绿色的完成，没有按钮。唯一例外是运行失败：`failure` 为「没写出结果」时原样显示「没写出结果」（`robroz_card::failure_line`）。

## 模型的工具调用

宿主机调用模型时可以注册工具（`host/src/tools.rs`），工具也按上面的三级判：

- 判为 `Takeover` 的 MCP 工具先出审批卡，等批准再执行；其它仍是 `Takeover` 的宿主工具还不执行。
- 一次任务最多向模型请求 8 次；第 8 次仍是工具调用时任务 `done`，`failure` 为「工具调用超过 8 轮」。
- 单次工具结果最多 16000 个字符，超出截断并加「…（已截断）」。
- 没注册任何工具时请求里不带 `tools`，行为和不接工具一样。

MCP 工具的注册、命名和审批见 [search-and-mcp.md](search-and-mcp.md#模型怎么调用)。

## MCP stdio 命令批准

stdio 服务器的 `command` 和 `args` 必须先经 `POST /api/mcp/servers/{id}/approve-command` 批准，`/test`、`/tools` 和模型循环里的注册才会启动该进程。未批准时接口返回 409 `{"error":"command_not_approved","command","args"}`，模型循环跳过这台服务器。http 服务器不需要这项批准。改了命令或参数后批准失效，需要再批一次。见 [search-and-mcp.md](search-and-mcp.md#mcp) 和 [api.md](api.md)。

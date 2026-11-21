<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/wordmark-dark.svg">
    <img src="assets/brand/wordmark-light.svg" alt="robroz" width="240">
  </picture>
</p>

# robroz

robroz。单机私有化的多智能体协同，交互是 IM，不是 IDE。

第一期已经开工。宿主机编排、定时、Docker 管理和容器里的执行器都用 Rust。客户端是 Rust 原生，Windows、Linux、macOS 各出一个包，不用 Tauri，也不套网页。不留 Go，不引 Python。

这期不做文件树、终端、桌面流、Bot 详情、Webhook、E2B 和多租户。

CI 跑宿主机测试，并在 Windows、Linux、macOS 上构建客户端。

## 术语表

| 叫法 | 是什么 | 在哪里用 |
| --- | --- | --- |
| bro | 一个 agent，也就是一个 bot。新建一个 agent 叫「引入一个 bro」。 | 只用在界面上：页签和列表标题叫「Bros」，资料页叫「bro 资料」，新建按钮叫「＋ 新 bro」。 |
| 默认名 | 新建时自动填的名字：第一个叫 `bro`，之后依次叫 `bro 2`、`bro 3`。用户随时可以改名。 | 只在新建时填一个默认值，不新增字段。 |
| `botId` / `botName` | 契约里已经冻住的字段名。 | 代码、接口和数据库里照旧用这两个名字，不改成 bro。 |
| 群 | 一个会话，可以有多个人和多个 bro。 | 界面、代码和接口都用这个叫法。 |
| 私聊 | 只有你和一个 bro 的群，不是另外一种概念。 | 界面上叫私聊，在代码里就是一个群。 |

## 节点 2

宿主机是单个 Rust 进程（`robroz-host`）。这一节点只做五件事：建群、加 Bot、发消息、派任务、拉事件。任务写入后状态保持 `queued`，不会启动容器，也不跑执行器。

- `POST /api/groups`：创建群
- `GET /api/groups`：列出群，按创建先后排序，返回 `{"groups":[{"id","name","tenantId"}]}`，没有群时为 `{"groups":[]}`
- `POST /api/groups/:id/members`：把 Bot 加入群；同一个 Bot 重复加入成功且不重复记账
- `POST /api/groups/:id/messages`：保存消息，`kind` 为 `text`、`image` 或 `file`。只落库，不写文件
- `POST /api/groups/:id/tasks`：给已在群里的 Bot 派任务，`state` 固定为 `queued`
- `POST /api/groups/:id/say` `{"body"}`：发一句话。先存成 text 消息，再看要不要派任务：私聊直接派给那个 bro；群里只有以 `@名字 内容`（或 `＠`）开头、名字是群成员时才派，名字取最长匹配，内容为空不派。返回 201 `{"message","task"}`，没派任务时 `task` 为 `null`。空内容 400「请填写内容」，群不存在 404「群不存在」。解析规则是 `robroz_contract::route_mention`，客户端可直接复用。`/messages` 和 `/tasks` 不变
- `GET /api/groups/:id/events`：按 `createdAt` 升序返回消息，以及每个任务恰好一张状态卡片（`label` 为「排队」）

SQLite 路径来自环境变量 `ROBROZ_DB`，默认 `./robroz.db`。监听地址来自 `ROBROZ_ADDR`，默认 `127.0.0.1:8790`。

```
cargo test -p robroz-host
```

`crates/robroz-contract`、`crates/robroz-policy`、`crates/robroz-card` 是旁边的共享 crate。宿主机目前只依赖 `robroz-contract`。`robroz-policy` 没有接到宿主机上。

## 节点 3

`pump` 把排队任务送进本机 Docker。`POST /api/groups/:id/tasks` 仍然马上返回 `queued`。第一次 `pump` 为一个任务启动一个容器，并把同一张状态卡改成 `running`，标签「进行」（`TaskState::label`），`createdAt` 不变，`updatedAt` 更新。容器成功写完结果后，下一次 `pump` 删掉容器，同一张卡改为 `done` / 「完成」，并追加一条 delivery：`summary` 为「已写入结果」，`path` 为 `tasks/{taskId}/result.txt`（相对路径，没有前导斜杠，也不带 `/workspace`），`briefing` 为 false。文件内容是任务的 prompt。

每个群一个目录，`ROBROZ_VOLUMES/{groupId}`，默认根目录 `./data/groups`，挂到容器内的 `/workspace`。容器删除后目录还在。镜像来自 `ROBROZ_IMAGE`，默认 `robroz-runner:local`。

容器按宿主机进程的 uid/gid 跑（`--user {uid}:{gid}`，非 unix 平台不加），容器在群目录里写出的文件都归宿主机用户，宿主机删得掉。要换用户就设 `ROBROZ_CONTAINER_USER`，比如 `1000:1000`。镜像不带 `--user` 时默认是 nobody（65534），不是 root。

prompt 不走环境变量，也不放进群目录：宿主机启动容器前把它写进这个任务的控制目录 `ROBROZ_CONTROL/{taskId}/prompt.txt`，runner 只读容器内的 `/control/prompt.txt`（只读挂载），不会退回去读 `/workspace`。环境变量只剩 `ROBROZ_TASK_ID`，`docker inspect` 看不到 prompt，容器也改不了它。容器只有两个挂载：群目录 `/workspace`（可写）和自己任务的控制目录 `/control`（只读，里面是 `prompt.txt` 和接管标记 `takeover`，见节点 4）。

Docker 调用不在数据库锁里。pump 在宿主机自己的线程上跑：先在锁内读任务，把 `queued` 改成 `running`（同一个任务不会被取两次），放锁后再 `docker run` / `inspect` / `rm`，最后重新加锁写结果。慢容器不会卡住 HTTP。`docker run` 失败时任务改回 `queued`。

```
docker build -f runner/Dockerfile -t robroz-runner:local .
```

CI 只编译 `robroz-runner`，不构建镜像，测试也不需要 Docker daemon。启动失败时任务保持 `queued`，没有 delivery。容器非 0 退出或结果文件不存在时，删掉容器，任务保持 `running`，不发 delivery，也不发明新状态。本 PR 不调用 `robroz-policy`，没有配额、没有 blocked、没有 resume，也没有定时调度。

## 节点 4

状态事件增加字段 `containerUp`。排队、完成、卡住，以及容器已经删掉的失败运行，都是 false。只有容器还在跑的 `running` 才是 true。失败运行仍然是 `TaskState::Running`，没有第五种状态：色调和排队一样是灰，标签是「没写出结果」，动作是空。容器还在时标签仍是「进行」。卡住才是琥珀色，动作「去私聊接管」。容器中途消失（宿主机或 Docker 重启、容器被删）也走这条失败运行：保持 `running`，`containerUp` 为 false，标签「没写出结果」，不重新排队，也不再启动容器。对它调用 `POST /api/tasks/:id/resume` 返回 409「这次不能恢复」。

权限分三级，都在 `robroz-policy::tier`：
- 放行（`Allow`）：工作区读写，不通知人，成功时把 prompt 写入 `tasks/{taskId}/result.txt` 并完成。
- 计数（`Metered`）：出网只允许 GET，次数记在群目录 `tasks/{taskId}/outbound.txt`，由 `robroz-policy::meter` 按任务计数，上限是环境变量 `ROBROZ_GET_LIMIT`，默认 8（用满上限仍算正常）。超限或非 GET：删掉容器，任务以 `done` 结束，带 `failure`「出网超额：已用 N 次，上限 M」或「出网只允许 GET：用了 POST」，不发 delivery，也不改成 blocked。
- 接管（`Takeover`）：`robroz-action:external` 和 `robroz-action:captcha`，不代填凭据，删掉容器，变成 blocked /「卡住」（external 另发审批卡，状态卡 `waitingFor` 为 `approval`；captcha 为 `takeover`，见「等什么（`waitingFor`）」），人到私聊接管，`POST /api/tasks/:id/resume` 恢复时挂回原来的群目录。

`POST /api/tasks/:id/resume` 只接受 blocked：任务不存在返回 404「任务不存在」，不是 blocked 返回 409「这次不能恢复」。成功返回 200 `{"id","state":"queued"}`：按库里存的卡住记录写接管标记，清掉这条记录，状态回到 `queued`，`containerUp` false。resume 不读群目录。库里没有这条卡住记录（比如宿主机存这条记录之前就卡住的旧任务）返回 500 `{"error":"恢复失败，找不到卡住记录"}`；写接管标记失败返回 500 `{"error":"恢复失败，无法写入接管标记"}`。这两种情况任务都留在 `blocked`，状态和 `updatedAt` 都不变，也不写标记。容器由之后的 `pump` 在原来的群目录上再启动，resume 本身不启动容器。没有定时，也不调用 `should_start_container`。容器仍然 `--network none`，挂载 `/workspace`，不挂 docker.sock，本节点不建代理。

接管标记：runner 停下时在群目录写 `tasks/{taskId}/intent`。这个文件只表示「卡住了」，宿主机从来不读它的内容。容器退出时宿主机自己算停在哪一行：用库里存的 prompt 和 `ROBROZ_CONTROL/{taskId}/takeover` 里的哈希，按顺序找第一行 sha256 不在 takeover 里的 `robroz-action:external` / `robroz-action:captcha`（`robroz-policy::blocking_line`，runner 决定在哪停用的也是这个函数）。算出来的类型、原文和哈希存进库里这个任务的 `intent_kind` / `intent_line` / `intent_hash`，然后删掉群目录里的 intent（下次启动容器前也会再清一次）。如果有 intent 但算不出这样一行（容器说卡住了，其实没有没接管过的阻塞行），按节点 4 的失败运行处理：保持 `running`，`containerUp` false，「没写出结果」，不算卡住。resume 只用库里这份，把哈希追加到 `takeover`（每行一个哈希）。这个目录在群目录外面，只有宿主机写；容器只拿到自己任务的那一个，只读挂在 `/control`。runner 只读 `/control/takeover`，跳过哈希在里面的阻塞行，跳过的行不写进 `result.txt`；碰到别的阻塞行照样卡住，再接管一次就再追加一个哈希。`/workspace` 里任何地方的 takeover 文件（包括 Bot 自己伪造的）一律不认。同一行原文在 prompt 里出现多次时，接管一次就都放行。库里存的 prompt 不改写。旧数据库打开时自动加上这三列，原有数据不动。


## 节点 5：定时

`POST /api/schedules`，请求体 `{"groupId","assigneeBotId","cron","prompt"}`，返回 201 `{"id","groupId","assigneeBotId","cron","prompt","nextRunAt","lastTaskId"}`，`lastTaskId` 第一次运行前是 `null`。群不存在返回 404「群不存在」，Bot 不在群里返回 400「这个 Bot 不在群里」，prompt 为空返回 400「请填写内容」，cron 不合法返回 400「cron 格式不对」。定时存在 SQLite，`tenant_id` 为 `local`。

cron 由宿主机按 UTC 计算，解析用 `croner`。接受 5 段（`分 时 日 月 周`）或 6 段（`秒 分 时 日 月 周`），不接受年份字段和 `@daily` 这类别名，也拒绝永远不会触发的表达式（比如 2 月 30 日）。容器里没有 cron。

宿主机循环里 `tick(now)` 和 `pump` 一起跑。到点的定时会调用 `robroz_policy::should_start_container`：上一次的任务还在排队，或者还在运行且 `containerUp` 为 true，就跳过这一次；否则新建一个普通的 `queued` 任务。两种情况都把 `nextRunAt` 推到严格晚于当前时间的下一次，错过的不补跑。`tick` 只建排队任务，不启动容器，容器只由 `pump` 启动。

定时任务的状态卡和普通任务一样。成功完成时 delivery 的 `briefing` 为 true，路径仍是该群目录里的 `tasks/{taskId}/result.txt`；普通任务的 `briefing` 仍为 false。卡住、失败和 resume 的行为与节点 4 相同。

`GET /api/groups/:id/schedules` 列出该群的例行任务，按创建先后排序，返回 `{"schedules":[ScheduleEntry]}`。每条是 `Schedule` 的字段平铺，再加上 `paused`：`id`、`groupId`、`assigneeBotId`、`cron`、`prompt`、`nextRunAt`、`lastTaskId`、`paused`。没有定时时为 `{"schedules":[]}`，群不存在返回 404「群不存在」。`POST /api/schedules` 的返回形状不变，不含 `paused`。

`PATCH /api/schedules/:id`，请求体 `{"paused":bool}`，成功返回 200 和这条 `ScheduleEntry`。暂停只阻止 `tick` 再新建任务，已经排队或正在跑的那一次不受影响。重复暂停仍返回 `paused` 为 true，没有其它变化。恢复时把 `nextRunAt` 推到当前时间之后的下一次，错过的不补跑。例行任务不存在返回 404「例行任务不存在」。

## 选项卡

群里的 Bot 可以出一张选项卡，问人在固定选项里选。这是事件流里单独的 `choice` 事件，不是状态卡。`POST /api/groups/:id/choices`，请求体 `{"botId","prompt","options","multi"}`，`multi` 默认为 false，返回 201 就是这张卡。`options` 是 `[{id,label,primary}]`，每张 2 到 4 个；`id` 和 `label` 去掉首尾空白后不能为空，`id` 不能重复；`primary` 最多一个，表示 Bot 的推荐。多选卡会去掉 `primary`（一律为 false）。群不存在返回 404「群不存在」，Bot 不在群里返回 400「这个 Bot 不在群里」，prompt 为空返回 400「请填写内容」。选项不合法返回 400，说明为「选项需要 2 到 4 个」「选项不能为空」「选项重复」或「推荐项最多一个」。不合法的卡不会入库。未回答时 `choice` 和 `answeredAt` 都是 `null`。

回传是 `POST /api/choices/:id/answer`，请求体 `{"choice":[ids],"answeredBy"?}`（`answeredBy` 见「审批卡」一节里的「处理人 `answeredBy`」），单选也传数组，且必须恰好一个；多选传一个或多个，不能重复。成功返回 200，仍是同一张卡：`choice` 为所选 `id`（多选保留请求顺序），`answeredAt` 为回答时间。每张只能答一次，再答返回 409「已经选过了」。选项卡不存在返回 404「选项卡不存在」。未知的选项 `id`、单选选了多个、多选一个都没选、或 `id` 重复，返回 400，说明为「没有这个选项」「只能选一个」「至少选一个」或「选项重复」。`GET /api/groups/:id/events` 按 `createdAt` 升序一并返回选项卡，已回答的仍是同一张卡。

## 审批卡

群里的 bro 要删除、发送或花钱之前，先出一张审批卡问人。这是事件流里单独的 `approval` 事件，排在 `choice` 之后新增，原有事件不变。

- 发卡：`POST /api/groups/{id}/approvals`，请求体 `{"botId","action","summary","path"?}`，返回 201 和这张卡。
  - `action` 是 `delete`、`send` 或 `spend`。
  - `summary` 去掉首尾空白后不能为空，只能一行，最多 200 字。
  - `path` 选填，只用来展示；去掉空白后为空就当没填。
  - 群不存在返回 404「群不存在」，bro 不在群里返回 400「这个 Bot 不在群里」，`summary` 为空返回 400「请填写要做什么」，太长或有换行返回 400「说明只能一行，最多 200 字」。
- 回答：`POST /api/approvals/{id}/approve` 表示同意，`POST /api/approvals/{id}/deny` 表示拒绝。请求体可以不带，也可以带 `{"answeredBy"?: 名字}`，见下面的「处理人 `answeredBy`」。成功返回 200，还是同一张卡，`decision` 变成 `approved` 或 `denied`，`answeredAt` 是回答时间。每张只能答一次，再答返回 409「已经答过了」；卡不存在返回 404「审批卡不存在」。
- 卡的字段：`id`、`botId`、`botName`、`action`、`summary`、`path`、`decision`、`createdAt`、`answeredAt`、`answeredBy`。`path`、`decision`、`answeredAt` 和 `answeredBy` 一定会出现，后三个在回答前是 `null`。
- 审批卡会出现在 `GET /api/groups/{id}/events` 和实时推送里，新卡算一条未读。
- 客户端碰到不认识的事件类型应当跳过，以后新增事件时旧客户端才不会出错。

### 任务自动发的审批卡

除了 bro 通过接口自己发卡，任务跑到要对外写入的地方（prompt 里有 `robroz-action:external` 这一行）时，宿主机会替它发一张审批卡，不再等人去私聊接管。

- 这张卡的 `action` 是 `send`，`summary` 是「要对外写入：要做的事」，`botId` 是这个任务的执行者。
- 后来新增的可选字段 `taskId`：任务发的卡带上这个任务的 id，bro 通过接口自己发的卡不带这个字段，旧 JSON 不变。
- 发卡时任务处于 `blocked`，和以前一样算一条未读。
- 同意：宿主机替你记下接管，任务重新排队，接着跑完。
- 拒绝：任务直接变成 `done`，不执行那一步，也没有交付卡。状态卡带 `failure`「你拒绝了：要做的事」，客户端显示成灰色的「没做成」，见「没做成（`failure`）」一节。
- 如果任务已经通过 `/resume` 被接管走了，再回答这张卡只记下回答，不会再动任务。
- 验证码（`robroz-action:captcha`）不发审批卡，因为得有人去填，所以还是走「卡住，然后去私聊接管」。

### 处理人 `answeredBy`

审批卡和选项卡都有一个后来新增的 `answeredBy` 字段，事件里一定会出现，没填时是 `null`。旧库启动时会自动加上这一列。

- 主机只有一个共享令牌，分不清是谁在操作，所以这里存的是客户端回答时自己报的设备名或人名，主机不做核实。
- 回答时可以不带。带的话会去掉首尾空白，空的就当没填；只能一行，最多 40 个字，不符合时返回 400「answeredBy 只能一行，最多 40 个字」。审批卡的请求体不是合法 JSON 时返回 422「请求格式不正确」。
- 客户端怎么显示：返回的 `answeredBy` 和自己报的名字一样就写「我」，不一样就写返回的名字，是 `null` 就不写是谁，只写「已批准 · HH:MM」或「已拒绝 · HH:MM」。时间一律用主机返回的 `answeredAt`。

## 节点 6：客户端

`client/` 是 iced 0.14 原生客户端，Windows、Linux、macOS 同一套代码。左边群列表，右边选中群的消息流：文字消息、图片/文件（只显示一行带标签的名字，不做查看器）、状态卡、交付卡和选项卡。

状态卡一个任务一张，按 `taskId` 原地更新。左边 Bot 名，右边一句状态。颜色、那句话和动作都来自 `robroz-card`（`appearance`、`status_line`、`palette`），客户端不自己定。卡住（`waitingFor` 为 `takeover`，或旧宿主机没带这个字段）是琥珀色，只有「去私聊接管」，这一期只打日志。`waitingFor` 为 `approval` 时不算卡住：灰色「等你批准」，没有按钮，去回答旁边那张审批卡就行。running 且容器在跑时，有 `progress` 就在 Bot 名下面左对齐加一行浅灰小字，超长用「…」，没有就不占位。`running` 且 `containerUp` 为 false 是灰色「没写出结果」，没有动作。交付卡是摘要加一行等宽的卷内相对路径，只显示，不能点。定时简报不是新卡种，就是同一个 delivery 事件 `briefing` 为 true，客户端只看这个字段，在角上加「简报」（文字来自 `robroz-card::briefing_badge`）。选项卡按 `id` 原地更新；这一期只显示，回传接口见上一节。

事件来源藏在 `EventSource` trait 后面（`client/src/source/`）。默认是 `HttpSource`：每秒轮询宿主机 `GET /api/groups/{id}/events`，直接解成 `robroz-contract` 的 `Events`，对不上契约就在底栏报错，不做兼容。事件流里出现 `type` 为 `choice` 的卡时，契约里还没有该类型的旧客户端会整段解码失败，而不是跳过这张卡。

`GET /api/groups/:id/events` 可带可选参数 `?since=`（RFC 3339）。只返回这个时间之后变过的事件：状态卡按 `updatedAt`，选项卡按 `answeredAt`（未回答则按 `createdAt`），消息和交付卡按 `createdAt`。没有变化时返回 `{"events":[]}`。时间不合法返回 400「since 不是合法时间」。参数省略或为空时仍返回全部事件。排序不变，仍按 `createdAt` 升序，相同时间再按 `id`。

宿主机地址用 `ROBROZ_HOST`，默认 `http://127.0.0.1:8790`。宿主机还没有列群接口，群用 `ROBROZ_GROUPS` 指定，逗号分隔，每项是 `id` 或 `id=名字`。`--fake` 或 `ROBROZ_FAKE=1` 改用离线的 `FakeSource`（单测也用它）。连不上时保留已显示的内容，下一秒再试。

```
ROBROZ_GROUPS=<groupId>=官网 cargo run -p robroz-client
cargo run -p robroz-client -- --fake
cargo test -p robroz-client
```

`cargo test -p robroz-client` 会在进程内起一份真实宿主机路由，用 `HttpSource` 读回来，不需要 Docker。

Linux 上运行需要系统里有 `libxkbcommon-x11`（X11）或 Wayland 运行库；编译本身不需要额外的开发包。

## 监听地址和令牌

- 默认只监听本机 `127.0.0.1:8790`，本机使用不用令牌。
- `ROBROZ_ADDR` 不是本机回环地址、又没设令牌时，宿主机拒绝启动，并提示先设 `ROBROZ_TOKEN` 或 `ROBROZ_TOKEN_FILE`。
- 令牌只从 `ROBROZ_TOKEN` 或 `ROBROZ_TOKEN_FILE`（文件路径）读，两个只能设一个。读进来时去掉首尾空白，空值等于没设。至少 16 个字符，中间不能有空白。
- 设了令牌以后，所有接口都要带 `Authorization: Bearer <令牌>`，监听本机时也一样。没带或带错返回 401，响应头带 `WWW-Authenticate: Bearer`。比对按常数时间做。
- 令牌不进日志，也不进任何报错信息。
- 客户端设了 `ROBROZ_TOKEN` 就会自动带上。

## 没做成（`failure`）

状态卡和交付卡都有一个冻结后新增的可选字段 `failure`，表示任务算 `done` 了，但活没做成。只有这种情况才带这个键，正常结果不带，所以旧 JSON 一点不变。

- 有四种情况会填 `failure`：
  - 模型调用失败：值是「模型没回：原因」。任务照样会走到 `done`，并且会出一张交付卡，状态卡和交付卡上带的是同一个 `failure`。
  - 审批被拒绝：值是「你拒绝了：要做的事」，其中「要做的事」取 prompt 里第一行普通内容，最多 60 个字。任务直接变成 `done`，没有交付卡，只有状态卡带 `failure`。见「审批卡」一节里的「任务自动发的审批卡」。
  - 出网超额或用了非 GET：值是「出网超额：已用 N 次，上限 M」或「出网只允许 GET：用了 POST」。任务直接变成 `done`，没有交付卡，只有状态卡带 `failure`。见上面权限三级里的「计数」。
  - 工具调用超过 8 轮：值是「工具调用超过 8 轮」。任务照样 `done`，结果里也写这句。见「接模型」。
- 状态卡只在 `done` 时带 `failure`。任务再次开跑时会清掉它，不会沿用上一次的。
- 客户端只要看到 `failure`，就把卡显示成灰色的「没做成：原因」，不显示绿色的完成，也没有按钮。

## 等什么（`waitingFor`）

状态卡有一个冻结后新增的可选字段 `waitingFor`，说明卡住的任务在等人做什么。不改冻住的字段和任务状态，旧 JSON 照样能解析。

- 只在 `blocked` 时带，其他状态都不带这个键。
- `approval`：这个任务有一张还没回答、而且对应它当前卡住那一行的审批卡（`robroz-action:external`）。去回答审批卡就行，同意后任务重新排队，拒绝后变成带 `failure` 的 `done`。
- `takeover`：没有这样的审批卡，比如验证码（`robroz-action:captcha`），得有人去私聊接管，再 `POST /api/tasks/:id/resume`。
- 同一个任务始终只有一张状态卡，按 `taskId` 原地更新。`waitingFor` 只改这张卡怎么显示，不另出卡。
- 客户端：`approval` 显示成灰色「等你批准」，没有按钮，列表预览写「Ada：等你批准」，不会被同时出现的审批卡摘要顶掉；`takeover` 或没有这个字段时，还是琥珀色「卡住」加「去私聊接管」。

## 进度（`progress`）

状态卡有一个冻结后新增的可选字段 `progress`：bro 正在干什么，原地更新在同一张状态卡上，不新增卡片类型，也不发消息。

- 只在 `running` 且容器在跑（`containerUp` 为 true）时带。`queued`、`blocked`、`done`（成功或带 `failure`）和容器没了的 `running` 都不带，旧 JSON 一点不变。
- 宿主机自己报的关键节点：开跑时「开始干活」（接了模型时是「在想怎么回」），沙箱起来后「在沙箱里跑」。
- bro 也能自己报：prompt 或模型回复里写一行 `robroz-progress:文字`。runner 把最后一行进度写到群目录 `tasks/{taskId}/progress`，宿主机在任务运行时读取；模型回复里的进度行开跑时就用上。进度行不会出现在 `result.txt` 里。
- 宿主机规范成一行：换行和连续空白变成一个空格，去掉首尾空白，最多 60 个字（超了截到 59 个字加「…」），不报错。
- 规范后的文字没变就不改 `updatedAt`，不会多推一次状态卡；变了才推。
- 任务结束、卡住、容器没了时清掉进度；重新开跑时从头报。
- 客户端：状态卡在 Bot 名下面加一行进度。聊天列表的预览也用这句，写成「Ada：在想怎么回」，不再是「Ada：进行」，太长时按列宽截断加「…」。没有进度、任务完成、卡住或没做成时，预览照旧显示状态那句话。

## 实时推送

`GET /api/groups/{id}/stream` 用 Server-Sent Events（`text/event-stream`）推送这个群的事件，和 `/events` 一样要带令牌。

- 每条推送是一个契约里的 `Event`：`event:` 是它的 `type`，`data:` 是事件的 JSON，`id:` 是这张卡的变化时间。
- 同一张卡再变化时会用同一个 `data.id` 再推一次，客户端按 id 原地替换，和轮询 `/events` 的处理方式相同。
- 断线重连时带标准的 `Last-Event-ID` 请求头，或者 `?since=`，只补推这个时间之后的变化。两者都带时以 `Last-Event-ID` 为准。时间不合法返回 400，群不存在返回 404。
- 没有变化就不推，只每 15 秒发一次保活注释。宿主机大约每 250 毫秒查一次库，不改任何写入路径。
- 轮询 `GET /api/groups/{id}/events?since=` 留作兜底。
- 不同类型的事件之间没有约定先后。比如一条消息和一张状态卡时间相同时，先推哪个不一定，客户端和测试都不要依赖这种顺序，只要按 id 去重、原地替换就行。

## 未读数

每个群记一个已读位置。现在是单用户，一个群只有一份已读位置。

- `GET /api/unread`：返回所有群的 `{groups:[Unread]}`，按 `lastActivityAt` 从新到旧排序，私聊也在里面。还没有任何活动的群排在最后，按建群顺序。
- `GET /api/groups/{id}/unread`：返回一个群的 `Unread`。群不存在返回 404。
- `POST /api/groups/{id}/read`：标记已读，请求体是 `{"readAt"?: 时间}`，不填就用当前时间。已读位置只会往后走，传一个更早的时间不会回退。时间不合法返回 400，群不存在返回 404，成功时返回更新后的 `Unread`。
- `Unread` 的字段：`groupId`、`readAt`（从没读过时为 `null`）、`unread`、`lastActivityAt`（群里没有任何活动时为 `null`）。
- 计入未读的只有 bro 发来、需要你看的东西：`readAt` 之后的交付卡、选项卡、审批卡、确认卡，以及变成卡住的任务。你自己发的消息，以及任务排队、进行中、完成这类状态变化，都不计入；但完成时的交付卡要计入。
- `lastActivityAt` 取消息、任务、交付卡、选项卡、审批卡和确认卡（已回答的按回答时间）里最新的一次变化，会话列表用它排序。

## bro 和私聊

bro 在库里就是一个 bot。私聊是只有这一个 bro 的普通群，所以消息、任务、事件、实时推送和未读数都直接可用，不另造概念。术语见上面的「术语表」。

- `POST /api/bros`：新建一个 bro，同时建好它的私聊，返回 201 和 `Bro`。请求体是 `{"name"?: 名字}`，不填名字时用默认名：没有叫 `bro` 的就叫 `bro`，否则从 `bro 2` 往上，取第一个没被占用的名字。
- `GET /api/bros`：返回 `{bros:[Bro]}`，按新建顺序排列。
- `PATCH /api/bros/{id}`：改名，请求体是 `{"name": 名字}`，私聊的群名也跟着改。bro 不存在返回 404。
- `POST /api/bros/{id}/dm`：取这个 bro 的私聊，没有就新建一个。新建时返回 201，已经有了返回 200。bro 不存在返回 404。
- `Bro` 的字段：`id`、`name`、`dmGroupId`。
- 名字会去掉首尾空白，不能为空，最长 64 个字符，不符合时返回 400。
- 私聊里只能有这一个 bro。往私聊里加别的 bro，`POST /api/groups/{id}/members` 会返回 409。

## 记忆（memories）

bro 记住的关于你的事：你是谁（`profile`）、你的工作习惯（`habit`）、项目记录（`project`）。bro 提议的记忆先是 `pending`，要在确认卡上点了才算数，见下面的「确认卡（confirm）」。

- `GET /api/memories?kind=&scope=&status=`：返回 `{memories:[Memory]}`，按 `updatedAt` 从新到旧。三个参数都可选，留空等于不筛。不传 `status` 时只返回 `kept` 和 `pending`，`rejected` 要写 `status=rejected` 才返回。忘掉的永远不返回。参数不合法返回 400。
- `POST /api/memories`：你自己记一条，请求体 `{"scope", "kind", "text"}`。`source` 是 `user`，`status` 直接是 `kept`，返回 201 和 `Memory`。
- `PATCH /api/memories/{id}`：请求体 `{"text"?, "kind"?, "scope"?}`，只改给了的字段，`updatedAt` 更新。一个字段都没给返回 400；不存在或已经忘掉返回 404。`status` 不能用 `PATCH` 改，请求体里带了也不理会。
- `DELETE /api/memories/{id}`：忘掉。只是软删，记下忘掉的时间，以后不会再出现。返回 204；重复忘掉也返回 204，不改第一次的时间；从没有过的 id 返回 404。忘掉一条还在 `pending` 的记忆时，它那张没回答的确认卡会一起关掉，记成 `denied`，`answeredBy` 是 `null`。
- `scope`：`user` 是所有 bro 都能用，`bot:<botId>` 只给这个 bro，bro 必须存在，否则 400「这个 bro 不存在」。其他写法返回 400。
- `kind`：`profile`、`habit`、`project`。`source`：`user`（你写的）或 `bot`（bro 提的）。`status`：`pending`（等你确认）、`kept`（在用）、`rejected`（你拒绝过，留着记录但不用）。
- `text` 会去掉首尾空白，不能为空，最长 2000 个字，不符合时返回 400。
- `Memory` 的字段：`id`、`scope`、`kind`、`text`、`source`、`status`、`createdAt`、`updatedAt`，以及后来新增的可选字段 `confirmId`：`pending` 且确认卡还没回答时带这张卡的 id，其他时候不带。
- 和其他 `/api` 一样，设了令牌就要带 `Authorization: Bearer <令牌>`。
- 注入模型：任务开始、宿主机调用模型时，把这个 bro 能用的已记下记忆（`scope` 为 `user` 或 `bot:<这个 bro>`，`status=kept`，没忘掉的）拼进 system 提示，加在固定开场白后面。格式是一段「你记得这些：」，下面每条一行，写成 `[kind] 原文`（`kind` 是 `profile` / `habit` / `project`），按 `updatedAt` 从新到旧。整段最多 4000 个字（按字符计），超出就整条丢掉最旧的，不截半条。一条都没有时不加这段。`pending`、`rejected` 和忘掉的都不会进去。


### 客户端的「记忆」页

点「记忆」按钮打开（在「例行任务」按钮旁边）。
- 按范围分组：「所有 bro」排第一，后面每个 bro 一组（「只给 Ada」）。每组里「待确认」和「已记下 N」分开放，只有已记下的组也显示「已记下 N」。每条显示原文、类型（资料、习惯、项目）和更新时间。一条也没有时显示「还没有记下任何东西」。
- 待确认的条目只有「记下」「不用」两个按钮，用 `confirmId` 调确认卡的回答接口，聊天里那张卡会跟着变。
- 已记下的条目可以「改」（走 `PATCH`）或「忘掉」（走 `DELETE`）。忘掉前会先确认一次「忘掉这条？bro 以后不会再用到它。」，忘掉以后这条直接从列表里消失。
- 「记一条」要先选范围，没选时那一行显示「选一下谁能用这条」，选好后换成灰色的「只给 Bob 用」或「所有 bro 都能用」。这一行一直占着位置，「保存」不会跳。
- 「保存」在没写内容、没改动、超过 2000 字或者没选范围时是灰的。保存成功后更新时间刷新，旁边标「已保存」；失败就在原地提示「没保存：原因」，草稿还在。

## 技能（skills）

bro 会的做法，只是一段提示文字：标题、`slug` 和说明。权限还是由策略决定，技能本身不开放任何能力。bro 提议的技能先是 `pending`，要在确认卡上点了才算数，见下面的「确认卡（confirm）」。

- `GET /api/skills?scope=&status=`：返回 `{skills:[Skill]}`，按 `updatedAt` 从新到旧。两个参数都可选。不传 `status` 时只返回 `kept` 和 `pending`，`rejected` 要写 `status=rejected` 才返回。忘掉的永远不返回。参数不合法返回 400。
- `POST /api/skills`：你自己写一条，请求体 `{"scope", "slug", "title", "instruction"}`。`source` 是 `user`，`status` 直接是 `kept`，返回 201 和 `Skill`。已经有同样的 `slug` 且没忘掉时返回 409「这个 slug 已经有了」。
- `PATCH /api/skills/{slug}`：请求体 `{"title"?, "instruction"?, "scope"?}`，只改给了的字段，`updatedAt` 更新。一个字段都没给返回 400；不存在或已经忘掉返回 404。`status` 和 `slug` 不能用 `PATCH` 改。
- `DELETE /api/skills/{slug}`：忘掉。只是软删。返回 204；重复忘掉也返回 204；从没有过的 slug 返回 404。忘掉一条还在 `pending` 的技能时，它那张没回答的确认卡会一起关掉，记成 `denied`，`answeredBy` 是 `null`。忘掉以后这个 slug 可以再写一条。
- `GET /api/groups/{id}/skills`：列出该群已启用、未忘掉的技能，返回形状和 `GET /api/skills` 一样，按 `updatedAt` 从新到旧。没有启用的返回 `{"skills":[]}`，群不存在返回 404「群不存在」。
- `PUT /api/groups/{id}/skills/{slug}`：在这个群里启用这条技能。群或技能不存在（或已忘掉）返回 404。已经启用再点一次也是 204。
- `DELETE /api/groups/{id}/skills/{slug}`：关掉。群或技能不存在返回 404；本来就没启用也是 204。
- `scope` 和记忆一样：`user` 是所有 bro 都能用，`bot:<botId>` 只给这个 bro。
- `slug` 是小写字母、数字和单个连字符，不能开头或结尾是连字符，最长 64 个字。`title` 最长 80 个字，`instruction` 最长 8000 个字，都会去掉首尾空白，不能为空。
- `Skill` 的字段：`id`、`scope`、`slug`、`title`、`instruction`、`source`、`status`、`createdAt`、`updatedAt`，以及可选的 `confirmId`：`pending` 且确认卡还没回答时带这张卡的 id，其他时候不带。
- 注入模型：任务开始、宿主机调用模型时，把这个 bro 能用、并且已在该群启用的已记下技能（`scope` 为 `user` 或 `bot:<这个 bro>`，`status=kept`，没忘掉的）另拼一段进 system 提示，加在记忆段后面（没有记忆就直接加在开场白后面）。格式是一段「你会这些技能：」，下面每条是 `### 标题 (slug)` 再加说明，按 `updatedAt` 从新到旧。整段最多 8000 个字（按字符计），超出就整条丢掉最旧的，不截半条。一条都没有、或这个群里一条都没启用时不加这段。`pending`、`rejected` 和忘掉的都不会进去。

### 客户端的「技能」页

点「技能」按钮打开，结构和「记忆」页一样。
- 按范围分组：「所有 bro」排第一，后面每个 bro 一组。每组里「待确认」和「已保存 N」分开放。每条显示标题、`slug` 和说明的前三行。一条也没有时显示「还没有保存任何技能」。
- 待确认的条目有「保存」「不用」两个按钮，用 `confirmId` 调确认卡的回答接口，聊天里那张卡会跟着变。
- 已保存的条目可以「改」（走 `PATCH`，`slug` 不能改）或「删除」（走 `DELETE`）。删除前要先确认一次「删除这项？bro 以后不会再用到它。」
- 已保存的条目下面有「在这些群里用」，每个群一个开关，打开走 `PUT /api/groups/{id}/skills/{slug}`，关掉走 `DELETE`。打开页面时先用 `GET /api/groups/{id}/skills` 读出真实状态，还没读到时开关是灰的，点不了，旁边写「读取中」；读失败写「读不到」。
- 「加一项」要填英文短名（`slug`，只能用小写字母、数字和连字符）、标题（最多 80 字）、说明（最多 8000 字），还要选范围。范围提示、「保存」置灰、成功标「已保存」、失败原地提示「没保存：原因」，这些都跟记忆页一样。

## 确认卡（confirm）

bro 想记下一条记忆或收下一项技能时不会直接写入，而是提一条，等你在确认卡上点「记下／保存」或「不用」。这是事件流里的 `confirm` 事件，原有事件和任务状态都没变。记忆和技能用同一种卡。

- bro 怎么提：在 prompt 或模型回复里写一行 `robroz-memory:{"kind":"habit","text":"周报用中文","scope":"user"}`，或一行 `robroz-skill:{"slug":"weekly-report","title":"写周报","instruction":"先列三点","scope":"user"}`。记忆的 `kind` 和 `text` 必填，技能的 `slug`、`title` 和 `instruction` 必填，`scope` 都选填，不填就是 `bot:<执行这个任务的 bro>`。runner 把这些行写进群目录的 `tasks/{taskId}/proposals.jsonl`，每行是 `{"type":"memory",...}` 或 `{"type":"skill",...}`。这些行不会出现在 `result.txt` 里；没有这种行时结果一个字节都不变。
- 什么时候收：任务成功跑完（`done` 且没有 `failure`）后，宿主机读这个文件，读完就删。没跑完、卡住、带 `failure` 的任务提的都不收。
- 哪些不收：JSON 不对、`type` 不认识、必填字段不对或超长、`scope` 写法不对或 bro 不存在，这些行直接跳过，不报错，也不影响任务。已经有同一 `scope`、同一 `kind`、同样文字（连续空白算一个）的记忆，而且没忘掉的，不论是 `pending`、`kept` 还是 `rejected`，都不再提。已经有同样 `slug`、而且没忘掉的技能，不论状态，都不再提。一个任务最多收前 20 条，记忆和技能加在一起算。
- 每收一条，就存一条 `source=bot`、`status=pending` 的记忆或技能，再发一张确认卡。
- 回答：和审批卡用同一组接口。`POST /api/approvals/{id}/approve` 是「记下／保存」，变成 `kept`；`POST /api/approvals/{id}/deny` 是「不用」，变成 `rejected`，记录留着，但不会拼进提示，bro 也不会再提同一条。请求体、`answeredBy`、返回 200 和这张卡、只能答一次（再答 409「已经答过了」）都和审批卡一样；卡不存在返回 404。`status` 只能这样改，列表里对 `pending` 条目作答时用 `confirmId` 调这两个接口。
- 卡的字段：`id`、`botId`、`botName`、`subject`、`taskId`、`decision`、`createdAt`、`answeredAt`、`answeredBy`，这几个一定会出现（`taskId`、`decision`、`answeredAt`、`answeredBy` 可能是 `null`）。
  - `subject` 是 `memory` 时还有 `memoryId`、`kind`、`text`、`scope`，卡上要显示的内容都在这里，客户端不用再另外请求。`text` 和 `scope` 是记忆现在的样子，`pending` 时用 `PATCH` 改了，这张卡也会再推一次。
  - `subject` 是 `skill` 时还有 `slug`、`title`、`instruction`、`scope`。`title`、`instruction` 和 `scope` 是技能现在的样子，`pending` 时用 `PATCH` 改了，这张卡也会再推一次。没有的字段不出现。
  - 同一个任务提了很多条时，客户端可以按 `taskId` 把超过 3 条的折叠起来。
- 客户端：卡的左上角是「记忆」标签，下面是完整原文和一行范围（「所有 bro」或「只给 Ada」），按钮是「记下」和「不用」。回答以后按钮消失，换成「记下了 · 我 · HH:MM」或灰色的「不用了 · …」，处理人的规则和审批卡一样。别人先答了（409）时，卡上原地显示别人的结果。在记忆页忘掉一条待确认的记忆，卡显示「不用了 · HH:MM」，不带名字。同一个任务待确认的卡超过 3 张时，从第 4 张起收成「还有 N 条待确认」，点开再逐条确认。
- 技能卡：左上角是「技能」标签，显示标题和 `slug`，说明下面有一行范围（「所有 bro」或「只给 Ada」）。说明默认只露前三行，点「展开」看全部，再点「收起」折回去。按钮是「保存」和「不用」，回答以后换成「保存了 · 我 · HH:MM」或灰色的「不用了 · …」。409、处理人和折叠的规则都跟记忆卡一样。点「保存」以后这条技能变成 `kept`，但还要在群里启用才会拼进提示，见「技能」一节。
- 确认卡会出现在 `GET /api/groups/{id}/events` 和实时推送里，新卡算一条未读。它和审批卡存在同一张表里，但不会以 `approval` 事件出现，也不影响状态卡的 `waitingFor`。

## 发一句话（`/say`）

客户端输入框发出的每一句话都走 `POST /api/groups/{id}/say`，请求体 `{"body": 内容}`。主机先把它存成一条文本消息，再决定要不要派任务；消息和任务在同一个事务里写入。原来的 `/messages` 和 `/tasks` 不变。

- 私聊里，整句话都派给那个 bro。
- 群里，只有以 `@名字 内容` 开头时才派任务，名字必须是群成员。全角 `＠` 也认。名字后面要紧跟空格或者句子结束，多个名字都能匹配时取最长的，所以 `@bro 2 写 README` 派给 `bro 2`，不会派给 `bro`。
- `@名字` 后面没有内容，或者名字不是群成员，就只存消息，不派任务。
- 返回 201 和 `{"message", "task"}`。没派任务时 `task` 为 `null`。
- 存的是去掉首尾空白以后的内容。去掉空白后为空返回 400「请填写内容」，群不存在返回 404「群不存在」。

## 接模型

bro 用的是任意 OpenAI 兼容服务的 `POST {ROBROZ_MODEL_BASE_URL}/chat/completions`，变量见下面的「宿主机环境变量」。

- 没设密钥就不接模型，bro 把 prompt 原样写进结果。CI 和端到端测试都走这条路。
- 设了密钥，就必须同时设 `ROBROZ_MODEL_BASE_URL` 和 `ROBROZ_MODEL`。两个密钥变量都设了、密钥文件是空的、密钥里有空白，或者地址不以 `http://` / `https://` 开头，宿主机都拒绝启动，报错里只写变量名，不写值。
- 调用时会把该 bro 已记下的记忆，以及已在该群启用的技能，拼进 system 提示，规则见「记忆」和「技能」两节。任务的 prompt 还是用户那一条，不会改。
- 模型由宿主机调用，单次调用最长等 120 秒。密钥不进容器，也不进群目录，日志和报错里都不会出现。沙箱照样不联网。
- 模型可以发起工具调用：宿主机在本机执行，再把结果回给模型，直到得到文字答复。一次任务最多向模型请求 8 次；第 8 次仍是工具调用，任务照样 `done`，`failure` 为「工具调用超过 8 轮」。单次工具结果最多 16000 个字（按字符计），超出就截断并加上「…（已截断）」。执行时进度显示「在用 <工具名>」。策略判为需审批或 Takeover 的工具先不执行，只把拒绝结果回给模型。没有注册任何工具时，请求里不带 `tools`，行为和原来完全一样。
- 宿主机在 `docker run` 之前把模型的回答写进这个任务的 `/control/reply.txt`（容器里只读），runner 用它代替回显，写进 `result.txt`。每次运行前都会先删掉旧的回答，重试时不会拿到上一次的。
- 任务会不会卡住，仍然只看 prompt 里的 `robroz-action:` 行，跟模型回答了什么无关。
- 模型调用失败时任务照样完成，结果写「模型没回：原因」。工具调用超过 8 轮时同样完成，结果和 `failure` 都写「工具调用超过 8 轮」。
- 真跑模型时，密钥由 Panda 在私聊里交给后端，不要发到群里，也不要提交进仓库。

## 联网搜索

宿主机提供两个工具给模型：`web_search`（搜索公开网页）和 `web_fetch`（用 GET 读一个页面，转成纯文本）。都在宿主机上调用，沙箱仍然不联网。权限归 Metered 那一级，和现有出网一样按任务计次，超额时任务变成带 `failure`「出网超额：已用 N 次，上限 M」的 `done`，没有交付卡。`web_fetch` 只允许 http / https，并拒绝内网、回环和链路本地地址。

服务由 robroz 用户自己选、自己填，不必把密钥交给别人：

- 可以选 [Brave Search](https://brave.com/search/api/) 或自建 [SearXNG](https://docs.searxng.org/)。
- 设置接口：`GET` / `PUT /api/settings/search`，以及 `POST /api/settings/search/test` 用来试连。`GET` 返回 `provider`（`brave`、`searxng` 或 `null`）、`url`、`hasKey` 和 `keyLast4`，绝不回显密钥。`PUT` 时 `key` 不传表示不改，传 `null` 表示清除；不合法的值返回 400。`POST …/test` 会真的发一次查询，返回 `{ok}` 或 `{ok: false, error}`，超时 10 秒。设置接口里的值优先于环境变量。密钥存在本机 SQLite 的 `settings` 表，不进日志，也不进报错。
- `source`：已生效时，`GET` 和 `PUT` 的返回体带可选字段 `source`，为 `settings`（来自设置接口，优先）或 `env`（来自环境变量）；完全没配置时不返回这个字段。
- 也可以用环境变量：`ROBROZ_SEARCH_PROVIDER`（`brave` 或 `searxng`）、`ROBROZ_BRAVE_KEY`（Brave 的订阅密钥）、`ROBROZ_SEARXNG_URL`（SearXNG 实例地址，要以 `http://` 或 `https://` 开头）。
- 还没设置时，`web_search` 仍可以调用，会返回「还没设置联网搜索」。这句话会原样出现在交付里，交付事件带可选字段 `needsSetup: "search"`，客户端按这个字段显示「去设置」，不要靠匹配文字。

### 怎么填搜索服务

两种方式选一种。设置接口里填的优先于环境变量，改了不用重启宿主机。

**用设置接口**（宿主机设了令牌时加上 `-H "Authorization: Bearer $ROBROZ_TOKEN"`）：

```sh
# Brave：在 Brave Search API 后台拿订阅密钥
curl -X PUT http://127.0.0.1:8790/api/settings/search \
  -H 'Content-Type: application/json' \
  -d '{"provider":"brave","key":"<你的 Brave 密钥>"}'

# SearXNG：填实例地址，不用密钥
curl -X PUT http://127.0.0.1:8790/api/settings/search \
  -H 'Content-Type: application/json' \
  -d '{"provider":"searxng","url":"https://searx.example.com"}'

# 试连：返回 {"ok":true}，或者 {"ok":false,"error":"原因"}
curl -X POST http://127.0.0.1:8790/api/settings/search/test

# 只换密钥就只传 key；清除密钥传 null
curl -X PUT http://127.0.0.1:8790/api/settings/search \
  -H 'Content-Type: application/json' -d '{"key":null}'
```

PUT 成功后返回的就是 `GET` 的内容，只有 `hasKey` 和 `keyLast4`，看不到密钥本身。

**用环境变量**：启动宿主机前设 `ROBROZ_SEARCH_PROVIDER=brave` 加 `ROBROZ_BRAVE_KEY`，或者 `ROBROZ_SEARCH_PROVIDER=searxng` 加 `ROBROZ_SEARXNG_URL`。

注意：

- 密钥只放在宿主机本机，不要提交进仓库，也不要发到群里。
- 宿主机会请求 SearXNG 的 `/search?format=json`（地址末尾写不写 `/search` 都行），实例要在 `settings.yml` 的 `search.formats` 里打开 `json`，否则试连会失败。
- 搜索和 `web_fetch` 都算在每个任务的出网次数里，上限见 `ROBROZ_GET_LIMIT`。

## MCP

宿主机作为 MCP 客户端，可以接本机用 stdio 启动的服务器，或用 Streamable HTTP 访问的服务器。沙箱不直接连这些服务器。任务开始时，宿主机把「该群已启用、并且服务器本身也是 enabled」的工具注册进模型循环；某台服务器连接失败就跳过它，任务照常进行。

- 端点：`GET` / `POST /api/mcp/servers`，`PATCH` / `DELETE /api/mcp/servers/{id}`，`GET /api/mcp/servers/{id}/tools`（实时连一次，列出工具），`POST /api/mcp/servers/{id}/test`（试连，返回 `{ok, error?, toolCount?}`）。按群启用：`GET /api/groups/{id}/mcp` 列出已启用的服务器，`PUT` / `DELETE /api/groups/{id}/mcp/{serverId}` 打开或关掉。
- 每台服务器有唯一的名字，以及 `stdio`（填启动命令和参数）或 `http`（填地址）两种传输方式。可以存多条密钥：stdio 用环境变量，HTTP 用请求头。接口只返回 `{name, in, hasKey, keyLast4}`，绝不回显密钥。`PATCH` 时 `key` 不传表示不改，传 `null` 表示清除。密钥不进日志，也不进报错。
- `GET …/tools` 每一项带 `name`、`description` 和 `needsApproval`：工具标注了 `readOnlyHint: true` 时为 `false`，否则为 `true`。列表项可带 `toolCount`，是最近一次成功连接时看到的工具数。
- 交给模型的工具名是 `{safe_server}__{safe_tool}`（只保留字母、数字、`_`、`-`，最长 64 个字符；重名时加短哈希）。进度行和审批卡仍用原来的 `{server}.{tool}`，例如「在用 notes.write_note」。
- 权限跟 `robroz-policy` 对齐：`readOnlyHint: true` 的工具是 Metered（按一次出网 GET 计次），直接调用；其余要先出审批卡（`action` 为 `call`），卡上带 `mcpServer`、`mcpTool` 和参数摘要，并带 `taskId`。批准、拒绝走现有的 `POST /api/approvals/{id}/approve` 和 `/deny`，答过一次再答是 409。等待期间任务是 `blocked`，不加新状态。批准后执行并把结果交回模型继续；拒绝则不执行，任务 `done`，交付卡带 `failure`（「你拒绝了：{server}.{tool}」），不是成功交付。
- 工具结果和其它宿主工具一样，超过 16000 个字符会截断，末尾加上「…（已截断）」。
- **stdio 启动的是本机进程，只能由用户通过这些接口配置；模型不能增删服务器。** 不要把来历不明的命令填进去。
- 服务器配置存在本机 SQLite，表是 `mcp_servers` 和 `group_mcp`。

## 客户端环境变量

- `ROBROZ_USER_NAME`：这台客户端的名字。答审批卡和选项卡时作为 `answeredBy` 发出去，卡片回来时如果和它一样就显示「我」。不设就用电脑的主机名，主机名也拿不到时用 `robroz`。只取第一行非空的内容，最多 40 个字。两台设备不要用同一个名字，否则会互相把对方的回答显示成「我」。

## 宿主机环境变量

- `ROBROZ_ADDR`：监听地址，默认 `127.0.0.1:8790`。不是本机回环地址时必须设令牌
- `ROBROZ_TOKEN`：接口令牌，至少 16 个字符。不设就不鉴权
- `ROBROZ_TOKEN_FILE`：从这个文件读令牌，和 `ROBROZ_TOKEN` 只能设一个
- `ROBROZ_DB`：SQLite 路径，默认 `./robroz.db`
- `ROBROZ_VOLUMES`：群目录根，默认 `./data/groups`，每个群一个子目录，挂到 `/workspace`
- `ROBROZ_CONTROL`：任务控制目录根（每个任务的 `prompt.txt` 和接管标记 `takeover`），默认 `./data/control`。必须在 `ROBROZ_VOLUMES` 外面，否则宿主机拒绝启动
- `ROBROZ_IMAGE`：runner 镜像，默认 `robroz-runner:local`
- `ROBROZ_CONTAINER_USER`：容器的 `--user`，默认宿主机进程的 `uid:gid`
- `ROBROZ_GET_LIMIT`：每个任务允许的出网 GET 次数，默认 8。用满 8 次仍算正常，第 9 次任务以 `done` 结束并带 `failure`「出网超额：已用 9 次，上限 8」
- `ROBROZ_MODEL_BASE_URL`：OpenAI 兼容服务的地址，比如 `https://api.openai.com/v1`，要以 `http://` 或 `https://` 开头。设了密钥就必须设
- `ROBROZ_MODEL`：模型名，比如 `gpt-4o-mini`。设了密钥就必须设
- `ROBROZ_MODEL_KEY`：模型密钥。不设就不接模型，bro 原样回显
- `ROBROZ_MODEL_KEY_FILE`：从这个文件读模型密钥，和 `ROBROZ_MODEL_KEY` 只能设一个
- `ROBROZ_SEARCH_PROVIDER`：联网搜索用哪家，`brave` 或 `searxng`。不设、也没在设置接口里填，就是还没设置联网搜索
- `ROBROZ_BRAVE_KEY`：Brave Search 的订阅密钥。只存在本机，接口不回显
- `ROBROZ_SEARXNG_URL`：SearXNG 实例地址，要以 `http://` 或 `https://` 开头

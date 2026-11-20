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

## 节点 2

宿主机是单个 Rust 进程（`robroz-host`）。这一节点只做五件事：建群、加 Bot、发消息、派任务、拉事件。任务写入后状态保持 `queued`，不会启动容器，也不跑执行器。

- `POST /api/groups`：创建群
- `POST /api/groups/:id/members`：把 Bot 加入群；同一个 Bot 重复加入成功且不重复记账
- `POST /api/groups/:id/messages`：保存消息，`kind` 为 `text`、`image` 或 `file`。只落库，不写文件
- `POST /api/groups/:id/tasks`：给已在群里的 Bot 派任务，`state` 固定为 `queued`
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

状态事件增加字段 `containerUp`。排队、完成、卡住，以及容器已经删掉的失败运行，都是 false。只有容器还在跑的 `running` 才是 true。失败运行仍然是 `TaskState::Running`，没有第五种状态：色调和排队一样是灰，标签是「没写出结果」，动作是空。容器还在时标签仍是「进行」。卡住才是琥珀色，动作「去私聊接管」。

GET 次数记在群目录 `tasks/{taskId}/outbound.txt`，由 `robroz-policy::outbound` 判断，上限是环境变量 `ROBROZ_GET_LIMIT`，默认 8。超限或非 GET：删掉容器，保持 `running`，`containerUp` false，不发 delivery，也不改成 blocked。`robroz-action:external` 和 `robroz-action:captcha` 走 `side_effect`，变成 blocked / 「卡住」。工作区读写允许，成功时仍把 prompt 写入 `tasks/{taskId}/result.txt` 并完成。

`POST /api/tasks/:id/resume` 只接受 blocked：任务不存在返回 404「任务不存在」，不是 blocked 返回 409「这次不能恢复」。成功返回 200 `{"id","state":"queued"}`：按库里存的卡住记录写接管标记，清掉这条记录，状态回到 `queued`，`containerUp` false。resume 不读群目录。库里没有这条卡住记录（比如宿主机存这条记录之前就卡住的旧任务）返回 500 `{"error":"恢复失败，找不到卡住记录"}`；写接管标记失败返回 500 `{"error":"恢复失败，无法写入接管标记"}`。这两种情况任务都留在 `blocked`，状态和 `updatedAt` 都不变，也不写标记。容器由之后的 `pump` 在原来的群目录上再启动，resume 本身不启动容器。没有定时，也不调用 `should_start_container`。容器仍然 `--network none`，挂载 `/workspace`，不挂 docker.sock，本节点不建代理。

接管标记：runner 停下时在群目录写 `tasks/{taskId}/intent`。这个文件只表示「卡住了」，宿主机从来不读它的内容。容器退出时宿主机自己算停在哪一行：用库里存的 prompt 和 `ROBROZ_CONTROL/{taskId}/takeover` 里的哈希，按顺序找第一行 sha256 不在 takeover 里的 `robroz-action:external` / `robroz-action:captcha`（`robroz-policy::blocking_line`，runner 决定在哪停用的也是这个函数）。算出来的类型、原文和哈希存进库里这个任务的 `intent_kind` / `intent_line` / `intent_hash`，然后删掉群目录里的 intent（下次启动容器前也会再清一次）。如果有 intent 但算不出这样一行（容器说卡住了，其实没有没接管过的阻塞行），按节点 4 的失败运行处理：保持 `running`，`containerUp` false，「没写出结果」，不算卡住。resume 只用库里这份，把哈希追加到 `takeover`（每行一个哈希）。这个目录在群目录外面，只有宿主机写；容器只拿到自己任务的那一个，只读挂在 `/control`。runner 只读 `/control/takeover`，跳过哈希在里面的阻塞行，跳过的行不写进 `result.txt`；碰到别的阻塞行照样卡住，再接管一次就再追加一个哈希。`/workspace` 里任何地方的 takeover 文件（包括 Bot 自己伪造的）一律不认。同一行原文在 prompt 里出现多次时，接管一次就都放行。库里存的 prompt 不改写。旧数据库打开时自动加上这三列，原有数据不动。


## 节点 5：定时

`POST /api/schedules`，请求体 `{"groupId","assigneeBotId","cron","prompt"}`，返回 201 `{"id","groupId","assigneeBotId","cron","prompt","nextRunAt","lastTaskId"}`，`lastTaskId` 第一次运行前是 `null`。群不存在返回 404「群不存在」，Bot 不在群里返回 400「这个 Bot 不在群里」，prompt 为空返回 400「请填写内容」，cron 不合法返回 400「cron 格式不对」。定时存在 SQLite，`tenant_id` 为 `local`。

cron 由宿主机按 UTC 计算，解析用 `croner`。接受 5 段（`分 时 日 月 周`）或 6 段（`秒 分 时 日 月 周`），不接受年份字段和 `@daily` 这类别名，也拒绝永远不会触发的表达式（比如 2 月 30 日）。容器里没有 cron。

宿主机循环里 `tick(now)` 和 `pump` 一起跑。到点的定时会调用 `robroz_policy::should_start_container`：上一次的任务还在排队，或者还在运行且 `containerUp` 为 true，就跳过这一次；否则新建一个普通的 `queued` 任务。两种情况都把 `nextRunAt` 推到严格晚于当前时间的下一次，错过的不补跑。`tick` 只建排队任务，不启动容器，容器只由 `pump` 启动。

定时任务的状态卡和普通任务一样。成功完成时 delivery 的 `briefing` 为 true，路径仍是该群目录里的 `tasks/{taskId}/result.txt`；普通任务的 `briefing` 仍为 false。卡住、失败和 resume 的行为与节点 4 相同。

## 节点 6：客户端

`client/` 是 iced 0.14 原生客户端，Windows、Linux、macOS 同一套代码。左边群列表，右边选中群的消息流：文字消息、图片/文件（只显示一行带标签的名字，不做查看器）、状态卡和交付卡。

状态卡一个任务一张，按 `taskId` 原地更新。左边 Bot 名，右边一句状态。颜色、那句话和动作都来自 `robroz-card`（`appearance`、`status_line`、`palette`），客户端不自己定。卡住只有「去私聊接管」，这一期只打日志。`running` 且 `containerUp` 为 false 是灰色「没写出结果」，没有动作。交付卡是摘要加一行等宽的卷内相对路径，只显示，不能点。定时简报不是新卡种，就是同一个 delivery 事件 `briefing` 为 true，客户端只看这个字段，在角上加「简报」（文字来自 `robroz-card::briefing_badge`）。

事件来源藏在 `EventSource` trait 后面（`client/src/source/`）。默认是 `HttpSource`：每秒轮询宿主机 `GET /api/groups/{id}/events`，直接解成 `robroz-contract` 的 `Events`，对不上契约就在底栏报错，不做兼容。宿主机地址用 `ROBROZ_HOST`，默认 `http://127.0.0.1:8790`。宿主机还没有列群接口，群用 `ROBROZ_GROUPS` 指定，逗号分隔，每项是 `id` 或 `id=名字`。`--fake` 或 `ROBROZ_FAKE=1` 改用离线的 `FakeSource`（单测也用它）。连不上时保留已显示的内容，下一秒再试。

```
ROBROZ_GROUPS=<groupId>=官网 cargo run -p robroz-client
cargo run -p robroz-client -- --fake
cargo test -p robroz-client
```

`cargo test -p robroz-client` 会在进程内起一份真实宿主机路由，用 `HttpSource` 读回来，不需要 Docker。

Linux 上运行需要系统里有 `libxkbcommon-x11`（X11）或 Wayland 运行库；编译本身不需要额外的开发包。

## 宿主机环境变量

- `ROBROZ_ADDR`：监听地址，默认 `127.0.0.1:8790`
- `ROBROZ_DB`：SQLite 路径，默认 `./robroz.db`
- `ROBROZ_VOLUMES`：群目录根，默认 `./data/groups`，每个群一个子目录，挂到 `/workspace`
- `ROBROZ_CONTROL`：任务控制目录根（每个任务的 `prompt.txt` 和接管标记 `takeover`），默认 `./data/control`。必须在 `ROBROZ_VOLUMES` 外面，否则宿主机拒绝启动
- `ROBROZ_IMAGE`：runner 镜像，默认 `robroz-runner:local`
- `ROBROZ_CONTAINER_USER`：容器的 `--user`，默认宿主机进程的 `uid:gid`
- `ROBROZ_GET_LIMIT`：每个任务的 GET 上限，默认 8

# openbots

Project Hive。单机私有化的多智能体协同，交互是 IM，不是 IDE。

第一期已经开工。宿主机编排、定时、Docker 管理和容器里的执行器都用 Rust。客户端是 Rust 原生，Windows、Linux、macOS 各出一个包，不用 Tauri，也不套网页。不留 Go，不引 Python。

这期不做文件树、终端、桌面流、Bot 详情、Webhook、E2B 和多租户。

CI 跑宿主机测试，并在 Windows、Linux、macOS 上构建客户端。

## 节点 2

宿主机是单个 Rust 进程（`openbots-host`）。这一节点只做五件事：建群、加 Bot、发消息、派任务、拉事件。任务写入后状态保持 `queued`，不会启动容器，也不跑执行器。

- `POST /api/groups`：创建群
- `POST /api/groups/:id/members`：把 Bot 加入群；同一个 Bot 重复加入成功且不重复记账
- `POST /api/groups/:id/messages`：保存消息，`kind` 为 `text`、`image` 或 `file`。只落库，不写文件
- `POST /api/groups/:id/tasks`：给已在群里的 Bot 派任务，`state` 固定为 `queued`
- `GET /api/groups/:id/events`：按 `createdAt` 升序返回消息，以及每个任务恰好一张状态卡片（`label` 为「排队」）

SQLite 路径来自环境变量 `OPENBOTS_DB`，默认 `./openbots.db`。监听地址来自 `OPENBOTS_ADDR`，默认 `127.0.0.1:8790`。

```
cargo test -p openbots-host
```

`crates/hive-contract`、`crates/hive-policy`、`crates/hive-card` 是旁边的共享 crate。宿主机目前只依赖 `hive-contract`。`hive-policy` 没有接到宿主机上。

## 节点 3

`pump` 把排队任务送进本机 Docker。`POST /api/groups/:id/tasks` 仍然马上返回 `queued`。第一次 `pump` 为一个任务启动一个容器，并把同一张状态卡改成 `running`，标签「进行」（`TaskState::label`），`createdAt` 不变，`updatedAt` 更新。容器成功写完结果后，下一次 `pump` 删掉容器，同一张卡改为 `done` / 「完成」，并追加一条 delivery：`summary` 为「已写入结果」，`path` 为 `tasks/{taskId}/result.txt`（相对路径，没有前导斜杠，也不带 `/workspace`），`briefing` 为 false。文件内容是任务的 prompt。

每个群一个目录，`OPENBOTS_VOLUMES/{groupId}`，默认根目录 `./data/groups`，挂到容器内的 `/workspace`。容器删除后目录还在。镜像来自 `OPENBOTS_IMAGE`，默认 `openbots-runner:local`。

```
docker build -f runner/Dockerfile -t openbots-runner:local .
```

CI 只编译 `hive-runner`，不构建镜像，测试也不需要 Docker daemon。启动失败时任务保持 `queued`，没有 delivery。容器非 0 退出或结果文件不存在时，删掉容器，任务保持 `running`，不发 delivery，也不发明新状态。本 PR 不调用 `hive-policy`，没有配额、没有 blocked、没有 resume，也没有定时调度。

## 节点 4

状态事件增加字段 `containerUp`。排队、完成、卡住，以及容器已经删掉的失败运行，都是 false。只有容器还在跑的 `running` 才是 true。失败运行仍然是 `TaskState::Running`，没有第五种状态：色调和排队一样是灰，标签是「没写出结果」，动作是空。容器还在时标签仍是「进行」。卡住才是琥珀色，动作「去私聊接管」。

GET 次数记在群目录 `tasks/{taskId}/outbound.txt`，由 `hive-policy::outbound` 判断，上限是环境变量 `OPENBOTS_GET_LIMIT`，默认 8。超限或非 GET：删掉容器，保持 `running`，`containerUp` false，不发 delivery，也不改成 blocked。`hive-action:external` 和 `hive-action:captcha` 走 `side_effect`，变成 blocked / 「卡住」。工作区读写允许，成功时仍把 prompt 写入 `tasks/{taskId}/result.txt` 并完成。

`POST /api/tasks/:id/resume` 只接受 blocked：删掉 intent，状态回到 `queued`，`containerUp` false。容器由之后的 `pump` 在原来的群目录上再启动，resume 本身不启动容器。没有定时，也不调用 `should_start_container`。容器仍然 `--network none`，挂载 `/workspace`，不挂 docker.sock，本节点不建代理。

## 节点 5：定时

`POST /api/schedules`，请求体 `{"groupId","assigneeBotId","cron","prompt"}`，返回 201 `{"id","groupId","assigneeBotId","cron","prompt","nextRunAt","lastTaskId"}`，`lastTaskId` 第一次运行前是 `null`。群不存在返回 404「群不存在」，Bot 不在群里返回 400「这个 Bot 不在群里」，prompt 为空返回 400「请填写内容」，cron 不合法返回 400「cron 格式不对」。定时存在 SQLite，`tenant_id` 为 `local`。

cron 由宿主机按 UTC 计算，解析用 `croner`。接受 5 段（`分 时 日 月 周`）或 6 段（`秒 分 时 日 月 周`），不接受年份字段和 `@daily` 这类别名，也拒绝永远不会触发的表达式（比如 2 月 30 日）。容器里没有 cron。

宿主机循环里 `tick(now)` 和 `pump` 一起跑。到点的定时会调用 `hive_policy::should_start_container`：上一次的任务还在排队，或者还在运行且 `containerUp` 为 true，就跳过这一次；否则新建一个普通的 `queued` 任务。两种情况都把 `nextRunAt` 推到严格晚于当前时间的下一次，错过的不补跑。`tick` 只建排队任务，不启动容器，容器只由 `pump` 启动。

定时任务的状态卡和普通任务一样。成功完成时 delivery 的 `briefing` 为 true，路径仍是该群目录里的 `tasks/{taskId}/result.txt`；普通任务的 `briefing` 仍为 false。卡住、失败和 resume 的行为与节点 4 相同。


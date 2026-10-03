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

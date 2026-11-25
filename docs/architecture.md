# 架构

各部分怎么分工、数据怎么流。目录和 crate 一览见 [AGENTS.md](../AGENTS.md#仓库是什么)；接口见 [api.md](api.md)；权限见 [permissions.md](permissions.md)。

## 总体

```
客户端（iced）──HTTP / SSE──> 宿主机 robroz-host ──docker run──> 容器 robroz-runner
                                │  SQLite                         │ /workspace = 群目录（可写）
                                │  模型（OpenAI 兼容）              │ /control   = 控制目录（只读）
                                │  联网搜索、MCP 客户端              │ --network none
```

- 宿主机是一个 Rust 进程。依赖 `robroz-contract`（契约）、`robroz-policy`（权限）、`robroz-card`（卡片文案）、`robroz-runner`（和容器共享的文件格式）、`robroz-model`（调模型）、`robroz-mcp`（MCP 客户端）。
- 容器里只有 runner，不联网，拿不到模型密钥。模型、搜索、MCP 都在宿主机上调用。
- 客户端只通过 HTTP 接口和 SSE 跟宿主机说话，卡片怎么显示由 `robroz-card` 和 `robroz-ui` 决定。

## 宿主机

- HTTP：axum，路由在 `host/src/lib.rs:router_from`，外面套一层令牌校验 `host/src/auth.rs:require_token`。
- 存储：SQLite，路径 `ROBROZ_DB`。主要的表：`groups`、`bots`、`memberships`、`messages`、`tasks`、`deliveries`、`schedules`（`host/src/lib.rs`），`choices`、`approvals`（审批卡和确认卡同一张表），`group_reads`（已读位置），`memories`，`skills`、`group_skills`，`settings`（搜索和模型设置），`mcp_servers`、`group_mcp`。旧库启动时自动补新加的列，原有数据不动。`tenant_id` 固定 `local`。
- 主循环：`host/src/main.rs:main` 每秒调一次 `tick`（例行任务）、`pump`（任务）和 Watchdog。实现在 `host/src/pump.rs` 和 `host/src/watchdog.rs`。

### pump

1. 在数据库锁里取一个 `queued` 任务，改成 `running`（同一个任务不会被取两次），然后放锁。
2. 接了模型时先调模型，把回答写进 `ROBROZ_CONTROL/{taskId}/reply.txt`。每次运行前先删掉旧的回答，重试拿不到上一次的。模型调用和工具调用在这个任务自己的线程上跑（`host/src/steps.rs`，一个任务同时最多一个），pump 线程不等；回答好了下一轮 pump 再接着起容器。
3. 放锁后 `docker run` / `inspect` / `rm`，最后重新加锁写结果。慢容器不会卡住 HTTP。`docker run` 失败时任务改回 `queued`，没有交付卡。
4. 容器退出后按结果写状态：完成发交付卡；碰到阻塞行变 `blocked`；出网超额变带 `failure` 的 `done`；非 0 退出、没写出结果、容器中途消失变 `done`，`failure` 为「没写出结果」，不重排队。细节见 [api.md 的「pump 做什么」](api.md#what-the-pump-does)。
5. 成功完成的任务读 `proposals.jsonl`，发确认卡（见 [memory-and-skills.md](memory-and-skills.md)）。
6. The Watchdog only hints when a model or tool step is slow (at half of `ROBROZ_STEP_TIMEOUT_SECS`) or a container's progress has not moved (it writes `progress` and a log line). It does not change the state or requeue. An old task that is `running` with `containerUp` false ends as `done` + 「没写出结果」. A step that reaches `ROBROZ_STEP_TIMEOUT_SECS` is cancelled by the pump, not the Watchdog: see step 7 (step timeout) in [api.md's What pump does](api.md#what-the-pump-does).

### 容器

- 镜像 `ROBROZ_IMAGE`，默认 `robroz-runner:local`，用 `docker build -f runner/Dockerfile -t robroz-runner:local .` 构建。CI 只编译 runner，不构建镜像，单元测试不需要 Docker。
- 参数在 `host/src/sandbox.rs`：`--network none`，不挂 docker.sock。只有两个挂载：群目录 `ROBROZ_VOLUMES/{groupId}` → `/workspace`（可写），任务控制目录 `ROBROZ_CONTROL/{taskId}` → `/control`（只读）。
- 用户：`--user {uid}:{gid}`，默认是宿主机进程的 uid/gid（非 unix 平台不加），容器写出的文件归宿主机用户、宿主机删得掉。`ROBROZ_CONTAINER_USER` 可以改，比如 `1000:1000`。镜像本身不带 `--user` 时默认 nobody（65534），不是 root。
- 环境变量只有 `ROBROZ_TASK_ID`。prompt 不走环境变量，也不进群目录：宿主机起容器前写进 `ROBROZ_CONTROL/{taskId}/prompt.txt`，runner 只读 `/control/prompt.txt`，不会退回去读 `/workspace`。`docker inspect` 看不到 prompt，容器也改不了它。
- `ROBROZ_CONTROL` 必须在 `ROBROZ_VOLUMES` 外面，否则宿主机拒绝启动。

### runner

`runner/` 读 `/control/prompt.txt`，有 `/control/reply.txt` 就用它代替回显，逐行处理：

| 行 | 处理 |
| --- | --- |
| `robroz-action:external` / `robroz-action:captcha` | 阻塞行，没接管过就停下写 `tasks/{taskId}/intent`。见 [permissions.md](permissions.md#blocking-lines)。 |
| `robroz-progress:文字` | 写 `tasks/{taskId}/progress`（最后一行为准）。 |
| `robroz-memory:{…}` / `robroz-skill:{…}` | 写 `tasks/{taskId}/proposals.jsonl`。 |
| 其他 | 写进 `tasks/{taskId}/result.txt`。 |

以上特殊行都不进 `result.txt`；没有特殊行时结果一个字节都不变。

### 模型

- `crates/robroz-model` 调 `POST {ROBROZ_MODEL_BASE_URL}/chat/completions`，单次最长等 120 秒。
- 地址和模型名没有配齐时不接模型，任务交付「还没设置模型」，交付卡带 `needsSetup:"model"`。密钥可选：没密钥时请求不带 `Authorization` 头。测试和 CI 设 `ROBROZ_MODEL=echo` 时仍把 prompt 原样写成结果。设置接口优先于环境变量，改完不用重启，见 [configuration.md 的「模型」](configuration.md#model)。
- system 提示 = 固定开场白（`robroz_model::SYSTEM_PROMPT`）+ 记忆段 + 技能段（`host/src/pump.rs:system_extra`）。任务的 prompt 还是用户那一条，不改。
- 密钥不进容器、不进群目录，不出现在日志和报错里。启动校验和变量见 [configuration.md 的「模型」](configuration.md#model)。
- 工具调用循环在 `host/src/tools.rs`，上限和截断见 [permissions.md](permissions.md#the-models-tool-calls)。

### 例行任务

`tick(now)` 和 pump 一起跑。cron 按 UTC 算，用 `croner` 解析。到点时：上一次的任务还在排队，或者还在运行且 `containerUp` 为 true，就跳过（`robroz_policy::should_start_container`）；否则新建一个普通 `queued` 任务。两种情况都把 `nextRunAt` 推到严格晚于现在的下一次，错过的不补跑。`tick` 只建任务，不起容器。暂停只阻止新建，已经在排队或在跑的那次不受影响。成功完成时交付卡 `briefing` 为 true。容器里没有 cron。

### 实时推送

`GET /api/groups/{id}/stream` 不改任何写入路径：宿主机大约每 250 毫秒查一次库，把变化过的卡推出去，没变化每 15 秒发一次保活注释。见 [api.md](api.md#real-time-updates)。

## 卡片和界面层

- `crates/robroz-card`：每种卡的状态那句话、色调、动作（`status_line`、`appearance`、`palette`、`briefing_badge`）。客户端只照着画，不自己定。
- `crates/robroz-tokens`：设计 token，来源 `crates/robroz-tokens/tokens.json`，`cargo run -p robroz-tokens --bin gen` 生成代码。
- `crates/robroz-ui`：客户端共用的界面层，包括卡片视图模型（`cards/`）、会话列表、布局和外观接口 `Skin`（`theme.rs`）。

### 外观层（Skin）

- 组件的颜色、圆角、描边、阴影、材质、控件尺寸、字号和动效全部从 `Skin` 取（`palette`、`radii`、`stroke`、`shadows`、`material`、`controls`、`metrics`、`type_`、`motion`），不写死样式，也不知道自己跑在哪个系统上。
- 现在有浅色和深色两套（`Skin::light` / `Skin::dark`）。设置页（以及连不上宿主的那一页）在 Language / 语言下面有一行 Appearance / 「外观」：System / 「跟随系统」、Light / 「浅色」、Dark / 「深色」，默认 System。选 System 时通过 iced 的 `theme_changes()` 跟着操作系统实时切换，不用重启。选择保存在 `lang` 旁边的 `theme` 文件里，一行，值为 `system`、`light` 或 `dark`；没有这个文件就是 System。底栏没有 Dark 开关。`ROBROZ_THEME=light|dark` 只在 fake 和 gallery 下生效；这些模式下没有这个变量时默认 Light。
- 窄屏：窗口宽度不超过 `NARROW_BREAKPOINT`（480）或触摸输入时，`Skin::for_input` 打开窄屏规则：侧栏收成抽屉，选项按钮换行，路径中间省略，bro 名字最多 8 个字。
- 最小点击区：外观参数 `controls.touch_target` 由外观决定，`robroz-ui` 只读这个值；触摸或窄屏时取 `max(touch_target, TOUCH_FLOOR)`，`TOUCH_FLOOR` 是 44（`Controls::hit`）。界面层自己不写这个数字。
- 第二期的外观库拆分计划见 [roadmap-phase2.md](roadmap-phase2.md)。

## 客户端

- `client/` 是 iced 0.14 桌面程序，三个平台同一套代码。界面行为见 [client.md](client.md)。
- 事件来源在 `client/src/source/`，藏在 `EventSource` trait 后面：
  - `HttpSource`（默认）：增量轮询 `GET /api/groups/{id}/events?since=`，解成 `robroz-contract` 的事件。不认识的 `type` 和解不开的单条事件跳过，不影响其他卡。
  - `stream`：打开的那个群走 SSE。断线后按 1、2、4…最多 30 秒重连，带 `Last-Event-ID`。SSE 断着时每秒轮询一次；连着时轮询放慢到每 10 秒一次兜底。其他群靠轮询和 `GET /api/unread` 更新。
  - `FakeSource`：`--fake` 或 `ROBROZ_FAKE=1`，离线脚本，单测也用它。
- 卡片上的按钮走 `client/src/api.rs:CardApi`。

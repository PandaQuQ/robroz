# openbots

Project Hive。单机私有化的多智能体协同，交互是 IM，不是 IDE。

第一期还没开工。契约字段冻住之后，从宿主机编排开始。这期不做文件树、终端、桌面流、Bot 详情、Webhook、E2B 和多租户。

CI 在有后端模块时跑 `go test ./...`，在有 `client/package.json` 时跑客户端构建。两边都还没有时，对应检查跳过，不算通过节点。

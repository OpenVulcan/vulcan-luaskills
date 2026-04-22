# 任务目标

本次任务目标是将当前项目中 `vldb-controller-client` 的 Git 版本锁定到 `vldb-controller` 最新提交，并结合 `D:\projects\vldb-controller\docs\DOWNSTREAM_IMPACT_REPORT.md` 与对端最近一次提交 diff，完成当前项目与 `vldb-controller` 的接口兼容修复，确保编译、运行时行为与文档预期一致。

# 执行步骤

1. 盘点当前项目中所有 `vldb-controller-client` 依赖与对接入口，确认版本锁定位置、调用方式与潜在受影响模块。
2. 分析 `vldb-controller` 最新提交及其下游影响报告，提炼本项目必须适配的签名变化、权限行为变化、auto-spawn 地址规则变化与错误模型变化。
3. 在当前项目中完成依赖版本更新与代码修复，必要时同步调整文档或注释，保持实现与上游行为一致。
4. 运行针对性的构建、测试或检查命令，验证改动后编译通过，且关键对接逻辑满足预期。
5. 对照本计划逐项复核，补充执行变更总结，并在确认闭环后迁移计划文件到 `docs/completed/20260422/`。

# 技术选型

- 依赖管理以 Cargo 的 git `rev` 固定方式为准，直接对齐 `vldb-controller` 最新提交对应版本。
- 代码分析优先基于 `vulcan-codekit` 建立结构化代码地图，再结合精确源码阅读定位改动点。
- 兼容修复遵循最小必要变更原则，只调整确实受上游变更影响的模块与逻辑，避免无关重构。
- 验证优先使用 `cargo check`、必要的测试或聚焦模块检查，确保本地结果可复现。

# 验收标准

- `Cargo.toml` 与 `Cargo.lock` 中的 `vldb-controller-client` 已锁定到上游最新 Git 提交。
- 当前项目所有受影响代码已完成适配，不再依赖已变更的旧接口行为或旧签名假设。
- 至少完成一次成功的构建校验；若存在测试，则说明已执行范围与结果。
- 计划文件末尾已补充完整的执行变更总结，并在任务完成后迁移到 `docs/completed/20260422/01-VLDB_CONTROLLER_INTEGRATION_FIX.md`。

# 执行变更总结

## 1. 核心修复与调整概述

- 已将 `vldb-controller-client` 的 Git 锁定版本从 `255683d0f2fc9116051afb3730c4e0319ad1d786` 升级到 `b4d93c4e91b27a44226cc0d3cd6efe3220257616`，与 `vldb-controller` 最新提交保持一致。
- 已根据上游最新提交的客户端 API 变更，修复当前项目对私有 `client::BoxError` 路径的引用，改为使用公共导出的 `BoxError`。
- 已同步补充 `space_controller` 对接文档，明确 auto-spawn 与本地可绑定 endpoint 的约束，避免沿用旧行为认知导致接入配置错误。

## 2. 📂 文件变更清单

新增：

- `docs/plan/20260422-01-VLDB_CONTROLLER_INTEGRATION_FIX.md`

修改：

- `Cargo.toml`
- `Cargo.lock`
- `src/host/controller.rs`
- `docs/HOST_DATABASE_PROVIDER_GUIDE.md`
- `docs/FFI_INTEGRATION_GUIDE.md`

删除：

- 无

## 3. 💻 关键代码调整详情

- `Cargo.toml`：更新 `vldb-controller-client` 的 git `rev` 到上游最新提交。
- `Cargo.lock`：通过 `cargo update -p vldb-controller-client` 重新锁定依赖元数据。
- `src/host/controller.rs`：将异步控制器调用约束中的错误类型改为上游公开导出的 `BoxError`，消除新版 SDK 中 `client` 模块私有别名导致的编译失败。
- `docs/HOST_DATABASE_PROVIDER_GUIDE.md` 与 `docs/FFI_INTEGRATION_GUIDE.md`：补充 `space_controller.auto_spawn=true` 时 endpoint 需为本地可绑定地址的说明，并注明远端 controller 场景应关闭 auto-spawn。

## 4. ⚠️ 遗留问题与注意事项

- 当前项目未直接使用上游本次变更中 `endpoint_url()` / `bind_addr()` 的公开签名，因此未产生额外编译适配点。
- 当前项目也未直接依赖上游 FFI 句柄层实现，因此本轮无需在本仓库额外调整 FFI 代码。
- 若宿主未来配置 `space_controller.endpoint` 为远端主机名或远端服务地址，请确保 `auto_spawn=false`，否则将触发上游最新版本对本地 bind 地址转换的限制。
- 本轮已验证 `cargo check` 与 `cargo test` 均通过，可作为当前对接修复完成依据。

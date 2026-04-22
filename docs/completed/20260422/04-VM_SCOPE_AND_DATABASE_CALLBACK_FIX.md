# 任务目标

本次任务目标是修复三项新增审查问题：一是 `LuaVmRequestScopeGuard` 在重置失败时仍可能把脏 VM 放回池中；二是 `vulcan.call` 的嵌套上下文切换与恢复不是异常安全的；三是数据库 provider 回调当前仍是进程级全局单例，导致同一进程中的多个引擎实例存在回调串线风险。

# 执行步骤

1. 复核 `src/runtime/engine.rs` 中 `LuaVmLease`、`LuaVmRequestScopeGuard` 与 `vulcan.call` 的上下文切换路径，确认哪些失败路径仍会让污染状态回流到 VM 池。
2. 修改 Lua VM 池租约与作用域守卫逻辑，引入“重置失败即淘汰当前 VM”的处理，保证任何重置失败都不会把已损坏实例重新放回池中。
3. 为 `vulcan.call` 增加嵌套调用专用的上下文守卫，确保切换失败、被调 skill 抛错、恢复阶段出错时都能尽力恢复外层执行上下文。
4. 重构数据库 provider 回调存储模型，将进程级全局注册表改为“全局默认回调 + 引擎创建时快照”，并把快照沿着 `LuaEngine -> Sqlite/LanceDb host -> binding` 传递，消除多引擎串线。
5. 补充针对性测试，至少覆盖 VM 重置失败后的淘汰逻辑、`vulcan.call` 的异常恢复路径，以及数据库回调快照隔离行为。
6. 运行构建与测试，对照计划逐项验收；完成后补写执行变更总结，并将计划文件迁移到 `docs/completed/20260422/`。

# 技术选型

- Lua VM 池继续沿用现有对象池模型，但新增显式淘汰能力；一旦检测到实例已不可信，直接将其移出池并允许后续按需补建。
- `vulcan.call` 的修复采用 RAII 守卫，优先保证嵌套上下文恢复的异常安全，而不是继续依赖尾部手工对称回滚。
- 数据库 provider 回调采用“创建时快照”方案，保持现有外部 FFI 注册 API 不变，同时把实例隔离下沉到引擎私有状态。
- 修改范围控制在 `runtime/engine`、`host/database`、`providers/*` 与必要的 FFI 接入层，不扩散到本轮之外的 FFI 安全契约问题。

# 验收标准

- `LuaVmRequestScopeGuard` 在入口重置失败与退出重置失败时都不会把脏 VM 返回对象池。
- `vulcan.call` 在切换阶段、被调执行阶段和恢复阶段出错时，外层 Lua 上下文仍能被恢复到调用前状态，或在无法恢复时显式淘汰该 VM。
- SQLite/LanceDB provider 回调对不同引擎实例实现隔离，后注册回调不会影响既有引擎的后续数据库请求。
- 至少完成一次成功的 `cargo test` 与一次成功的 `cargo check`；若静态检查仍受历史问题影响，需要在总结中如实记录。
- 计划文件末尾已追加完整执行变更总结，并在任务闭环后迁移至 `docs/completed/20260422/04-VM_SCOPE_AND_DATABASE_CALLBACK_FIX.md`。

# 执行变更总结

## 1. 核心修复与调整概述

- 修复了池化 Lua VM 在请求级重置失败时仍可能回流对象池的问题，为 `LuaVmLease` 与 `LuaVmPool` 增加显式淘汰能力，并让 `LuaVmRequestScopeGuard` 在入口/出口清理失败时直接退役当前 VM。
- 为 `vulcan.call` 引入嵌套调用上下文守卫，完整快照并恢复 `vulcan` 核心表、请求上下文、内部执行标记、文件上下文、依赖路径以及数据库绑定，覆盖切换失败、被调技能抛错、恢复失败等路径。
- 将数据库 provider 回调从“运行时每次读取进程级全局注册表”改为“引擎创建时捕获一次快照并沿宿主链路下传”，消除了多引擎实例之间的 SQLite/LanceDB 回调串线风险。
- 补充了针对性回归测试，覆盖 VM 入口重置失败淘汰、退出重置失败淘汰、`vulcan.call` 异常恢复以及数据库回调快照隔离。

## 2. 📂文件变更清单

- 修改：`src/runtime/engine.rs`
- 修改：`src/host/database.rs`
- 修改：`src/providers/sqlite.rs`
- 修改：`src/providers/lancedb.rs`
- 新增：无
- 删除：无

## 3. 💻关键代码调整详情

- 在 `src/runtime/engine.rs` 中新增 `VulcanCoreModuleState` 与 `LuaNestedCallScopeGuard`，把 `vulcan.call` 的上下文切换改成 RAII 管理，并在恢复时重新挂回 `vulcan/runtime/context/deps` 等核心表结构。
- 在 `src/runtime/engine.rs` 中重构 `LuaVmRequestScopeGuard`，由直接持有 `Lua` 改为持有 `LuaVmLease`；新增 `LuaVmLease::discard` 与 `LuaVmPool::discard`，保证损坏 VM 不会再次被借出。
- 在 `src/runtime/engine.rs` 中把数据库 provider 快照挂到 `LuaEngine`，并在初始化 `SqliteSkillHost` / `LanceDbSkillHost` 时一并传入。
- 在 `src/providers/sqlite.rs` 与 `src/providers/lancedb.rs` 中为 host/binding 增加 `provider_callbacks` 字段，宿主回调模式改为只使用引擎私有快照，而不再直接读取进程级全局回调表。
- 在 `src/host/database.rs` 中新增 `RuntimeDatabaseProviderCallbacks` 快照结构与分发方法，并补充多快照隔离测试，验证先创建的快照不会被后续注册覆盖。

## 4. ⚠️遗留问题与注意事项

- 本轮目标内的问题已经完成修复，`cargo check`、`cargo test --lib`、`cargo test` 均已通过。
- 仓库里仍存在本轮之外的既有未提交改动，包括 `Cargo.toml`、`Cargo.lock`、`src/ffi.rs`、`src/ffi_standard.rs`、`src/host/controller.rs`、`src/providers/sqlite.rs` 等文件的历史工作区变化；本次修复没有覆盖这些与当前目标无关的内容。
- 标准 FFI 安全契约与历史 Clippy 债务仍未在本轮处理范围内，后续若继续做全盘质量治理，建议单独开任务闭环。

# 任务目标

修正 `vulcan-luaskills` 在 space-controller 模式下错误复用 `binding_tag` 作为 controller `binding_id` 的实现，避免不同客户端实例在同一 `space_id` 下争抢同一 binding 槽位，恢复 controller 设计中的“资源共享、绑定隔离、按客户端回收”语义。

# 详细执行步骤

1. 梳理当前 `RuntimeDatabaseBindingContext`、SQLite provider、LanceDB provider 与 controller bridge 的调用链，确认 `binding_tag`、`space_id`、`binding_id` 的现状语义与落点。
2. 设计并引入 controller 专用 binding 标识生成方案，使其与稳定 `binding_tag` 解耦，同时保持 `binding_tag` 继续承担日志、诊断与宿主管理标签职责。
3. 修改 SQLite 与 LanceDB 在 controller 模式下的启用阶段与后续数据面调用，统一改用新的 controller binding 标识。
4. 检查 bridge 生命周期与 controller client session 关系，确保新的 binding 标识在当前 bridge 生命周期内稳定、跨 bridge 实例唯一。
5. 补充或更新必要测试与文档说明，覆盖共享资源复用、绑定唯一性与已有诊断字段不变这三类关键行为。
6. 完成逐项自检，确认实现、文档与验证结果一致后补充执行变更总结并归档计划文件。

# 技术选型

- 保留 `binding_tag` 作为稳定业务标签，不改变其 `{space_label}-{skill_id}` 组成规则。
- 新增 controller 专用 binding 标识生成逻辑，优先复用 controller 当前 client session 语义；若现有 SDK 公共接口不便直接透出，则在 bridge 层缓存或派生与当前 controller client 实例严格绑定的唯一标识。
- SQLite 与 LanceDB 共用同一套 controller binding 标识生成策略，避免两个 provider 出现语义分叉。
- 仅调整 space-controller 模式相关逻辑，不影响 dynamic library 与 host callback 模式。

# 验收标准

- 不同客户端实例访问同一 `space_id` 下相同物理数据库资源时，不再因为复用同一个 `binding_id` 而发生 owner 冲突。
- controller 仍可基于 `db_path` 或 `default_db_path/db_root` 复用底层资源实例。
- `binding_tag` 在诊断输出、宿主管理与文档中的稳定标签语义保持不变。
- SQLite 与 LanceDB 的 controller 模式实现保持一致。
- 相关代码能够通过已有测试或新增验证，至少覆盖关键调用链与核心语义校验。

## 执行变更总结

### 1. 核心修复与调整概述

- 已修正 `space_controller` 模式下错误将稳定 `binding_tag` 直接当作 controller `binding_id` 的实现。
- 已在 controller bridge 层引入基于当前 controller client 会话作用域的 binding id 派生逻辑，统一生成 `"{binding_tag}@{binding_scope_id}"` 形式的客户端隔离 binding 标识。
- 已同步改造 SQLite 与 LanceDB 的启用阶段及全部 controller 数据面调用链，确保同一 bridge 生命周期内使用一致的 controller binding id。
- 已补充文档说明，明确 `binding_tag` 在 `space_controller` 模式下仅保留稳定标签语义，不再等同于 controller `binding_id`。

### 2. 📂文件变更清单

- 新增：
  - 无
- 修改：
  - `src/host/controller.rs`
  - `src/providers/sqlite.rs`
  - `src/providers/lancedb.rs`
  - `docs/HOST_DATABASE_PROVIDER_GUIDE.md`
  - `README.md`
  - `docs/plan/20260424-03-CONTROLLER_BINDING_ID_SEMANTICS_FIX.md`
- 删除：
  - 无

### 3. 💻关键代码调整详情

- 在 `LuaRuntimeSpaceControllerBridge` 中新增 `binding_scope_id` 缓存字段，并在 `connect()` 成功后通过 controller SDK 的 `list_clients()` 解析当前可见 `client_session_id`，作为本 bridge 的绑定作用域。
- 新增 `controller_binding_id_for_binding()` 与纯函数 `build_controller_binding_id()`，将 controller binding id 的生成逻辑统一收敛到 bridge 层。
- 将 SQLite provider 中所有 controller 数据面调用使用的 `binding_id` 全部改为通过 bridge 派生，替换此前直接复用 `binding_tag` 的实现。
- 将 LanceDB provider 中启用阶段与全部 controller 数据面调用使用的 `binding_id` 同步改为 bridge 派生，保证与 SQLite 行为一致。
- 新增 bridge 级单元测试，验证 controller binding id 会保留稳定 `binding_tag`，同时附加客户端隔离后缀。

### 4. ⚠️遗留问题与注意事项

- 当前 controller binding id 的作用域来源于 bridge 初始化阶段可见的 controller client session；它在同一 bridge 生命周期内保持稳定，符合 SDK 的自动重放模型，但不会在 session 恢复后主动改写已派生的 binding id。
- 本次改动刻意不改变 `binding_tag` 的格式与宿主管理语义，因此 `host_callback` 模式下基于 `binding_tag` 的数据库路由行为保持不变。
- 已完成 `cargo check`、`cargo test controller_binding_id_preserves_tag_and_adds_scope_suffix --lib` 与 `cargo test host::controller --lib` 验证；尚未新增端到端多客户端集成测试，如后续需要可再补 controller 实例级回归用例。

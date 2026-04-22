# 任务目标

本次任务目标是针对 FFI 引擎并发模型与 Lua VM 池上下文隔离问题执行深度修复，重点解决三项问题：一是 `src/ffi.rs` 中全局引擎注册表锁跨引擎调用持有，导致宿主重入 FFI 时可能死锁；二是 `src/runtime/engine.rs` 中 `call_skill` 的请求级上下文清理不是异常安全的，可能污染复用 VM；三是 `run_lua` 写入的 `__runlua_args` 全局变量未清理，导致跨请求参数泄漏。

# 执行步骤

1. 复核 `src/ffi.rs` 与 `src/ffi_standard.rs` 当前的引擎注册、查找、释放路径，设计“注册表短锁 + 引擎句柄独立互斥”的并发模型，避免在执行引擎操作时继续持有全局注册表锁。
2. 修改 FFI 引擎注册表的数据结构与访问辅助函数，确保只在查表阶段持有全局锁，并在读取到目标引擎句柄后立即释放。
3. 在 `src/runtime/engine.rs` 中抽取请求级 Lua 上下文守卫，统一接管 `request`、内部执行标记、文件上下文、依赖上下文、LanceDB/SQLite 绑定与 `__runlua_args` 的安装与恢复。
4. 将新的上下文守卫应用到 `call_skill` 与 `run_lua`，确保任意早退、编译错误、执行错误路径都能自动回滚到借出前状态。
5. 为本轮修复补充或调整测试，至少覆盖 VM 上下文不泄漏与核心执行路径可通过构建/测试验证。
6. 运行构建与测试，对照计划逐项验收；完成后补写执行变更总结，并将计划文件迁移到 `docs/completed/20260422/`。

# 技术选型

- FFI 注册表改为“全局注册表只管理句柄映射，引擎实例由独立 `Arc<Mutex<...>>` 管理”的方式，以降低锁作用域并保留现有同步 API 形态。
- Lua VM 请求级上下文恢复采用 RAII 守卫，优先保证异常安全与状态隔离，而不是继续依赖手工对称清理。
- `run_lua` 的参数注入与清理纳入同一守卫统一管理，避免再出现局部补丁式清理遗漏。
- 代码修改遵循最小必要重构原则，只修复与本轮问题直接相关的并发与隔离边界，不扩散到本轮之外的历史 FFI lint 债务。

# 验收标准

- FFI 引擎操作执行期间不再持有全局注册表锁，宿主回调重入时不会因为全局注册表互斥锁产生自锁。
- `call_skill` 在任意上下文安装或执行失败路径下，都不会把脏的请求级上下文留在池化 Lua VM 中。
- `run_lua` 在成功与失败路径下都会清理或恢复 `__runlua_args`，后续请求不可读取到上一请求残留参数。
- 至少完成一次成功的 `cargo test`，并完成一次针对性构建校验；若静态检查仍受历史问题影响，需要在总结中如实记录。
- 计划文件末尾已追加完整的执行变更总结，并在任务闭环后迁移至 `docs/completed/20260422/03-FFI_ENGINE_AND_VM_CONTEXT_DEEP_FIX.md`。

# 执行变更总结

## 1. 核心修复与调整概述

- 已重构 FFI 引擎句柄注册模型，将原先“全局注册表锁包住整个引擎调用”的路径改为“注册表短锁 + 引擎独立互斥”，并额外加入同线程同引擎重入检测，避免宿主回调重入时出现静默死锁。
- 已为池化 Lua VM 引入统一的请求级作用域守卫，进入执行前先归一化到干净基线，退出时无论成功、失败还是中途 `?` 早退，都会自动回滚 `vulcan` 请求上下文、内部执行标记、文件上下文、依赖上下文、数据库绑定和 `__runlua_args`。
- 已把新的 VM 作用域守卫接入 `call_skill`、`run_lua` 与 `render_help_payload`，同时补充回归测试，确保本轮修复不仅覆盖用户指出的两条主路径，也顺手消除了帮助渲染的同类风险点。

## 2. 📂 文件变更清单

新增：

- `docs/completed/20260422/03-FFI_ENGINE_AND_VM_CONTEXT_DEEP_FIX.md`

修改：

- `src/ffi.rs`
- `src/ffi_standard.rs`
- `src/runtime/engine.rs`

删除：

- 无

## 3. 💻 关键代码调整详情

- `src/ffi.rs`
  - `FfiEngineSlot` 改为持有 `Arc<Mutex<LuaEngine>>`，新增 `clone_engine_handle`，确保执行引擎操作前就释放全局注册表锁。
  - 新增 `ActiveFfiEngineGuard` 与线程局部活动引擎集合，对同线程同引擎重入访问返回明确错误，避免卡死。
  - 补充两项 FFI 回归测试，分别验证“操作期间可再次获取注册表锁”和“同线程重入返回错误而非死锁”。
- `src/ffi_standard.rs`
  - 标准 ABI 的引擎创建路径同步改用新的 `FfiEngineSlot::new`，保持 JSON ABI 与标准 ABI 的引擎句柄模型一致。
- `src/runtime/engine.rs`
  - 新增 `clear_runlua_args_global`、`reset_pooled_vm_request_scope` 与 `LuaVmRequestScopeGuard`，统一处理池化 VM 的请求级状态隔离。
  - `call_skill`、`run_lua`、`render_help_payload` 全部改为“执行结果 + 清理结果”双结果合并返回，正常路径可显式暴露清理失败，异常路径仍由守卫兜底。
  - 新增三项回归测试，验证作用域守卫在提前失败时仍能回滚，以及 `run_lua` 在成功/失败后都会清理 `__runlua_args`。

## 4. ⚠️ 遗留问题与注意事项

- `cargo test --lib`、`cargo test` 与 `cargo check` 已通过，本轮修复覆盖的执行路径已经完成验证。
- `cargo clippy --all-targets --all-features -- -D warnings` 仍未通过，当前主要失败点仍集中在既有 FFI 安全契约、缺失 `# Safety` 文档与多项历史 lint 债务，并非本轮新引入回归。
- 当前工作区仍包含本轮之外的既有改动，例如 `Cargo.toml`、`Cargo.lock`、`docs/FFI_INTEGRATION_GUIDE.md`、`docs/HOST_DATABASE_PROVIDER_GUIDE.md`、`src/host/controller.rs` 与 `src/providers/sqlite.rs`，本次未覆盖或回退这些内容。

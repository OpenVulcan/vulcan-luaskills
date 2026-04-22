# 任务目标

本次任务目标是对 `vldb-controller` 升级后的遗留问题执行深度修复，重点解决两项高优先级问题：一是 `src/host/controller.rs` 中 controller 桥接在 `Tokio current_thread runtime` 下可能触发 panic 的执行路径；二是 `space_controller` 相关对接文档中把上游硬约束写成软建议的问题，确保实现、测试与文档三者完全一致。

# 执行步骤

1. 复核当前 controller 桥接逻辑，确认 `Handle::try_current() + block_in_place` 的风险边界，并设计兼容同步线程、`multi_thread runtime` 与 `current_thread runtime` 的统一执行方案。
2. 修改 `src/host/controller.rs` 中的执行桥接逻辑，避免在不满足 `block_in_place` 前提时触发 panic，并保持现有调用方接口不变。
3. 为桥接逻辑补充针对性测试，覆盖至少一种已处于 Tokio runtime 的调用路径，确保不会因 runtime flavor 差异产生崩溃。
4. 修正 `docs/FFI_INTEGRATION_GUIDE.md` 与相关文档中关于 `space_controller.auto_spawn` 的表述，将软建议升级为与上游实际行为一致的强规则。
5. 运行构建、测试与必要的静态检查，对照计划逐项验收；完成后补写执行变更总结，并将计划文件迁移至 `docs/completed/20260422/`。

# 技术选型

- controller 桥接优先采用与宿主 Tokio runtime 解耦的安全执行路径，避免依赖 `block_in_place` 的 runtime flavor 前提。
- 测试以最小必要范围覆盖 controller 桥接关键路径，优先验证“已处于 Tokio runtime 中调用”的安全性。
- 文档修复遵循“实现即规范”的原则，以 `vldb-controller-client` 最新行为作为唯一事实来源。
- 代码修改遵循最小必要变更原则，不做与本轮风险无关的重构。

# 验收标准

- `src/host/controller.rs` 不再在 `Tokio current_thread runtime` 场景下因 `block_in_place` 路径触发 panic。
- 至少补充一项能验证桥接逻辑安全性的测试，并在本地执行通过。
- `docs/FFI_INTEGRATION_GUIDE.md` 及相关文档中 `auto_spawn + endpoint` 的约束表述已改为强规则，不再使用会弱化约束的措辞。
- 至少完成一次成功的 `cargo test` 与一次针对性构建校验；若静态检查范围受历史问题影响，需要如实记录。
- 计划文件末尾已追加完整的执行变更总结，并在任务闭环后迁移到 `docs/completed/20260422/02-CONTROLLER_BRIDGE_DEEP_FIX.md`。

# 执行变更总结

## 1. 核心修复与调整概述

- 已重构 controller 桥接执行路径，移除“只要处于 Tokio runtime 就直接走 `block_in_place`”的假设，改为在异步宿主场景下把 future 分发到桥接自身 runtime 的 worker 线程执行，并同步回收结果。
- 已为桥接逻辑补充回归测试，明确验证在 `current_thread Tokio runtime` 中调用时不会再触发 panic，同时保留同步调用路径的正确性验证。
- 已将 `space_controller.auto_spawn` 的 endpoint 约束从软建议升级为硬规则，文档与上游 `vldb-controller-client` 最新行为保持一致。

## 2. 📂 文件变更清单

新增：

- `docs/plan/20260422-02-CONTROLLER_BRIDGE_DEEP_FIX.md`

修改：

- `src/host/controller.rs`
- `docs/FFI_INTEGRATION_GUIDE.md`
- `docs/HOST_DATABASE_PROVIDER_GUIDE.md`

删除：

- 无

## 3. 💻 关键代码调整详情

- `src/host/controller.rs`
  - 新增 `run_future_on_bridge_runtime` 与 `run_future_on_bridge_runtime_handle`，统一封装桥接 runtime 执行逻辑。
  - `run_controller_operation_with_client` 现在不再使用 `tokio::task::block_in_place`，从而规避 `current_thread runtime` 下的 panic 风险。
  - 新增两项测试，分别覆盖同步调用方与 `current_thread Tokio runtime` 调用方。
  - 顺带清理 `Drop` 中无意义的 `let _ = runtime.block_on(...)` 写法，避免局部 lint 噪音。
- `docs/FFI_INTEGRATION_GUIDE.md`
  - 将 `auto_spawn=true` 时 endpoint 的描述改为“必须使用本地可绑定地址格式”。
  - 将远端 controller / 远端主机名端点场景改为“必须关闭 `auto_spawn`”。
- `docs/HOST_DATABASE_PROVIDER_GUIDE.md`
  - 同步修正文档语气强度，确保两份对接文档对同一约束的表述完全一致。

## 4. ⚠️ 遗留问题与注意事项

- `cargo test host::controller --lib` 与 `cargo test` 已通过，本轮深度修复覆盖点已完成验证。
- `cargo clippy --all-targets --all-features -- -D warnings` 仍未通过，但失败项主要是仓库既有 FFI 安全文档、原始指针 API 边界与若干历史 lint 问题，并非本轮修复新引入。
- 当前工作区仍包含本轮之外的既有改动，例如 `Cargo.toml`、`Cargo.lock` 与 `src/providers/sqlite.rs`，本次未覆盖或回退这些修改。

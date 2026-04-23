# 任务目标

为隔离 `run_lua` / `vulcan.runtime.lua.exec` 执行链增加独立虚拟机池能力，默认提供最小 1、最大 4、空闲冷却销毁时间的默认配置，并允许宿主按与常规 VM 池相同的参数结构进行覆盖配置。

## 执行步骤

1. 梳理当前普通 `run_lua` 与隔离 `luaexec` 的执行路径，确认真正缺失池化的是哪条链路。
2. 设计宿主运行时配置字段，保证与现有 `LuaVmPoolConfig` 参数语义一致，并提供默认值。
3. 在运行时引擎中增加独立的 `run_lua` VM 池，确保隔离执行优先复用该池，而不是每次新建 VM。
4. 保持普通 skill VM 池与隔离 `run_lua` VM 池职责分离，避免上下文污染和配置耦合。
5. 补充单元测试，覆盖默认配置、宿主自定义配置以及池化复用后的基本行为。
6. 同步更新 README、Skill 开发手册与 FFI/宿主文档，说明默认值、配置方式和生效范围。
7. 完成验证后追加执行变更总结并归档计划文件。

## 技术选型

- 复用现有 `LuaVmPoolConfig` 结构语义，避免再引入一套近似配置模型。
- 在 `LuaRuntimeHostOptions` 中新增独立的 `runlua_pool_config`，由宿主显式覆盖；未配置时使用默认值。
- 默认值采用：
  - `min_size = 1`
  - `max_size = 4`
  - `idle_ttl_secs = 60`
- 隔离执行池仅服务于 `vulcan.runtime.lua.exec` / 内部 runlua 隔离执行，不改变普通 `call_skill` / `run_lua` 主池配置。

## 验收标准

- 隔离 `run_lua` 执行链不再默认每次新建 Lua VM。
- 宿主可以像配置常规 VM 池一样配置隔离执行池。
- 宿主不配置时，默认使用 `1/4/60`。
- 现有普通 VM 池行为保持不变。
- `cargo test -q` 通过。

---

## 执行变更总结

### 1. 核心修复与调整概述

- 为隔离 `vulcan.runtime.lua.exec` 执行链新增了独立的 `runlua_pool`，不再沿用“每次执行都新建隔离 VM”的旧行为。
- 宿主运行时选项新增 `runlua_pool_config`，参数语义与常规 `LuaVmPoolConfig` 保持一致；未配置时默认采用 `min_size=1 / max_size=4 / idle_ttl_secs=60`。
- 普通 skill VM 池与隔离 `runlua` VM 池职责保持分离，普通 `call_skill / run_lua` 主池行为未被改变。
- 标准 C ABI、公共 JSON FFI 示例和文档已经同步支持新字段，宿主可以按原有配置方式直接覆盖专用池参数。

### 2. 📂文件变更清单

#### 修改

- `src/runtime/engine.rs`
- `src/host/options.rs`
- `src/ffi_standard.rs`
- `src/ffi.rs`
- `include/vulcan_luaskills_ffi.h`
- `README.md`
- `docs/FFI_INTEGRATION_GUIDE.md`
- `docs/FFI_HOST_CHECKLIST.md`
- `docs/SKILL_DEVELOPER_MANUAL.md`
- `examples/ffi/c/demo.c`
- `examples/ffi/python/demo.py`
- `examples/ffi/python/lifecycle_demo.py`
- `examples/ffi/python/query_demo.py`
- `examples/ffi/demo_runtime/run_python_install_demo.py`
- `examples/ffi/host_provider_demo/run_python_host_provider_demo.py`
- `examples/ffi/go/demo.go`
- `examples/ffi/go/lifecycle_demo/main.go`
- `examples/ffi/go/query_demo/main.go`
- `examples/ffi/typescript/demo.ts`
- `examples/ffi/typescript/lifecycle_demo.ts`
- `examples/ffi/typescript/query_demo.ts`

#### 新增

- 无

#### 删除

- 无

### 3. 💻关键代码调整详情

- `LuaRuntimeHostOptions` 新增 `runlua_pool_config`，并新增 `LuaRuntimeRunLuaPoolConfig` 结构，用于承载宿主对隔离 `runlua` 池的覆盖配置。
- `LuaEngine` 新增 `runlua_pool` 字段，并在引擎构造、重置和加载预热阶段建立独立池；默认配置通过 `default_runlua_vm_pool_config()` 统一收口。
- `create_vm` 与 `create_runlua_vm` 分离：普通 VM 继续承载 `vulcan.runtime.lua.exec` 桥接，隔离 `runlua` VM 只注册执行所需的 `vulcan`、skill 函数与 `vulcan.call`。
- `populate_vulcan_luaexec_bridge` 改为直接调度 `execute_runlua_request_inline_with_runtime(...)`，并通过 `acquire_runlua_vm(...)` 复用专用池中的隔离 VM。
- 新增测试覆盖：
  - 默认 `runlua` 池配置
  - 宿主覆盖配置
  - `execute_runlua_request_json_inline(...)` 连续执行时的池复用行为
- 标准 C ABI `FfiLuaRuntimeHostOptions` 与跨语言示例均增加 `runlua_pool_config` 字段，保持 FFI 接口与宿主配置同步。

### 4. ⚠️遗留问题与注意事项

- 当前专用池只作用于 `vulcan.runtime.lua.exec` 隔离执行链；后续如需继续收敛宿主契约，应考虑把历史遗留的外部执行器配置面彻底清理。
- TypeScript 示例已同步字段，但当前环境没有 `tsc` 与 `koffi` 依赖，因此本轮只完成了代码对齐，没有做真实编译验证。
- 这次能力属于 ABI 形状扩展；目前仓库没有第三方实际接入，因此可以接受直接扩展。后续若要冻结对外 ABI，应把类似字段变更纳入版本化策略。

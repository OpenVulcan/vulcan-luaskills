# 任务目标

正式废弃历史遗留的 `luaexec_program` 配置项，统一 `vulcan.runtime.lua.exec` 的执行模型为“当前进程内 + 独立 runlua VM 池”，避免宿主继续误解该字段仍然可用。

## 执行步骤

1. 梳理 `luaexec_program` 在宿主配置、标准 C ABI、示例与文档中的残留位置。
2. 删除 Rust 宿主配置中的 `luaexec_program` 字段，并同步移除标准 C ABI/FFI 对应结构字段与解析逻辑。
3. 清理 Python / Go / TypeScript / C 示例中的旧字段赋值与结构定义，确保所有接入样例统一到新语义。
4. 更新 README、FFI 指南、Skill 开发手册与计划文档，明确 `vulcan.runtime.lua.exec` 现在始终走进程内独立池，不再支持外部执行器路径。
5. 运行格式化、语法检查与测试，补齐必要的兼容性修正。
6. 在计划末尾追加执行变更总结，并归档到 `docs/completed/20260423/`。

## 技术选型

- 不保留兼容壳，也不新增废弃告警；直接删除 `luaexec_program`，避免继续暴露一个不会生效的宿主契约。
- 以 `runlua_pool_config` 作为唯一的 `luaexec` 性能/资源控制入口。
- 保持 `vulcan.runtime.lua.exec` 语义单一：始终为当前进程内的隔离执行，不再保留子进程分支。

## 验收标准

- `LuaRuntimeHostOptions` 与标准 C ABI 中不再存在 `luaexec_program` 字段。
- 所有官方 FFI 示例不再声明或赋值 `luaexec_program`。
- README、FFI 文档与 Skill 开发手册中不再暗示存在外部子进程 `luaexec` 模型。
- `cargo fmt`、`gofmt`、`python -m py_compile`、`cargo test -q` 通过。

---

## 执行变更总结

### 1. 核心修复与调整概述

- 正式删除了历史遗留的 `luaexec_program` 配置项，统一 `vulcan.runtime.lua.exec` 的执行模型为“当前进程内 + 独立 runlua VM 池”。
- 宿主配置、标准 C ABI、JSON/标准 FFI 示例与说明文档已经同步清理，不再保留“外部 `luaexec` 执行器路径”这一误导性契约。
- 保留 `runlua_pool_config` 作为隔离 `luaexec` 唯一的资源调优入口，继续沿用默认 `1/4/60`。

### 2. 📂文件变更清单

#### 修改

- `src/host/options.rs`
- `src/ffi_standard.rs`
- `src/ffi.rs`
- `include/vulcan_luaskills_ffi.h`
- `README.md`
- `docs/FFI_INTEGRATION_GUIDE.md`
- `docs/SKILL_DEVELOPER_MANUAL.md`
- `docs/completed/20260423/03-RUNLUA_VM_POOL.md`
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

- 从 `LuaRuntimeHostOptions` 中移除了 `luaexec_program` 字段，宿主层不再提供外部执行器路径配置。
- 从标准 C ABI `FfiLuaRuntimeHostOptions` 与头文件 `vulcan_luaskills_ffi.h` 中同步删除对应字段，并清理 Rust 侧解析逻辑。
- 所有官方 C / Python / Go / TypeScript FFI 示例都已移除 `luaexec_program` 的结构定义、初始化赋值与示例载荷。
- README、FFI 指南和 Skill 开发手册统一明确：`vulcan.runtime.lua.exec` 现在始终在当前进程内执行，并由 `runlua_pool_config` 控制隔离池参数。

### 4. ⚠️遗留问题与注意事项

- 当前 `vulcan.runtime.lua.exec` 的唯一性能/资源调优入口是 `runlua_pool_config`；如果未来还要扩展更细粒度策略，应继续围绕进程内独立池模型设计。
- TypeScript 示例已经同步删掉字段，但当前环境没有 `tsc` 与 `koffi`，因此这轮仍未完成 TypeScript 真实编译验证。
- 这次属于 ABI 形状删减；当前仓库没有第三方实际接入，因此可以直接执行。后续若正式冻结 ABI，应避免在稳定版本中频繁做这类字段删除。

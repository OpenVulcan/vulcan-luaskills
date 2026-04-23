## 任务目标

为宿主层增加一个明确的 skill 忽略列表能力，使宿主可以主动关闭某些已集成或默认安装的 skill，避免在宿主已经切换到更强的原生、gRPC 或 VMM 能力实现时，仍然加载冲突的 LuaSkill 包。

## 执行步骤

1. 梳理当前 skill 加载流程，确认跳过 skill 的最早安全位置。
2. 在宿主选项中增加稳定的忽略列表字段，并明确其匹配对象为 skill 目录派生出的 `skill_id`。
3. 在加载阶段命中忽略列表时直接跳过 skill，确保依赖安装、数据库绑定与 entry 注册都不会发生。
4. 为忽略列表补充单元测试，覆盖被忽略 skill 不进入 entry registry 的行为。
5. 同步更新 README 与 Skill 开发手册，说明该能力属于宿主强制策略，不是 skill 自判定机制。
6. 完成验证后追加执行变更总结并归档计划文件。

## 技术选型

- 字段放在 `LuaRuntimeHostOptions`，因为这是宿主运行时级策略，不属于请求上下文。
- 使用 `Vec<String>` 表达忽略列表，保持 FFI / JSON / 标准 ABI 都容易传输。
- 加载阶段按目录派生出的 `skill_id` 匹配，避免依赖 `skill.yaml` 中的展示名称。
- 命中忽略后在依赖安装与数据库绑定之前返回，避免产生资源副作用。

## 验收标准

- 宿主可以通过配置忽略一个或多个 skill。
- 被忽略的 skill 不触发依赖准备、SQLite/LanceDB 绑定与 entry 注册。
- 未配置忽略列表时，现有加载行为保持不变。
- `cargo test -q` 通过。
- 文档说明清楚该能力的使用场景、匹配规则与边界。

## 执行变更总结

### 1. 核心修复与调整概述

- 在宿主运行时配置中新增 `ignored_skill_ids`，让宿主可以按目录派生出的 `skill_id` 强制跳过某些 skill。
- 在 `load_from_roots` 的早期加载阶段加入忽略判定，命中后直接跳过，避免触发依赖准备、SQLite/LanceDB 绑定与 entry 注册。
- 同步标准 C ABI、JSON FFI 默认反序列化、Python / Go / TypeScript / C 示例的 host options 结构，避免新增字段导致跨语言结构体错位。
- 补充文档说明，明确该能力是宿主强制策略，不是自动 capability 反判定，也不是 skill 自声明启停机制。

### 2. 📂文件变更清单

- 新增：
  - `docs/plan/20260423-02-HOST_SKILL_IGNORE_LIST.md`
- 修改：
  - `src/host/options.rs`
  - `src/runtime/engine.rs`
  - `src/ffi.rs`
  - `src/ffi_standard.rs`
  - `include/vulcan_luaskills_ffi.h`
  - `README.md`
  - `docs/SKILL_DEVELOPER_MANUAL.md`
  - `docs/FFI_INTEGRATION_GUIDE.md`
  - `docs/FFI_HOST_CHECKLIST.md`
  - `docs/HOST_DATABASE_PROVIDER_GUIDE.md`
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
- 删除：
  - 无

### 3. 💻关键代码调整详情

- `LuaRuntimeHostOptions` 新增 `ignored_skill_ids: Vec<String>`，并为 serde 反序列化提供默认值，保证旧 JSON 配置缺字段时仍可解析。
- `LuaEngine::is_host_ignored_skill` 负责按宿主忽略列表匹配目录派生出的 `skill_id`。
- `LuaEngine::load_from_roots` 在构造 `SkillManager`、准备依赖与加载 skill 之前执行忽略判定，确保被忽略 skill 不产生资源副作用。
- `FfiLuaRuntimeHostOptions` 新增 `ignored_skill_ids` 与 `ignored_skill_ids_len`，标准 C ABI 可直接传入宿主忽略列表。
- 新增 `load_from_roots_skips_host_ignored_skill_before_resource_setup` 回归测试，验证被忽略 skill 不进入 `skills`、不进入 `entry_registry`，也不会创建同级依赖、状态或数据库目录。

### 4. ⚠️遗留问题与注意事项

- 当前能力只做宿主显式忽略列表，不做自动 capability 反判定；公共能力字典和 entry/skill 级 activation 仍适合作为未来扩展。
- 忽略列表匹配的是目录派生出的 `skill_id`，不是 `skill.yaml` 的展示名称。
- TypeScript 示例仍未在本机完成 `tsc` 编译验证；本轮只保证结构体定义与头文件字段顺序同步。
- 本轮验证已通过 `cargo fmt`、`gofmt`、Python 示例语法检查与 `cargo test -q`。

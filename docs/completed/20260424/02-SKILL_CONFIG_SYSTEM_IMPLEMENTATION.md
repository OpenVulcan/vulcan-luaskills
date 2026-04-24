## 任务目标

基于既有《Skill 配置系统设计稿》，在 `vulcan-luaskills` 中正式实现统一 Skill 配置系统，完成以下能力：

1. 由 `luaskills` 自己统一维护单一主配置文件。
2. 宿主可显式指定配置文件路径；未指定时使用默认路径。
3. Lua 侧提供 `vulcan.config.*` 读写接口。
4. Rust / FFI 宿主侧提供跨 skill 的配置管理接口，便于外部封装成单一 `runtime-config` 工具。
5. 配置值第一版统一为字符串，不引入复杂 schema 或自动启停逻辑。

## 执行步骤

1. 梳理当前运行时、宿主选项、FFI 结构与对外暴露面的代码落点。
2. 新增统一配置存储模块，完成路径解析、配置文件读写、原子写回与错误处理。
3. 在宿主选项中增加 `skill_config_file_path`，并同步标准 C ABI / 头文件解析。
4. 在 `LuaEngine` 中接入配置存储，提供宿主侧 `list/get/set/delete` 方法。
5. 在 Lua 运行时中新增 `vulcan.config.get/set/delete/has/list`。
6. 在标准 FFI 与公共 `_json` FFI 中增加配置管理接口。
7. 补充单元测试、运行时测试与文档说明。
8. 对照设计稿逐项自检，补写执行变更总结并归档计划文件。

## 技术选型

1. 配置物理存储采用单文件 JSON。
2. 默认路径采用 `<runtime_root>/config/skill_config.json`。
3. 文件内容按 `skill_id` 分组，键值统一为字符串。
4. 写入采用“临时文件 + rename 覆盖”的原子替换策略。
5. Lua 仅访问当前 skill 命名空间；宿主接口支持跨 skill 管理。

## 验收标准

1. `LuaRuntimeHostOptions` 支持显式配置 `skill_config_file_path`。
2. 未显式传入路径时，运行时可正确推导并使用默认配置文件路径。
3. Lua 中可通过 `vulcan.config.*` 完成当前 skill 的配置读写与枚举。
4. Rust / FFI 宿主侧可对任意 `skill_id` 执行 `list/get/set/delete`。
5. 配置文件不存在时按空配置处理；写入后可持久化读取。
6. 配置值统一按字符串处理。
7. 缺配置不会自动禁用 skill，由 skill 自己决定如何提示。
8. 新增测试通过，且不破坏现有主链路能力。

## 执行变更总结

### 1. 核心修复与调整概述

本次实现为 `vulcan-luaskills` 增加了统一 Skill 配置系统，形成了“单一主配置文件 + 按 `skill_id` 分组 + Lua 与宿主双侧访问协议”的完整闭环。运行时现在支持显式配置 `skill_config_file_path`，未配置时会自动回落到 `<runtime_root>/config/skill_config.json`。Lua 侧新增 `vulcan.config.get/set/delete/has/list`，宿主侧新增 Rust API、标准 C ABI 与公共 `_json` FFI` 的跨 skill 配置管理接口，可直接包装成单一 `runtime-config` 工具。

### 2. 📂文件变更清单

新增：

- `src/runtime/config.rs`
- `docs/SKILL_CONFIG_SYSTEM_DESIGN.md`

修改：

- `src/runtime/mod.rs`
- `src/lib.rs`
- `src/host/options.rs`
- `src/runtime/engine.rs`
- `src/ffi_standard.rs`
- `src/ffi.rs`
- `include/vulcan_luaskills_ffi.h`
- `include/vulcan_luaskills_json_ffi.h`
- `README.md`
- `docs/FFI_INTEGRATION_GUIDE.md`
- `docs/SKILL_DEVELOPER_MANUAL.md`
- `examples/ffi/c/demo.c`
- `examples/ffi/python/demo.py`
- `examples/ffi/demo_runtime/run_python_install_demo.py`
- `examples/ffi/host_provider_demo/run_python_host_provider_demo.py`
- `examples/ffi/go/demo.go`
- `examples/ffi/go/lifecycle_demo/main.go`
- `examples/ffi/go/query_demo/main.go`
- `examples/ffi/typescript/demo.ts`
- `examples/ffi/typescript/lifecycle_demo.ts`
- `examples/ffi/typescript/query_demo.ts`

删除：

- 无

### 3. 💻关键代码调整详情

1. 新增 `SkillConfigStore`：
   - 统一负责主配置文件路径解析、缺文件空配置处理、键校验、按 `skill_id` 分组读写、原子写回与跨 skill 枚举。
2. 宿主配置与运行时接入：
   - `LuaRuntimeHostOptions` 新增 `skill_config_file_path`。
   - `LuaEngine` 新增 `skill_config_store`，并在 `load_from_roots` 阶段刷新默认 `runtime_root`。
3. Lua 配置协议落地：
   - 向 Lua 注入 `vulcan.config.get/set/delete/has/list`。
   - Lua 侧默认只访问当前 skill 命名空间，不支持跨 skill 直接读写。
4. 宿主管理接口落地：
   - Rust API 新增 `list/get/set/delete`。
   - 标准 C ABI 新增 `vulcan_luaskills_ffi_skill_config_*`。
   - 公共 `_json` FFI 新增 `vulcan_luaskills_ffi_skill_config_*_json`。
5. 测试与示例同步：
   - 新增配置存储单元测试、Lua/Engine 运行时测试、标准 FFI / JSON FFI 往返测试。
   - C / Python / Go / TypeScript 示例同步补入 `skill_config_file_path` 字段。

### 4. ⚠️遗留问题与注意事项

1. 当前配置值第一版统一为 `string`；如需复杂结构，建议由 skill 自己存 JSON 字符串并再自行解码。
2. 当前不做“未配置即不加载”的自动逻辑；更推荐由 skill 在缺配置时返回明确提示，引导用户通过宿主配置工具补齐。
3. TypeScript 示例虽然已同步字段，但本轮只完成了依赖交付与源码对齐，未在当前环境内执行 `tsc`/真实运行验证。
4. 宿主若需要对外提供 AI 可调用的配置能力，推荐统一包装成单一 `runtime-config(action, skill_id?, key?, value?)` 工具，而不是暴露多个离散接口。

# LuaSkills 0.4.6 升级说明

本文面向从 `0.4.4` 升级到 `0.4.6` 的宿主、SDK 与调试工具维护者。

`0.4.6` 的核心目标是收敛运行时目录配置：宿主只需要传入一个独立的 `runtime_root`，LuaSkills 负责从该根目录推导所有固定子目录。这样可以避免宿主程序目录、Lua 运行时目录、skill 目录、原生库目录互相混用。

## 一、版本升级范围

本次需要同步升级以下组件：

- 主仓库 crate：`luaskills = 0.4.6`
- 主仓库 Rust demo：`examples/demo-rust = 0.4.6`
- TypeScript SDK：`@luaskills/sdk = 0.4.6`
- Python SDK：`luaskills-sdk = 0.4.6`
- Go SDK module tag：`v0.4.6`
- runtime asset 默认 tag：`v0.4.6`

升级时建议保持主仓库、SDK、demo 和运行时资产版本一致，避免新 SDK 向旧 FFI 传入旧版本无法理解的 host options。

## 二、运行时目录新规则

新宿主集成只传入：

```text
runtime_root = <独立 LuaSkills 运行时根目录>
```

LuaSkills 会固定推导：

```text
runtime_root/bin
runtime_root/libs
runtime_root/lua_packages
runtime_root/resources
runtime_root/skills
runtime_root/temp
runtime_root/temp/downloads
runtime_root/dependencies
runtime_root/state
runtime_root/databases
runtime_root/config
runtime_root/config/skill_config.json
runtime_root/system_lua_lib
```

关键变化：

- 宿主程序目录不再等同于 LuaSkills runtime 目录。
- 宿主工具直接放在 `runtime_root/bin`，不再放到 `runtime_root/bin/tools`。
- Lua C module、FFI 库和上级 DLL 依赖统一放在 `runtime_root/libs`。
- Lua 包探测根固定为 `runtime_root/lua_packages`。
- 默认统一 skill 配置文件固定为 `runtime_root/config/skill_config.json`。
- 默认 system Lua 库目录固定为 `runtime_root/system_lua_lib`。

推荐目录形态：

```text
app_root/
  bin/
    vulcan-code.exe

lua_runtime/
  bin/
  libs/
  lua_packages/
  resources/
  skills/
  temp/
  dependencies/
  state/
  databases/
  config/
  system_lua_lib/
```

## 三、Host Options 迁移

### JSON FFI 与 SDK

JSON FFI 和 SDK 默认只传 `runtime_root`：

```json
{
  "host_options": {
    "runtime_root": "D:/path/to/lua_runtime"
  }
}
```

旧字段仍作为兼容字段存在，但新集成不应再主动传入这些派生目录：

- `temp_dir`
- `resources_dir`
- `lua_packages_dir`
- `host_provided_tool_root`
- `host_provided_lua_root`
- `host_provided_ffi_root`
- `system_lua_lib_dir`
- `download_cache_root`
- `dependency_dir_name`
- `state_dir_name`
- `database_dir_name`
- `skill_config_file_path`

当 `runtime_root` 存在时，LuaSkills 会按固定布局重写这些派生字段。

### Rust 直连宿主

Rust 直连宿主应使用：

```rust
LuaRuntimeHostOptions::with_runtime_root(runtime_root)
```

或确保传入 `LuaRuntimeHostOptions` 后调用到引擎前完成规范化。`LuaEngine::new` 会对 host options 做统一 normalization。

### Standard C ABI

为了保持旧 `FfiLuaRuntimeHostOptions` 的 v1 结构体布局兼容，`runtime_root` 不直接加到 v1 结构体里。

旧宿主继续使用：

```c
luaskills_ffi_engine_new(const FfiLuaEngineOptions *options, ...)
```

新宿主如果要通过 standard C ABI 传 `runtime_root`，使用 v2：

```c
FfiLuaRuntimeHostOptionsV2 host = {
    .base = old_host_options,
    .runtime_root = "D:/path/to/lua_runtime",
};

FfiLuaEngineOptionsV2 options = {
    .pool = pool_options,
    .host = host,
};

luaskills_ffi_engine_new_v2(&options, &engine_id, &error_out);
```

这条规则可以避免旧二进制因为结构体尺寸或字段顺序变化而错误读取内存。

## 四、SDK 调整

三个 SDK 已统一到同一规则：

- 默认 host options 只传 `runtime_root`。
- `host_provided_tool_root`、`host_provided_lua_root`、`host_provided_ffi_root` 等派生目录默认不再由 SDK 展开。
- SDK 创建运行时布局时会创建 `bin`、`libs`、`lua_packages`、`resources`、`skills`、`temp`、`config`、`system_lua_lib` 等固定目录。
- 文档中 `bin/tools` 已迁移为 `bin`。

升级 SDK 后，如果宿主仍手动覆盖派生目录，需要重新检查这些覆盖是否还必要。新模式下建议删除这些覆盖，只保留 `runtime_root` 和真正的策略项，例如 provider mode、callback mode、网络下载策略、controller 配置等。

## 五、调试工具变化

`luaskills-debug` 现在也遵循真实 runtime root 布局：

- 调试时先把目标 skill 同步到 `runtime_root/skills/<skill_id>`。
- 加载时复用正式 `load_from_roots -> call_skill` 链路。
- 工具目录使用 `runtime_root/bin`。
- 原生库目录使用 `runtime_root/libs`。
- system Lua 库目录使用 `runtime_root/system_lua_lib`。

并发调试同一个 skill 和同一个 runtime root 仍不建议直接并发执行。若需要并发调试，应使用不同 `runtime_root`，或先做同步再串行运行调试调用。

## 六、升级检查清单

升级宿主时逐项确认：

- 已将 LuaSkills 运行时目录从宿主程序目录中剥离出来。
- 已准备独立 `runtime_root`。
- SDK、demo、主仓库和 runtime asset tag 已统一到 `0.4.6` / `v0.4.6`。
- JSON FFI / SDK host options 只传 `runtime_root` 和策略字段。
- Standard C ABI 新宿主使用 `luaskills_ffi_engine_new_v2`。
- `runtime_root/bin/tools` 已迁移为 `runtime_root/bin`。
- 原生库和 Lua C module 依赖已放到 `runtime_root/libs`。
- Lua 包已放到 `runtime_root/lua_packages`。
- skill 配置默认路径已接受为 `runtime_root/config/skill_config.json`。
- 若宿主确实需要多个 runtime root，共享配置路径必须显式设计，不要依赖隐式猜测。

## 七、常见错误

### 找不到 `lcurl.safe`

优先检查：

- `lcurl.dll` 或对应平台动态库是否在 `runtime_root/libs`。
- Lua C module 是否在 `runtime_root/libs` 或 `runtime_root/lua_packages` 下能被当前 package path / cpath 探测到。
- 宿主是否仍把依赖放在旧的程序目录或 `bin/tools` 下。
- SDK 和 FFI 动态库是否同为 `0.4.6`。

### runtime manifest 缺少文件

如果出现类似缺少 `THIRD_PARTY_LICENSES.json` 的错误，说明当前 runtime assets 不完整或版本不匹配。应重新安装对应 `v0.4.6` 的 runtime assets，并确认 `runtime_root/resources/luaskills-packages/` 下包含完整 manifest、license、help 和 package 清单。

### 并发调试删除同步目录失败

同一个 skill 和同一个 runtime root 不应被多个 debug 进程同时同步。使用独立 runtime root，或拆分为先同步、再串行运行。

## 八、兼容性边界

- JSON FFI 和三个 SDK 是本次推荐接入路径。
- Standard C ABI 的旧 `luaskills_ffi_engine_new` 保持 v1 host options 布局。
- Standard C ABI 新增 `luaskills_ffi_engine_new_v2` 承载 `runtime_root`。
- 旧派生目录字段仍可用于兼容旧宿主，但不再作为新宿主集成推荐入口。


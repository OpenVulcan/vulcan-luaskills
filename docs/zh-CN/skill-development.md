# Lua Skill 开发手册

## 1. 文档目标

本文档面向 **Lua Skill 作者**，整理当前仓库里已经实际暴露给 Lua 的 `vulcan.*` 能力面，并说明：

- 当前有哪些 `vulcan.*` 能力可用
- 每项能力怎么调用
- 哪些能力默认可用，哪些依赖宿主注入
- 哪些字段属于内部实现细节，不建议 skill 直接依赖

本文档以当前代码实现为准，主要对应 `src/runtime/engine.rs` 中的 Lua 注入逻辑。

## 2. 快速结论

当前 Skill 运行时会暴露这些顶级能力：

- `vulcan.call`
- `vulcan.runtime.*`
- `vulcan.fs.*`
- `vulcan.path.*`
- `vulcan.process.*`
- `vulcan.os.*`
- `vulcan.json.*`
- `vulcan.cache.*`
- `vulcan.host.*`
- `vulcan.context.*`
- `vulcan.deps.*`
- `vulcan.sqlite.*`
- `vulcan.lancedb.*`

同时还有两个重要事实：

1. 当前运行时默认把 skill 当作**受信代码**执行，不提供沙箱安全承诺。
2. `vulcan.context.*`、`vulcan.deps.*`、`vulcan.sqlite.*`、`vulcan.lancedb.*` 中有一部分内容依赖宿主或当前 skill 绑定状态，不能假设始终存在有效值。
3. 宿主可以通过 `LuaRuntimeHostOptions.ignored_skill_ids` 强制忽略某些 skill，被忽略的 skill 不会进入依赖准备、数据库绑定或 entry 注册阶段。

## 2.1 Skill 命名与发布包规则

`skill_id` 是 LuaSkills 运行时、生命周期管理、配置命名空间、依赖目录、数据库绑定和 canonical entry 名称共同使用的稳定主键。当前规则是“目录名即 `skill_id`”，而不是由 `skill.yaml` 里的字段声明。

`skill_id` 和每个 `entry.name` 必须使用同一套标识符格式：

```text
^[a-z]([a-z0-9-]*[a-z0-9])?$
```

规则含义：

- 必须以小写 ASCII 字母开头。
- 后续只能包含小写 ASCII 字母、数字和连字符 `-`。
- 不能以下划线、数字、大写字母或连字符结尾。
- 合法示例：`vulcan-codekit`、`codekit2`、`vulcan-runtime-tools`。
- 非法示例：`2codekit`、`Vulcan-codekit`、`vulcan_codekit`、`vulcan-codekit-`。

Skill 包结构必须遵守：

- 物理目录名就是最终 `skill_id`，例如 `skills/vulcan-codekit/` 的 `skill_id` 是 `vulcan-codekit`。
- `skill.yaml` 不允许声明 `skill_id` 字段；如果出现该字段，运行时会拒绝加载。
- `skill.yaml` 的 `name` 是人类可读元数据，不参与 `skill_id` 匹配。
- 每个 `entries[].name` 是当前 skill 内部的局部入口名，也必须满足同一标识符规则。
- 运行时对外暴露的 canonical entry 名称为 `{skill_id}-{entry_name}`；若与宿主保留名称或其他入口冲突，会追加稳定数字后缀，形成 `{skill_id}-{entry_name}-{N}`。

GitHub 托管 skill 的安装与发布资产必须保持同一个 `skill_id`：

- 安装请求若未显式传入 `skill_id`，运行时会从 GitHub 仓库名派生 `skill_id`，不会自动剥离 `luaskills-` 前缀。
- release zip 文件名必须是 `{skill_id}-v{version}-skill.zip`。
- checksum 文件名必须是 `{skill_id}-v{version}-checksums.txt`。
- zip 内部只能包含与 `skill_id` 同名的顶层目录，并且必须包含 `{skill_id}/skill.yaml`。
- 仓库名、release 资产名前缀、checksum 文件名前缀、zip 顶层目录和最终安装目录应全部使用同一个 `skill_id`。

## 3. 顶级能力总览

| 顶级项 | 作用 | 默认可用 | 备注 |
| --- | --- | --- | --- |
| `vulcan.call` | 调用其他 skill 入口 | 是 | 要求第二个参数必须是 Lua table |
| `vulcan.runtime` | 运行时辅助能力 | 是 | 包含日志、cwd、luaexec、skill 管理桥接等 |
| `vulcan.fs` | 文件系统读写 | 是 | 不做沙箱限制 |
| `vulcan.path` | 路径拼接 | 是 | 返回对 Lua 友好的系统路径 |
| `vulcan.process` | 启动子进程 | 是 | 返回结构化结果 |
| `vulcan.os` | 宿主 OS/架构信息 | 是 | `os`、`arch` |
| `vulcan.json` | JSON 编解码 | 是 | JSON ↔ Lua table |
| `vulcan.cache` | 运行时缓存 | 是 | 在 `vulcan.runtime.lua.exec` 中会被禁用 |
| `vulcan.models` | 标准模型能力 | 是 | 只有宿主注册对应 callback 后能力才会开启 |
| `vulcan.host` | 宿主注册工具桥接 | 是 | 宿主未注册 callback 时为空能力面 |
| `vulcan.context` | 请求与当前入口上下文 | 是 | 多数值由宿主注入 |
| `vulcan.deps` | 当前 skill 依赖根路径 | 是 | 未解析到当前 skill 时可能为 `nil` |
| `vulcan.sqlite` | 当前 skill 的 SQLite 绑定 | 条件可用 | 未启用时仍有 `enabled/status/info` |
| `vulcan.lancedb` | 当前 skill 的 LanceDB 绑定 | 条件可用 | 未启用时仍有 `enabled/status/info` |

## 3.1 宿主强制忽略 skill

`ignored_skill_ids` 是宿主运行时级策略，用来在加载早期跳过某些 skill。

典型场景：

- 宿主已经提供了更强的原生、gRPC 或 VMM 能力实现
- 默认 skill 包与宿主现有功能重复
- 某个数据库型 skill 的能力已被宿主侧服务替代，不希望继续启动 SQLite / LanceDB 绑定

匹配规则：

- 匹配对象是 skill 目录派生出的 `skill_id`
- 不匹配 `skill.yaml` 的 `name`
- 不由 skill 自己声明，也不属于依赖判定

运行时效果：

- 命中忽略列表后，整个 skill 会被跳过
- 不准备 `dependencies.yaml`
- 不绑定 SQLite / LanceDB
- 不注册任何 entry
- 不会出现在 `list_entries` 或 `vulcan.call` 可调用目标中

这项能力保留宿主和用户的最终选择权。  
如果用户希望继续使用某个 skill，宿主不应在策略层把它加入忽略列表。

## 4. `vulcan.call`

### 4.1 用途

`vulcan.call(name, args)` 用来在一个 skill 内部调用另一个已加载的 skill 入口。

- `name`：目标入口的 canonical 名称
- `args`：必须是 Lua table
- 返回值：直接透传被调 skill 的返回值，支持多返回值

### 4.2 最小示例

```lua
local ok, result = pcall(vulcan.call, "demo-skill-run", {
    query = "hello",
    limit = 5,
})

if not ok then
    vulcan.runtime.log("warn", "call failed: " .. tostring(result))
    return nil
end

return result
```

### 4.3 注意事项

- 不存在的入口会直接报错。
- `args` 必须是 table，不能传字符串或其他标量。
- `vulcan.call` 会继承当前请求上下文、预算快照、tool config，并切换到目标 skill 的文件上下文与数据库绑定。
- 在 `luaexec` 场景下，存在额外重入保护，不能无限递归调回当前运行时调用方。

## 4.5 `vulcan.models.*`

`vulcan.models.*` 是 Lua skill 使用模型能力的固定标准接口。
它不是通用 host tool 调用，也不允许 Lua 选择 provider 配置。

支持的方法：

- `vulcan.models.status()`：返回 `{ ok = true, capabilities = { embed = boolean, llm = boolean } }`。
- `vulcan.models.has(capability)`：返回宿主是否注册了 `embed` 或 `llm` callback。
- `vulcan.models.embed(text)`：对单个非空字符串执行 embedding，并返回 table 包络。
- `vulcan.models.llm(system, user)`：执行一轮非流式 LLM 调用，并返回 table 包络。

最小示例：

```lua
if not vulcan.models.has("embed") then
    return {
        ok = false,
        reason = "model-embed-unavailable",
    }
end

local result = vulcan.models.embed("hello")
if not result.ok then
    return result
end

return result.vector
```

embedding 成功包络：

```lua
{
    ok = true,
    vector = { 0.1, 0.2, 0.3 },
    dimensions = 1536,
    usage = {
        input_tokens = 123,
    },
}
```

LLM 成功包络：

```lua
{
    ok = true,
    assistant = "...",
    usage = {
        input_tokens = 123,
        output_tokens = 456,
    },
}
```

错误包络：

```lua
{
    ok = false,
    error = {
        code = "provider_error",
        message = "model provider failed",
        provider_message = "raw provider error after host redaction",
        provider_code = "model_not_found",
        provider_status = 400,
    },
}
```

行为规则：

- `status()` 永远存在，并根据 callback 注册状态生成能力表。
- `has()` 只识别 `embed` 与 `llm`；未知能力返回 `false`。
- `embed()` 只接受一个非空字符串，不支持批量输入。
- `llm()` 只接受两个非空字符串，不支持 messages、tool call、stream 或 thinking 控制。
- Lua 不能传 `model`、`temperature`、`max_tokens`、`base_url`、`api_key`、`dimensions` 或 provider-specific 参数。
- LuaSkills 会把 caller context 传给宿主 callback，用于审计与成本归因，但不会通过模型 API 暴露给 Lua。
- 模型配置、API key、provider 路由、超时、预算和脱敏都由宿主负责。

宿主对接参考：

- [运行时架构中的模型能力边界](../architecture/runtime-model.md#standard-model-capability-boundary)
- [FFI 与 SDK 模型能力速查](../ffi/overview.md#model-capability-quick-path)
- [中文 FFI 模型 callback 对接说明](ffi/integration-guide.md#98-模型能力-callback)

## 4.6 `vulcan.host.*`

`vulcan.host.*` 是固定的宿主注册工具桥接。
它刻意比任意 `vulcan.xxx` 注入更窄：Lua 可以列出、探测和调用宿主工具，但不能自己创建新的顶级命名空间，也不能注册宿主工具。

支持的方法：

- `vulcan.host.list()`：返回当前宿主开放给 Lua 的工具元数据 table。
- `vulcan.host.has(tool_name)`：判断指定宿主工具是否存在。
- `vulcan.host.has_tool(tool_name)`：`has` 的别名。
- `vulcan.host.call(tool_name, args)`：使用 Lua table 参数调用指定宿主工具，并返回 Lua table 结果。

最小示例：

```lua
if not vulcan.host.has("vault.lookup") then
    return {
        ok = false,
        reason = "host-tool-unavailable",
    }
end

local result = vulcan.host.call("vault.lookup", {
    key = "demo-secret",
})

if not result.ok then
    return result
end

return result.value
```

推荐的宿主工具成功返回包络：

```lua
{
    ok = true,
    value = {
        text = "resolved value",
    },
    meta = {
        elapsed_ms = 120,
    },
}
```

推荐的错误返回包络：

```lua
{
    ok = false,
    error = {
        code = "tool_not_found",
        message = "host tool not found: vault.lookup",
    },
}
```

行为规则：

- 宿主未注册 host-tool callback 时，`list()` 返回空 table。
- 宿主未注册 host-tool callback 时，`has()` 和 `has_tool()` 返回 `false`。
- 宿主 callback 缺失或调用返回错误时，`call()` 返回错误包络。
- `args` 必须是 Lua table；对象型入参建议使用显式 key。
- 该桥接不支持 stream，宿主工具应返回完整 table 结果。
- 权限、超时、审计和 secret 管理仍由宿主负责。
- 标准模型能力应使用 `vulcan.models.*`，不要长期依赖通用 host-tool 协议。

## 5. `vulcan.runtime.*`

### 5.1 `vulcan.runtime.log(level, message)`

用于向宿主日志输出一条运行时日志。

```lua
vulcan.runtime.log("info", "skill started")
vulcan.runtime.log("warn", "budget is low")
vulcan.runtime.log("error", "query failed")
```

说明：

- `level` 会按文本内容粗分为 `error/fatal`、`warn`、其他。
- 该能力在普通 skill VM 中可用。
- 在 `vulcan.runtime.lua.exec(...)` 的隔离执行环境中，该函数会被禁用。

### 5.2 `vulcan.runtime.cwd()`

返回当前进程工作目录。

```lua
local cwd = vulcan.runtime.cwd()
```

### 5.3 `vulcan.runtime.temp_dir`

宿主注入的临时目录路径，可能为 `nil`。

```lua
local temp_dir = vulcan.runtime.temp_dir
```

### 5.4 `vulcan.runtime.resources_dir`

宿主注入的资源目录路径，可能为 `nil`。

```lua
local resources_dir = vulcan.runtime.resources_dir
```

### 5.5 `vulcan.runtime.overflow_type`

当前暴露两个固定常量：

- `vulcan.runtime.overflow_type.truncate`
- `vulcan.runtime.overflow_type.page`

它们主要供宿主侧预算/溢出策略相关逻辑使用。

### 5.6 `vulcan.runtime.internal`

当前会暴露这些字段：

- `tool_name`
- `skill_name`
- `luaexec_active`
- `luaexec_caller_tool_name`

这组字段属于**内部执行上下文**，建议只用于调试和定位问题，不建议作为长期公共协议依赖。

### 5.7 `vulcan.runtime.lua.exec(input)`

执行一次隔离的内联 Lua 运行时调用，返回 **Markdown 字符串**，不是普通 Lua table。

当前输入结构支持：

- `task`：人类可读任务摘要，可选
- `code`：内联 Lua 代码，可选
- `file`：要执行的 Lua 文件路径，可选
- `args`：传给代码的结构化参数对象，可选，默认空对象
- `timeout_ms`：超时时间（毫秒），可选，默认 `60000`

最小示例：

```lua
local rendered = vulcan.runtime.lua.exec({
    task = "inspect args",
    code = [[
        print("hello", args.name)
        return { ok = true, name = args.name }
    ]],
    args = {
        name = "codex",
    },
})

return rendered
```

重要限制：

- 返回值是 Markdown 渲染结果，不是结构化 Lua table。
- 隔离环境里会覆盖全局 `print`，把输出收集进结果文本。
- 隔离环境里会禁用：
  - `vulcan.runtime.log`
  - `vulcan.cache.put`
  - `vulcan.cache.get`
  - `vulcan.cache.delete`
  - `vulcan.runtime.lua.exec`（禁止递归再调）
- 该环境使用内部模拟请求上下文，因此你在 `luaexec` 中看到的：
  - `vulcan.context.client_info.name`
  - `vulcan.context.request.transport_name`
  默认会是 `luaexec_call` 一类内部标识，而不是外部真实客户端。
- 当前隔离执行链已经拥有独立 VM 池：
  - 默认 `min_size=1 / max_size=4 / idle_ttl_secs=60`
  - 可由宿主通过 `LuaRuntimeHostOptions.runlua_pool_config` 覆盖
  - 这只影响 `vulcan.runtime.lua.exec(...)`，不改变普通 skill VM 池
  - 当前不再支持为 `vulcan.runtime.lua.exec(...)` 单独配置外部执行器路径

### 5.8 `vulcan.runtime.skills.*`

这组能力用于让 skill 请求宿主执行安装、更新、启停、卸载等动作。

正式层级模型为：

```text
ROOT -> PROJECT -> USER
```

其中 `ROOT` 是系统控制级，运行时启动或加载时必须存在该层。普通 skill 不能通过 `vulcan.runtime.skills.*` 请求安装、更新、卸载、启用或停用 `ROOT` 级 skill。普通桥接只面向宿主开放且当前实际存在的 `PROJECT` / `USER` 层级。

`vulcan.runtime.skills.*` 固定等价于 `DelegatedTool` 权限：它看不到 `ROOT` skills，也不能写入 `ROOT`。FFI 查询与 prompt completion 入口在 `DelegatedTool` 下同样不会返回 `ROOT` entries、help 或 ROOT tool name 归属。`call_skill` 与 `run_lua` 是运行时执行面，允许调用当前已激活的 skill，不作为技能管理权限边界。如果 `ROOT` 已经存在同名 `skill_id`，普通层 install / update 会被拒绝；普通层 uninstall 仍可用于清理 `PROJECT` / `USER` 中的同名残留。

正式桥接建议包含：

- `vulcan.runtime.skills.enabled`
- `vulcan.runtime.skills.status()`
- `vulcan.runtime.skills.layers()`
- `vulcan.runtime.skills.install(input)`
- `vulcan.runtime.skills.update(input)`
- `vulcan.runtime.skills.uninstall(input)`
- `vulcan.runtime.skills.enable(input)`
- `vulcan.runtime.skills.disable(input)`

`status()` 当前固定返回：

- `enabled`
- `callback_registered`
- `mode`
- `message`

`layers()` 用于返回当前宿主允许普通桥接操作的层级标签。推荐返回内容包含：

- `default`
- `writable`
- `labels`
- `layers`

其中 `labels` 只应包含 `PROJECT` / `USER` 中当前实际存在的层级，不应包含 `ROOT`。如果当前没有项目上下文，运行时只返回 `USER`；如果只有 `ROOT`，则返回空列表且顶层 `writable=false`。bridge 关闭时仍可发现层级，但顶层 `writable` 与每个 layer 的 `writable` 都必须为 `false`。如果后续安装、更新、卸载输入允许指定层级，也只能使用 `layers()` 返回的标签。

完整层级与管理边界见 [Skill Root 层级与管理边界](architecture/skill-root-layer-policy.md)。

注意事项：

- 这组能力只有在宿主显式打开 `enable_skill_management_bridge` 时才允许执行。
- 即使宿主策略打开了，如果没有注册对应回调，也会返回明确错误。
- `input` / 返回值结构是宿主桥接契约的一部分，建议由宿主侧文档或测试夹具统一约束。

### 5.9 `vulcan.config.*`

这组能力用于读取和维护**当前 skill 自己的字符串配置**。

当前提供：

- `vulcan.config.get(key)`
- `vulcan.config.set(key, value)`
- `vulcan.config.delete(key)`
- `vulcan.config.has(key)`
- `vulcan.config.list()`

最小示例：

```lua
local api_token = vulcan.config.get("api_token")

if not api_token or api_token == "" then
    return "当前未配置 `api_token`。请使用宿主提供的 runtime-config 工具为当前 skill 设置 `api_token`。"
end

local endpoint = vulcan.config.get("endpoint") or "https://api.example.com"

vulcan.config.set("last_endpoint", endpoint)

return {
    ok = true,
    endpoint = endpoint,
}
```

`list()` 当前返回的是当前 skill 命名空间下的平面表：

```lua
local config = vulcan.config.list()
-- config.api_token
-- config.endpoint
```

注意事项：

- 当前配置值第一版统一为 `string`。
- 如果你确实需要复杂结构，建议把 JSON 文本作为字符串存入，再由 skill 自己 `vulcan.json.decode(...)`。
- 配置默认只作用于当前 skill，不能直接跨 skill 读写其他命名空间。
- 当前不做“未配置即不加载”的自动策略；更推荐 skill 在缺配置时返回明确提示，告知用户如何完成配置。
- 统一主配置文件默认位于 `<runtime_root>/config/skill_config.json`；宿主也可以显式覆盖路径。

## 6. `vulcan.fs.*`

### 6.1 支持的方法

- `vulcan.fs.list(dir)`
- `vulcan.fs.read(path)`
- `vulcan.fs.write(path, content)`
- `vulcan.fs.exists(path)`
- `vulcan.fs.is_dir(path)`

### 6.2 示例

```lua
local entries = vulcan.fs.list(vulcan.context.entry_dir)
local exists = vulcan.fs.exists(vulcan.context.entry_file)
local content = vulcan.fs.read(vulcan.context.entry_file)
```

### 6.3 注意事项

- 当前没有沙箱限制，skill 理论上可以访问宿主可访问的任意路径。
- `fs.read` / `fs.write` 处理的是文本内容。
- 路径参数必须是字符串，且会经过基础路径语法校验。

## 7. `vulcan.path.*`

当前只暴露：

- `vulcan.path.join(...)`

示例：

```lua
local config_path = vulcan.path.join(
    vulcan.context.skill_dir,
    "runtime",
    "config.json"
)
```

路径返回规则：

- 会按宿主系统返回正常路径文本。
- Windows 下不会把 `\\?\` 或 `\\?\UNC\` verbatim 前缀直接泄漏给 Lua。

## 8. `vulcan.process.*`

当前只暴露：

- `vulcan.process.exec(spec)`

### 8.1 请求结构

支持两种模式：

1. shell 模式

```lua
local result = vulcan.process.exec({
    shell = "echo hello",
    timeout_ms = 3000,
})
```

2. program 模式

```lua
local result = vulcan.process.exec({
    program = "git",
    args = { "status", "--short" },
    cwd = vulcan.runtime.cwd(),
    env = {
        DEMO_MODE = "1",
    },
    timeout_ms = 5000,
})
```

常用字段：

- `shell`
- `program`
- `args`
- `cwd`
- `env`
- `stdin`
- `timeout_ms`

### 8.2 返回结构

返回 table 固定包含：

- `ok`
- `success`
- `code`
- `stdout`
- `stderr`
- `timed_out`
- `error`

## 9. `vulcan.os.*`

当前提供：

- `vulcan.os.info()`

示例：

```lua
local info = vulcan.os.info()
-- info.os
-- info.arch
```

## 10. `vulcan.json.*`

当前提供：

- `vulcan.json.encode(value)`
- `vulcan.json.decode(text)`

示例：

```lua
local text = vulcan.json.encode({
    hello = "world",
    limit = 3,
})

local obj = vulcan.json.decode(text)
```

说明：

- Lua table 会被转换成 JSON 对象或数组。
- 解码后的 JSON 对象/数组会转换回 Lua table。

## 11. `vulcan.cache.*`

当前提供：

- `vulcan.cache.put(value, ttl_sec?)`
- `vulcan.cache.get(cache_id)`
- `vulcan.cache.delete(cache_id)`

示例：

```lua
local cache_id = vulcan.cache.put({
    summary = "warm result",
}, 60)

local cached = vulcan.cache.get(cache_id)
local deleted = vulcan.cache.delete(cache_id)
```

注意事项：

- 缓存作用域会优先落到当前 `tool_name`，否则落到当前 `skill_name`。
- 如果都取不到，会退化到内部 `__runtime` 作用域。
- 在 `vulcan.runtime.lua.exec(...)` 中，缓存接口会被主动清空，不可用。

## 12. `vulcan.context.*`

`vulcan.context` 用来读取当前请求和当前入口的运行时上下文。

当前字段包括：

- `request`
- `client_info`
- `client_capabilities`
- `client_budget`
- `tool_config`
- `skill_dir`
- `entry_dir`
- `entry_file`

### 12.1 `vulcan.context.request`

宿主传入的原始请求上下文对象，默认是空对象。

常见字段来自：

- `transport_name`
- `session_id`
- `request_id`
- `client_name`
- `client_info`
- `client_capabilities`

### 12.2 `vulcan.context.client_info`

当前请求的客户端元信息，常见字段：

- `kind`
- `name`
- `version`

说明：

- 如果宿主没有注入 `client_info`，这里可能是 `nil`。
- 如果你在 `luaexec` 中看到 `name = "luaexec_call"`，那是内部隔离执行环境的模拟上下文，不是外部真实客户端。

### 12.3 `vulcan.context.client_capabilities`

宿主传入的客户端能力对象，默认是空对象。

### 12.4 `vulcan.context.client_budget`

宿主解析后的预算快照对象，默认是空对象。

该对象由宿主决定内容，但常见会包含：

- `client_name`
- `tool_name`
- `skill_name`
- `tool_result`
- `file_read`

### 12.5 `vulcan.context.tool_config`

宿主解析后的工具配置对象，默认是空对象。

### 12.6 `vulcan.context.skill_dir / entry_dir / entry_file`

当前执行 skill 的文件上下文：

- `skill_dir`：当前 skill 目录
- `entry_dir`：当前入口脚本所在目录
- `entry_file`：当前入口脚本完整路径

说明：

- 在普通 skill 调用中，这三个值通常都可用。
- 在某些 runlua / help / 非 skill 文件场景里，可能为 `nil`。
- 当前实现会自动把 Windows verbatim 路径前缀去掉，保证 Lua 侧拿到的是正常系统路径。

## 13. `vulcan.deps.*`

当前字段包括：

- `vulcan.deps.tools_path`
- `vulcan.deps.lua_path`
- `vulcan.deps.ffi_path`

这三项表示当前 skill 的依赖根路径：

- 工具依赖目录
- Lua 依赖目录
- FFI 依赖目录

示例：

```lua
local lua_lib_root = vulcan.deps.lua_path
local ffi_root = vulcan.deps.ffi_path
```

注意事项：

- 这三项依赖当前 skill 所在根目录和宿主依赖布局。
- 如果当前没有有效 skill 上下文，它们会是 `nil`。
- skill 应当只依赖这些协议暴露出的路径，不要自己猜宿主物理目录结构。

## 14. `vulcan.sqlite.*`

`vulcan.sqlite` 是**按当前 skill 作用域隔离**的 SQLite 绑定。

### 14.1 当前字段与方法

- `vulcan.sqlite.enabled`
- `vulcan.sqlite.info()`
- `vulcan.sqlite.status()`
- `vulcan.sqlite.tokenize_text(input)`
- `vulcan.sqlite.execute_script(input)`
- `vulcan.sqlite.execute_batch(input)`
- `vulcan.sqlite.query_json(input)`
- `vulcan.sqlite.query_stream(input)`
- `vulcan.sqlite.query_stream_wait_metrics(input)`
- `vulcan.sqlite.query_stream_chunk(input)`
- `vulcan.sqlite.query_stream_close(input)`
- `vulcan.sqlite.upsert_custom_word(input)`
- `vulcan.sqlite.remove_custom_word(input)`
- `vulcan.sqlite.list_custom_words()`
- `vulcan.sqlite.ensure_fts_index(input)`
- `vulcan.sqlite.rebuild_fts_index(input)`
- `vulcan.sqlite.upsert_fts_document(input)`
- `vulcan.sqlite.delete_fts_document(input)`
- `vulcan.sqlite.search_fts(input)`

### 14.2 行为规则

- `enabled = true` 表示当前 skill 已绑定 SQLite 能力。
- `info()` / `status()` 总是存在。
- 当 SQLite 未启用时：
  - `enabled = false`
  - `info()` / `status()` 会返回禁用状态描述
  - 其余方法会直接报错：`current skill has not enabled sqlite`

### 14.3 开发建议

- 把 `info()` / `status()` 当成探测入口。
- 业务调用前先判断 `enabled`，避免把“能力未绑定”误当成查询失败。
- 具体输入输出字段请结合宿主的 SQLite provider 契约与：
  - [宿主数据库 Provider 对接说明](providers/host-database-provider-guide.md)

## 15. `vulcan.lancedb.*`

`vulcan.lancedb` 是**按当前 skill 作用域隔离**的 LanceDB 绑定。

### 15.1 当前字段与方法

- `vulcan.lancedb.enabled`
- `vulcan.lancedb.info()`
- `vulcan.lancedb.status()`
- `vulcan.lancedb.create_table(input)`
- `vulcan.lancedb.vector_upsert(input)`
- `vulcan.lancedb.vector_search(input)`
- `vulcan.lancedb.delete(input)`
- `vulcan.lancedb.drop_table(input)`

### 15.2 行为规则

- `enabled = true` 表示当前 skill 已绑定 LanceDB 能力。
- `info()` / `status()` 总是存在。
- 当 LanceDB 未启用时：
  - `enabled = false`
  - `info()` / `status()` 会返回禁用状态描述
  - 其余方法会直接报错：`current skill has not enabled lancedb`

### 15.3 特别说明

`vector_search(input)` 的结果里可能出现两种载荷形态：

- `data_json`
- `data`

当返回格式是 JSON 时，结果表里会放 `data_json`；否则会放原始二进制字符串 `data`。

具体输入输出字段请结合宿主的 LanceDB provider 契约与：

- [宿主数据库 Provider 对接说明](providers/host-database-provider-guide.md)

## 16. 常见开发建议

### 16.1 先探测，再调用

对宿主条件注入的能力，建议先探测：

```lua
if vulcan.sqlite.enabled then
    return vulcan.sqlite.query_json({
        sql = "select 1 as ok",
    })
end

return {
    ok = false,
    reason = "sqlite-disabled",
}
```

### 16.2 不要依赖内部字段名

这些属于内部机制，不建议 skill 长期依赖：

- `vulcan.runtime.internal.*`
- `vulcan.__sqlite_skill_name`
- `vulcan.__lancedb_skill_name`

### 16.3 不要猜测宿主目录布局

优先使用：

- `vulcan.context.skill_dir`
- `vulcan.context.entry_dir`
- `vulcan.context.entry_file`
- `vulcan.deps.*`

不要自己反推：

- runtime 根目录
- 其他 skill 的依赖目录
- 宿主是否使用了某个固定目录名

### 16.4 区分“外部请求上下文”和“内部 luaexec 上下文”

如果你在普通 skill 执行时看到真实客户端名称，而在 `vulcan.runtime.lua.exec(...)` 里看到 `luaexec_call`，这是当前设计的正常行为，不是 Bug。

## 17. 推荐阅读顺序

如果你主要是写 skill，建议按下面顺序阅读：

1. 本文档：了解当前 `vulcan.*` 真实能力面
2. [README.zh-CN.md](../../README.zh-CN.md)：理解运行时定位与宿主边界
3. [宿主数据库 Provider 对接说明](providers/host-database-provider-guide.md)：了解 SQLite / LanceDB 与宿主的对接契约

如果后续你要做宿主集成而不是写 skill，请改看：

- [FFI 对接文档](ffi/integration-guide.md)
- [FFI 宿主接入检查清单](ffi/host-checklist.md)

# vulcan-luaskills

`vulcan-luaskills` 是 Vulcan 生态中的 **LuaSkills 核运行时库**。  
它负责 skill 加载、help 树解析、Lua VM 执行、`vulcan.*` / `vulcan.runtime.*` 注入，以及 SQLite / LanceDB 等标准能力接线。

它**不是**：

- MCP server
- 配置文件读取器
- 客户端预算计算器
- 宿主分页/截断渲染器
- 宿主产品层工具展示器

一句话说：

**`vulcan-luaskills` 负责运行 skill，宿主负责决定怎么把这些能力公开给用户。**

## 当前定位

这个库是整个 LuaSkills 体系的核心真相层，主要承担：

- LuaSkills 包加载
- entry 枚举与调用
- strict help 树解析
- `vulcan.*` 公共能力注入
- `vulcan.runtime.*` system 能力注入
- SQLite / LanceDB 标准能力绑定
- 运行时缓存能力
- 结构化运行时结果输出

当前已经支持两种产物方向：

- Rust 宿主直接 `cargo` 引用
- 后续 FFI / 动态库导出

因此本项目在 Cargo 中同时声明：

```toml
[lib]
crate-type = ["rlib", "cdylib", "staticlib"]
```

## 核心原则

### 1. 库不读取宿主配置文件

库只接受宿主传入的配置对象，不负责自行读取：

- `client_budgets.yaml`
- `tool_configs.yaml`
- 宿主 temp 路径
- spill 目录
- Lua 包路径
- 动态库路径

也就是说：

**库只接受配置，不拥有配置来源。**

### 2. 宿主负责产品层行为

下列内容不属于 `vulcan-luaskills`：

- MCP 协议对象
- 分页/截断最终渲染
- spill 文件放置路径
- 客户端预算策略
- tool config 文件读取
- system tools 的最终公开名字

这些都应由宿主决定，例如：

- `vulcan-mcp`
- IDE 插件宿主
- 未来的 FFI 宿主

### 3. System 与 Skill 分层

当前模型分为两层：

- `system tools`
  - 给宿主直接对接
  - 宿主可自由改名、封装、决定是否公开
- `skills`
  - 统一能力面
  - 由库统一加载与暴露 entry 真相

也就是说：

**宿主使用 system，最终用户使用 skills。**

## 当前能力

### Runtime Core

- skill 加载与发现
- entry 描述与调用
- help 列表与详情
- 运行时上下文注入
- 运行时结构化结果
- 日志事件回调

### 标准命名空间

当前库负责向 Lua 注入标准能力，例如：

- `vulcan.fs.*`
- `vulcan.path.*`
- `vulcan.process.*`
- `vulcan.os.*`
- `vulcan.json.*`
- `vulcan.cache.*`
- `vulcan.call(...)`
- `vulcan.sqlite.*`
- `vulcan.lancedb.*`
- `vulcan.runtime.*`

### System 侧能力

当前已成形的 system 真相能力包括：

- skill help 列表
- skill help 详情
- runtime lua 执行链
- `vulcan.runtime.lua.exec`

宿主可以自由把这些 system 能力映射成：

- MCP tools
- IDE command
- slash command
- UI 面板
- 自动上下文注入

## 当前代码结构

```text
src/
├─ lib.rs                # 对外导出
├─ lua_engine.rs         # 核心运行时与 skill 调用
├─ lua_skill.rs          # skill.yaml 解析与 skill 模型
├─ entry_descriptor.rs   # entry 描述结构
├─ runtime_context.rs    # 宿主注入上下文
├─ runtime_result.rs     # 运行时结构化结果
├─ runtime_help.rs       # help 树结构
├─ runtime_options.rs    # 宿主传入运行时选项
├─ runtime_logging.rs    # 回调式日志事件
├─ tool_cache.rs         # 运行时缓存能力
├─ sqlite_host.rs        # SQLite 标准能力绑定
└─ lancedb_host.rs       # LanceDB 标准能力绑定
```

## LuaSkills 目录规则

当前 LuaSkills 目录名与 `skill_id` 采用严格规则：

```regex
^[a-z]([a-z0-9-]*[a-z0-9])?$
```

也就是说：

- 只允许小写字母、数字、连字符 `-`
- 不能以数字开头
- 不能以 `-` 结尾
- 不支持大写与其他特殊符号

## 命名规则

当前 skill entry 的 canonical 命名采用：

```text
skill-id-entry-name
```

例如：

- skill：`vulcan-codekit`
- entry：`ast-tree`
- canonical name：`vulcan-codekit-ast-tree`

如果组合后产生重名，则自动追加稳定后缀：

- `...-2`
- `...-3`

## Help 模型

当前 help 不再作为普通 skill tool 真相，而是作为结构化 help 树存在。

help 分成两层：

- 主 help
  - 描述 skill 总览与工作流目录
- 子 help / workflow help
  - 描述具体流程节点

system 层只返回结构化 help 信息；  
宿主决定是否把它转成 Markdown、UI、命令面板或其他形式。

## 宿主如何接入

### Rust 宿主

最推荐的方式是直接通过 Cargo 引入：

```toml
[dependencies]
vulcan-luaskills = { path = "../vulcan-luaskills" }
```

### FFI 宿主

后续可以基于同一套核心实现导出：

- `cdylib`
- `staticlib`

但 FFI 只是另一种产物形态，不是另一套实现。

## 开发

### 检查

```bash
cargo check
```

### 测试

```bash
cargo test --lib
```

## 配套项目

- [`vulcan-mcp`](https://github.com/OpenVulcan/vulcan-mcp)
  - MCP 宿主与协议适配层

## License

MIT

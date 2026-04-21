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

### Skill 依赖路径注入

运行时还会为当前正在执行的 skill 注入标准依赖根路径：

- `vulcan.deps.tools_path`
- `vulcan.deps.lua_path`
- `vulcan.deps.ffi_path`

这些路径由宿主根据当前 skill 所在空间与依赖目录规则计算后注入。  
skill 应该：

- 使用 `vulcan.context.*` 查找自身代码、帮助和资源
- 使用 `vulcan.deps.*` 查找当前 skill 的工具、Lua 依赖和 FFI 依赖

skill **不应该**：

- 通过 `..` 反推 runtime 根目录
- 猜测宿主目录名是否叫 `dependencies`、`deps`、`bin/tools`
- 自己拼接其他 skill 的依赖路径

一句话说：

**skill 只能依赖协议暴露的路径，不应该依赖宿主的物理目录实现细节。**

### System 侧能力

当前已成形的 system 真相能力包括：

- skill help 列表
- skill help 详情
- runtime lua 执行链
- `vulcan.runtime.lua.exec`
- 可选的 `vulcan.runtime.skills.*` 宿主管理桥接

宿主可以自由把这些 system 能力映射成：

- MCP tools
- IDE command
- slash command
- UI 面板
- 自动上下文注入

其中 `vulcan.runtime.skills.*` 采用宿主显式授权模型：

- 默认关闭
- 由宿主通过 `LuaRuntimeHostOptions.capabilities.enable_skill_management_bridge` 决定是否开放
- 即使开放，也必须由宿主注册技能管理回调后才会真正执行安装、更新、启停、卸载

这意味着：

- 拥有自己 TUI、GUI 或专用管理面的宿主，可以保持关闭
- 愿意允许 skill 调起管理动作的宿主，可以显式打开
- 未注册回调时，Lua 侧会收到明确错误，而不是静默成功

## 当前代码结构

```text
src/
├─ lib.rs                # 对外导出与兼容 re-export
├─ ffi.rs                # JSON 风格 FFI 接口与统一导出清单
├─ ffi_standard.rs       # 标准结构化 C ABI FFI 接口
├─ dependency/           # skill 依赖解析、安装与清理
├─ download/             # GitHub / URL / archive 下载与校验
├─ host/                 # 宿主回调与宿主选项模型
├─ providers/            # SQLite / LanceDB provider 绑定
├─ runtime/              # 引擎、上下文、帮助、结果与日志
└─ skill/                # manifest、来源记录、生命周期管理
```

当前 `ffi.rs` 与 `ffi_standard.rs` 仍位于 `src` 根目录，原因是：

- 它们都是顶层对外接口入口
- 直接依赖 `runtime`、`skill`、`host` 等多个子模块
- 当前文件规模仍可控，放在根目录更利于让宿主快速定位 FFI 导出面

如果后续继续扩展：

- FFI 回调
- 自动生成绑定
- 更多语言专用辅助层
- 更细的共享 ABI 类型

则更推荐进一步收敛成：

```text
src/
└─ ffi/
   ├─ mod.rs
   ├─ json.rs
   ├─ standard.rs
   ├─ types.rs
   └─ memory.rs
```

也就是说，**当前结构可以继续使用，但当 FFI 再显著扩张时，建议再下沉为独立目录模块。**

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

#### FFI C ABI

当前库已经提供两套并存的稳定 C ABI，用于让非 Rust 宿主通过：

- `cdylib`
- `staticlib`

直接接入同一套 LuaSkills 核运行时。

FFI 设计规则如下：

- 所有直接集成的核心引擎接口都同时提供：
  - 标准结构化接口
  - `_json` 结尾的 JSON 通用接口
- 结构明确、性能敏感的接入方应优先使用标准接口
- 动态语言或快速集成场景可以使用 `_json` 接口
- `_json` 接口统一使用 JSON 包络：
  - 成功：`{"ok":true,"result":...}`
  - 失败：`{"ok":false,"error":"..."}`
- 返回的字符串必须通过：
  - `vulcan_luaskills_ffi_string_free`
  释放

标准接口当前采用：

- 原生 C ABI 参数
- `error_out` 输出英文错误文本
- 复杂列表/结果结构通过专用 free 函数释放
- `source_type` 采用稳定整数协议：
  - `-1 = absent`
  - `0 = github`
  - `1 = url`

说明：

- 对于真正动态的值，例如 `run_lua` 的任意 JSON 返回值、`client_budget/tool_config` 一类上下文对象，
  标准接口仍会使用 JSON 字符串承载内容
- 这是为了避免把任意 JSON 树硬编码成脆弱的固定 C 结构

头文件位置：

- [include/vulcan_luaskills_ffi.h](include/vulcan_luaskills_ffi.h)

当前已导出的核心 FFI 能力包括：

- 引擎创建与释放
- `load/reload`
- `list_entries`
- `list_skill_help`
- `render_skill_help_detail`
- `prompt_argument_completions`
- `call_skill`
- `run_lua`
- `enable/disable`
- `install/update/uninstall`

完整对接文档：

- [docs/FFI_INTEGRATION_GUIDE.md](docs/FFI_INTEGRATION_GUIDE.md)

语言示例：

- [examples/ffi/python/demo.py](examples/ffi/python/demo.py)
- [examples/ffi/go/demo.go](examples/ffi/go/demo.go)
- [examples/ffi/typescript/demo.ts](examples/ffi/typescript/demo.ts)
- [examples/ffi/demo_runtime/README.md](examples/ffi/demo_runtime/README.md)

这些示例当前都采用同一规则：

- 通过环境变量 `VULCAN_LUASKILLS_LIB` 指向动态库文件
- 标准示例优先演示 `version / engine_new / engine_free`
- `demo_runtime` 目录额外提供一条真实的安装与调用烟测链
- 动态安装与调用部分通过 `_json` 接口完成

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

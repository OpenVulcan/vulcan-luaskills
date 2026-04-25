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

如果您当前主要是**写 Lua skill**，建议先看：

1. [docs/SKILL_DEVELOPER_MANUAL.md](docs/SKILL_DEVELOPER_MANUAL.md)
2. [docs/HOST_DATABASE_PROVIDER_GUIDE.md](docs/HOST_DATABASE_PROVIDER_GUIDE.md)

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

## FFI Beta 摘要

如果您是第一次打开这个仓库，并且主要关注 FFI，对当前 `v0.1.x / beta` 阶段可以直接这样理解：

- Rust 直连仍然是主集成方式
- 标准 C ABI 是低层正式宿主契约
- 公共 `_json` FFI 是高层易用公共接口
- 当前 FFI 更适合作为**受控宿主集成接口**
- 当前运行时默认把 skill 当作受信代码看待，不提供 Lua skill 沙箱安全承诺

也就是说：

- Rust 宿主优先直接接 Rust API
- C / C++ / Go 这类低层宿主优先看标准 C ABI
- Python / Node.js / TypeScript 这类动态宿主优先看公共 `_json` FFI

如果您只想先抓一份最短说明，建议按下面顺序阅读：

1. [docs/FFI_BETA_RELEASE_NOTES.md](docs/FFI_BETA_RELEASE_NOTES.md)
2. [docs/FFI_HOST_CHECKLIST.md](docs/FFI_HOST_CHECKLIST.md)
3. [docs/FFI_INTEGRATION_GUIDE.md](docs/FFI_INTEGRATION_GUIDE.md)
4. [docs/HOST_DATABASE_PROVIDER_GUIDE.md](docs/HOST_DATABASE_PROVIDER_GUIDE.md)

如果您只想先看示例，建议按下面顺序阅读：

1. [examples/ffi/c/demo.c](examples/ffi/c/demo.c)
2. [examples/ffi/python/demo.py](examples/ffi/python/demo.py)
3. [examples/ffi/go/demo.go](examples/ffi/go/demo.go)
4. [examples/ffi/typescript/demo.ts](examples/ffi/typescript/demo.ts)
5. [examples/ffi/typescript/README.md](examples/ffi/typescript/README.md)

## 发布产物分层

GitHub 端的 Lua 依赖构建不再只面向旧版 `deps-v1` 的 C 依赖包，而是按版本号 tag（例如 `v0.1.0`）拆成多类产物：

- `lua-runtime-{platform}.tar.gz`：运行期 Lua 包、原生运行库、资源清单与授权材料。
- `luaskills-ffi-sdk-{platform}.tar.gz`：FFI 头文件、动态库/链接库、SDK manifest 与授权材料。
- `luaskills-demo-ffi-{platform}.tar.gz`：动态库宿主 demo，面向 C / Python / Go / TypeScript 等 FFI 接入方式。
- `luaskills-demo-rust-{platform}.tar.gz`：Rust 直连 demo，面向直接依赖 crate 的非 FFI 接入方式。

运行期包只导出 `lua_packages/lib/lua`、`lua_packages/share/lua`、`libs`、`resources` 和 `licenses`。构建工具、LuaRocks、LuaJIT SDK、`lua51.dll` 等仅用于编译链路的内容不会作为 runtime 默认内容导出。打包脚本会迭代扫描 Lua C 模块、release 动态库以及已复制进 `libs/` 的下游依赖，将命中的 zlib、curl、OpenSSL、pcre2、libyaml 等运行库复制到 `libs/`，避免目标机器未安装对应系统包时运行失败。runtime 包还会按平台生成加载器辅助脚本：Windows 包携带 `resources/runtime-env.ps1`，Linux/macOS 包携带 `resources/runtime-env.sh`，并统一生成 `resources/bundled-libs.json` 记录实际复制库的来源。

demo / 源码环境可使用统一拉取脚本：

```powershell
.\scripts\fetch_runtime_deps.ps1 -Target all
.\scripts\fetch_runtime_deps.ps1 -Target lua
.\scripts\fetch_runtime_deps.ps1 -Target vldb
```

其中 `vldb` 会把 `vldb-controller(.exe)` 安装到运行根的 `bin/` 目录，匹配 demo 默认目录约定。

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

宿主还可以通过 `LuaRuntimeHostOptions.ignored_skill_ids` 提供一个强制忽略列表：

- 匹配对象是 skill 目录派生出的 `skill_id`，不是 `skill.yaml` 的展示名称
- 命中后该 skill 会在加载早期被跳过
- 被跳过的 skill 不会准备依赖、不会绑定 SQLite/LanceDB，也不会注册 entry
- 该能力适合宿主已经把某类能力切换到原生、gRPC、VMM 或其他更强实现时，用来屏蔽默认包或冲突包

这不是自动 capability 反判定系统，也不是 skill 自己决定是否启用的机制。  
最终是否忽略某个 skill，仍由宿主策略和用户安装/禁用意图决定。

### 2.4 统一 Skill 配置系统

当前 `luaskills` 已内建统一的 skill 配置存储协议：

- 物理上只有一个主配置文件
- 默认路径是 `<runtime_root>/config/skill_config.json`
- 宿主可通过 `LuaRuntimeHostOptions.skill_config_file_path` 显式覆盖
- 一旦显式提供 `skill_config_file_path`，运行时将不再推导默认 `runtime_root`
- 显式路径会在引擎创建时固定成绝对路径，不会随进程 `cwd` 漂移
- 即使 `skills/` 目录暂时还不存在，也会优先解析这条默认配置路径
- 如果传入了多个 skill root 且它们映射到不同的 `runtime_root`，则必须显式提供 `skill_config_file_path`
- 文件内部按 `skill_id` 分组存储
- 第一版配置值统一为 `string`

Lua 侧当前可直接使用：

- `vulcan.config.get(key)`
- `vulcan.config.set(key, value)`
- `vulcan.config.delete(key)`
- `vulcan.config.has(key)`
- `vulcan.config.list()`

其中：

- Lua 侧默认只访问当前 skill 自己的配置命名空间
- 未配置某个 key 时，不会自动让 skill 失效
- 更推荐由 skill 自己返回提示，告知用户如何通过宿主配置工具补齐所需配置

宿主侧则可通过 Rust API、标准 C ABI 或公共 `_json` FFI 包装成一个单工具，例如：

```text
runtime-config(action, skill_id?, key?, value?)
```

推荐动作只有四类：

- `list`
- `get`
- `set`
- `delete`

也就是说：

- skill 自己通过 `vulcan.config.*` 读写当前命名空间
- 宿主与 AI 则通过一个统一的 `runtime-config` 工具跨 skill 管理配置

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

其中隔离 `vulcan.runtime.lua.exec` 已经拥有独立的专用 VM 池：

- 默认配置是 `min_size=1 / max_size=4 / idle_ttl_secs=60`
- 宿主可以通过 `LuaRuntimeHostOptions.runlua_pool_config` 覆盖
- 该池只作用于隔离 `luaexec` 路径，不改变普通 skill VM 池和普通 `run_lua` 主池行为
- 当前不再提供外部 `luaexec` 执行器路径配置，隔离执行统一在当前进程内完成

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

当前库对外提供两层 FFI 接入面，用于让非 Rust 宿主通过：

- `cdylib`
- `staticlib`

直接接入同一套 LuaSkills 核运行时。

FFI 设计规则如下：

- 标准 C ABI：
  - 面向低层正式宿主契约
  - 适合结构明确、性能敏感的接入方
- 公共 `_json` FFI：
  - 面向动态语言和快速集成场景
  - 适合不想维护复杂 ABI 结构的宿主
- 公共 `_json` FFI 统一使用 JSON 包络：
  - 成功：`{"ok":true,"result":...}`
  - 失败：`{"ok":false,"error":"..."}`
- 返回的拥有型文本/字节缓冲必须通过：
  - `vulcan_luaskills_ffi_buffer_free`
  释放
- 结构化结果对象必须通过各自专用的 free 函数释放
- 只有公共 JSON FFI / helper 层仍明确返回裸字符串指针的辅助接口，才使用：
  - `vulcan_luaskills_ffi_string_free`

标准 C ABI 当前采用：

- 原生 C ABI 参数
- `error_out` 输出拥有型 UTF-8 错误缓冲
- 复杂列表/结果结构通过专用 free 函数释放
- `source_type` 采用稳定整数协议：
  - `-1 = absent`
  - `0 = github`
  - `1 = url`

说明：

- 对于真正动态的值，例如 `run_lua` 的任意 JSON 返回值、`client_budget/tool_config` 一类上下文对象，
  标准 C ABI 仍会使用 JSON 字符串承载内容
- 这是为了避免把任意 JSON 树硬编码成脆弱的固定 C 结构

#### FFI 接口怎么选

如果您是第一次决定接入方式，可以直接按下面的结论判断：

- 如果宿主本身是 Rust：
  - 优先直接引用 Rust API
  - 不建议为了“统一接口”额外绕一层 FFI
- 如果宿主是 C / C++ / Go / 其他能稳定处理结构体和 out 指针的语言：
  - 优先使用标准 C ABI
  - 这样更接近正式底层契约，后续升级路径也更稳定
- 如果宿主是 Python / Node.js / TypeScript / 动态脚本环境：
  - 优先使用公共 `_json` FFI
  - 这样更省去复杂结构体绑定和生命周期细节管理
- 如果宿主同时有“正式主链”与“快速扩展入口”两类需求：
  - 可以混合使用
  - 标准 C ABI 负责 `engine/load/list/call/lifecycle`
  - 公共 `_json` FFI 负责动态安装、快速原型和调试链路

当前 `beta / v0.1.x` 阶段的推荐理解是：

- Rust 直连仍然是主集成方式
- 标准 C ABI 是低层正式宿主契约
- 公共 `_json` FFI 是高层易用公共接口
- 两者不是互斥关系，而是面向不同接入成本与宿主能力的两层交付

#### FFI 固定术语

为了避免 README、对接指南和示例文档中出现多套混用叫法，当前固定使用下面这组术语：

- `标准 C ABI`
  - 指低层、结构化、面向正式宿主契约的 FFI 接口层
- `公共 `_json` FFI`
  - 指高层、JSON 包络、面向动态语言和快速集成的 FFI 接口层
- `标准 C ABI 头文件`
  - 指 [include/vulcan_luaskills_ffi.h](include/vulcan_luaskills_ffi.h)
- `公共 `_json` FFI 头文件`
  - 指 [include/vulcan_luaskills_json_ffi.h](include/vulcan_luaskills_json_ffi.h)

如果后续文档里为了简化阅读而出现“标准 ABI”“标准接口”“JSON 接口”等缩写说法，都默认回指上面这两套固定术语。

头文件位置：

- 标准 C ABI：
  - [include/vulcan_luaskills_ffi.h](include/vulcan_luaskills_ffi.h)
- 公共 `_json` FFI：
  - [include/vulcan_luaskills_json_ffi.h](include/vulcan_luaskills_json_ffi.h)

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

- [docs/FFI_BETA_RELEASE_NOTES.md](docs/FFI_BETA_RELEASE_NOTES.md)
- [docs/FFI_INTEGRATION_GUIDE.md](docs/FFI_INTEGRATION_GUIDE.md)
- [docs/FFI_HOST_CHECKLIST.md](docs/FFI_HOST_CHECKLIST.md)
- [docs/HOST_DATABASE_PROVIDER_GUIDE.md](docs/HOST_DATABASE_PROVIDER_GUIDE.md)

语言示例：

- [examples/ffi/c/demo.c](examples/ffi/c/demo.c)
- [examples/ffi/python/demo.py](examples/ffi/python/demo.py)
- [examples/ffi/python/lifecycle_demo.py](examples/ffi/python/lifecycle_demo.py)
- [examples/ffi/python/query_demo.py](examples/ffi/python/query_demo.py)
- [examples/ffi/go/demo.go](examples/ffi/go/demo.go)
- [examples/ffi/go/lifecycle_demo/main.go](examples/ffi/go/lifecycle_demo/main.go)
- [examples/ffi/go/query_demo/main.go](examples/ffi/go/query_demo/main.go)
- [examples/ffi/typescript/demo.ts](examples/ffi/typescript/demo.ts)
- [examples/ffi/typescript/lifecycle_demo.ts](examples/ffi/typescript/lifecycle_demo.ts)
- [examples/ffi/typescript/query_demo.ts](examples/ffi/typescript/query_demo.ts)
- [examples/ffi/c/README.md](examples/ffi/c/README.md)
- [examples/ffi/standard_runtime/README.md](examples/ffi/standard_runtime/README.md)
- [examples/ffi/demo_runtime/README.md](examples/ffi/demo_runtime/README.md)
- [examples/ffi/host_provider_demo/README.md](examples/ffi/host_provider_demo/README.md)

这些示例当前遵循两类接入方式：

- `c/demo.c`
  - 通过标准头文件与链接产物直接演示标准 C ABI 下的 `version / engine_new / load_from_roots / list_entries / call_skill / run_lua / engine_free`
- Python / Go / TypeScript / standard_runtime / demo_runtime / host_provider_demo
  - 通过环境变量 `VULCAN_LUASKILLS_LIB` 指向动态库文件
- `python/lifecycle_demo.py`
  - 额外演示标准 ABI 下的 `disable_skill / enable_skill` 生命周期切换
- `python/query_demo.py`
  - 额外演示标准 ABI 下的 `is_skill / skill_name_for_tool / prompt_argument_completions`
- `go/lifecycle_demo/main.go`
  - 额外演示标准 ABI 下的 `disable_skill / enable_skill` 生命周期切换
- `go/query_demo/main.go`
  - 额外演示标准 ABI 下的 `is_skill / skill_name_for_tool / prompt_argument_completions`
- `typescript/lifecycle_demo.ts`
  - 额外演示标准 ABI 下的 `disable_skill / enable_skill` 生命周期切换
- `typescript/query_demo.ts`
  - 额外演示标准 ABI 下的 `is_skill / skill_name_for_tool / prompt_argument_completions`
- `standard_runtime` 目录提供标准 ABI 示例共用的最小 skill 夹具
- Python / Go / TypeScript 标准示例当前也已覆盖 `load_from_roots + list_entries + call_skill + run_lua` 的结构化结果读取
- `demo_runtime` 目录额外提供一条真实的安装与调用烟测链
- 动态安装与调用部分通过公共 `_json` FFI 完成
- `host_provider_demo` 目录额外提供一条“宿主通过 host_callback 模式接管 SQLite 数据库落点”的独立烟测链

#### FFI 示例怎么选

如果您是第一次接触当前 FFI 接口，建议直接按目标选择示例，而不要一次性把所有目录都读完：

- 想先跑通标准 ABI 的最短闭环：
  - 先看 [examples/ffi/c/demo.c](examples/ffi/c/demo.c)
  - 或看 [examples/ffi/python/demo.py](examples/ffi/python/demo.py)
  - 或看 [examples/ffi/go/demo.go](examples/ffi/go/demo.go)
  - 或看 [examples/ffi/typescript/demo.ts](examples/ffi/typescript/demo.ts)
- 想看技能启停后的运行时变化：
  - 看 [examples/ffi/python/lifecycle_demo.py](examples/ffi/python/lifecycle_demo.py)
  - 看 [examples/ffi/go/lifecycle_demo/main.go](examples/ffi/go/lifecycle_demo/main.go)
  - 看 [examples/ffi/typescript/lifecycle_demo.ts](examples/ffi/typescript/lifecycle_demo.ts)
- 想看查询辅助接口：
  - 看 [examples/ffi/python/query_demo.py](examples/ffi/python/query_demo.py)
  - 看 [examples/ffi/go/query_demo/main.go](examples/ffi/go/query_demo/main.go)
  - 看 [examples/ffi/typescript/query_demo.ts](examples/ffi/typescript/query_demo.ts)
- 想看标准 ABI 示例共用的最小 skill 夹具：
  - 看 [examples/ffi/standard_runtime/README.md](examples/ffi/standard_runtime/README.md)
- 想看动态安装与真实调用烟测：
  - 看 [examples/ffi/demo_runtime/README.md](examples/ffi/demo_runtime/README.md)
- 想看宿主如何接管数据库 provider：
  - 看 [examples/ffi/host_provider_demo/README.md](examples/ffi/host_provider_demo/README.md)

一句话建议：

- 标准 C ABI 学习入口优先从 `demo / lifecycle_demo / query_demo` 这一组看
- 公共 `_json` FFI 与宿主 provider 接管链路，再按需要进入 `demo_runtime / host_provider_demo`

#### 宿主数据库 Provider

当前数据库接入不再只有一种方式。

每个数据库后端都支持三种模式：

- `dynamic_library`
- `host_callback`
- `space_controller`

其中：

- `dynamic_library`
  - 由 lib 自己加载本地数据库动态库
- `host_callback`
  - 由宿主注册数据库 provider 回调
  - lib 把数据库请求和稳定绑定上下文回调给宿主
- `space_controller`
  - 由 lib 把数据库请求转发给外部空间控制器
  - 代码层通过 `git + tag v0.2.1` 固定依赖 `vldb-controller-client`
  - 当前上游 Rust SDK 注册字段为 `client_name`，会话主键 `client_session_id` 由 SDK 内部自动管理
  - 稳定 `binding_tag` 只保留诊断与命名语义，controller 实际使用的 `binding_id` 会由 lib 结合当前 client 会话域派生，避免不同客户端实例抢占同一 binding
  - `v0.2.1` 额外修复了本地共享 controller 自动拉起阶段的重复拉起协调风险
  - 服务进程本体不走 Cargo 依赖注入，而是由宿主复制本地 controller 可执行文件后，通过 `space_controller.executable_path` 指定启动路径
  - 宿主不指定 `endpoint` 时，默认连接共享端点 `http://127.0.0.1:19801`
  - 宿主指定独立端点时，可切换到独占 controller 实例

而 `host_callback` 模式内部再细分两种回调传输方式：

- `standard`
- `json`

也就是说：

- 宿主如果偏向高性能和稳定 ABI，可以实现 `standard` 回调
- 宿主如果偏向动态语言、快速接入和原型验证，可以实现 `json` 回调

并且这些模式是**按后端分别配置**的：

- `sqlite_provider_mode`
- `sqlite_callback_mode`
- `lancedb_provider_mode`
- `lancedb_callback_mode`

这意味着宿主可以出现混合组合，例如：

- SQLite 使用 `host_callback + json`
- LanceDB 使用 `dynamic_library`

或者：

- SQLite 使用 `dynamic_library`
- LanceDB 使用 `space_controller`

这部分完整说明见：

- [docs/HOST_DATABASE_PROVIDER_GUIDE.md](docs/HOST_DATABASE_PROVIDER_GUIDE.md)

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

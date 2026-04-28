# luaskills FFI 宿主接入检查清单

## 1. 这份清单的用途

这份清单不是完整设计说明，也不是 API 逐项参考。  
它的目标只有一个：

- 让宿主在第一次接入 `luaskills` FFI 时，能按最短路径完成自检

如果您需要完整背景说明，请继续阅读：

- [FFI_INTEGRATION_GUIDE.md](FFI_INTEGRATION_GUIDE.md)
- [HOST_DATABASE_PROVIDER_GUIDE.md](HOST_DATABASE_PROVIDER_GUIDE.md)

## 2. 先选接入面

在真正写宿主代码之前，先确定这一步：

- 如果宿主本身是 Rust：
  - 优先直接接 Rust API
- 如果宿主是 C / C++ / C# / 其他能稳定处理结构体和 out 指针的语言：
  - 优先接标准 C ABI
- 如果宿主是 Python / Node.js / TypeScript / 动态脚本环境：
  - 优先接公共 `_json` FFI
  - TypeScript / Node.js 优先使用 [sdk/typescript](../sdk/typescript) 的 `@luaskills/sdk`，其中已经封装 JSON provider callback 注册与清理
- 如果宿主是 Python：
  - 优先使用 [sdk/python](../sdk/python) 的 `luaskills-sdk`，其中已经封装 JSON provider callback 注册与清理
- 如果宿主是 Go：
  - 可直接接标准 C ABI
  - 也可使用 [sdk/go](../sdk/go) 的 cgo JSON FFI SDK；该路径需要 `CGO_ENABLED=1`、C 编译器、链接库搜索路径与运行时动态库路径
  - 若需要 provider callback，需要宿主工程自行实现受控 cgo callback bridge，SDK 会通过显式错误提示这条边界
- 如果宿主需要“稳定主链 + 快速调试链”：
  - 可以混合使用
  - 标准 C ABI 负责主链
  - 公共 `_json` FFI 负责快速桥接和动态调试

## 3. 启动前检查

在 `engine_new` 之前，先确认这些条件：

- 已经准备好宿主运行时目录：
  - `temp`
  - `resources`
  - `lua_packages`
  - `dependencies`
  - `state`
  - `databases`
- 已经决定数据库 provider 模式：
  - `dynamic_library`
  - `host_callback`
  - `space_controller`
- 如果要用 callback：
  - callback 必须先注册，再创建 engine
  - TypeScript / Python 宿主优先使用 SDK 的 `set_*_provider_json_callback`，不要在业务代码里手写 buffer clone
- 如果要用 `space_controller`：
  - 已确认 `endpoint / auto_spawn / executable_path / process_mode`
- 如果连接远端 controller：
  - 必须关闭 `auto_spawn`
- 如果宿主会高频使用 `vulcan.runtime.lua.exec`：
  - 已决定是否覆盖 `runlua_pool_config`
  - 未配置时默认是 `min=1 / max=4 / idle_ttl_secs=60`
- 如果宿主需要屏蔽默认包或冲突包：
  - 在 `FfiLuaRuntimeHostOptions.ignored_skill_ids` 填入对应目录派生的 `skill_id`
  - 被忽略 skill 不会准备依赖、不会绑定数据库，也不会注册 entry

## 4. 标准创建顺序

第一次接入最推荐按这个顺序实现：

1. `version`
2. `engine_new`
3. `load_from_roots`
4. `list_entries`
5. `call_skill`
6. `run_lua`
7. `engine_free`

如果这条链还没跑通，不建议先去接：

- `install / update / uninstall`
- 数据库 provider callback
- `space_controller`

正式宿主构造 skill roots 时，建议先固定三层语义：

```text
ROOT -> PROJECT -> USER
```

- `ROOT` 是系统控制级，只通过 system tools 或受控 system updater 调整。
- `PROJECT` / `USER` 是普通用户管理面可操作层。
- `ROOT` root 必须出现在启动或加载 root 链中；缺失时应直接报错。
- 普通 `vulcan.runtime.skills.*` 不应暴露 `ROOT` 目标选项。
- 若开放普通技能管理桥接，应同时提供层级列表能力，例如 `vulcan.runtime.skills.layers()`，让调用方获取当前实际存在的 `PROJECT` / `USER` 标签；bridge 关闭时不要把层级标记为可写。
- `ROOT` 中存在同名 `skill_id` 时，任何 authority 都不能向 `PROJECT` / `USER` install 或 update 同名 skill；普通层显式 uninstall 可用于清理残留。
- 若将 system tools 暴露给普通 tools，宿主 wrapper 必须固定注入 `DelegatedTool` authority；只有管理员、修复或受控更新流程才应注入 `System`。
- 查询与 prompt completion 类 FFI 入口也必须注入 authority；`DelegatedTool` 下不得返回 `ROOT` entries、help detail、`is_skill=true` 或 ROOT tool name 归属。`call_skill` / `run_lua` 是运行时执行面，不作为 ROOT 可见性边界；如果不希望普通用户执行任意 Lua，应由宿主单独封装或不暴露 `run_lua`。
- skill config 接口按 `skill_id` 管理配置，不按 root 可见性过滤；配置只有被 Lua 通过 `vulcan.config.*` 读取时才会影响行为。若不希望客户修改配置，不应暴露对应 `set/delete` 能力，核心行为应通过宿主硬逻辑或内置核心 skill 固化。
- `protected_skill_ids` 已取消，不应再作为宿主接入参数或普通管理保护机制。

## 5. 生命周期与查询辅助的第二阶段顺序

基础调用链打通后，再按这个顺序往下补：

1. `disable_skill / enable_skill`
2. `is_skill`
3. `skill_name_for_tool`
4. `prompt_argument_completions`
5. `list_skill_help`
6. `render_skill_help_detail`

这样更容易定位问题，不会把“运行时主链问题”和“辅助接口问题”混在一起。

## 6. 内存释放检查

这是最容易误用的部分，建议逐项对照：

- 标准 C ABI 接口失败信息：
  - 通过 `FfiOwnedBuffer error_out` 返回
  - 读取后必须 `luaskills_ffi_buffer_free`
- 标准 C ABI 接口的单值文本输出：
  - 例如 `version_out` / `skill_id_out` / `result_json_out`
  - 也应按 `FfiOwnedBuffer` 读取与释放
- 结构化结果：
  - 不能手动释放内部字段
  - 必须调用结构体专用 free 函数
- 字符串数组：
  - 必须调用 `luaskills_ffi_string_array_free`
- 裸字符串辅助函数：
  - `luaskills_ffi_string_free` 只能释放 **luaskills 自己分配** 的字符串

一句话规则：

- 单值文本看 `FfiOwnedBuffer`
- 结构体结果看专用 free
- 不要自己猜该释放什么

## 7. 指针与缓冲规则

宿主在传参时要特别确认：

- `FfiBorrowedBuffer.ptr` 在调用期间必须有效
- `len > 0` 时，`ptr` 不能为 null
- 不能把宿主自己的内存伪装成 `FfiOwnedBuffer`
- 不能把宿主自己的字符串交给 `luaskills_ffi_string_free`

## 8. 回调与线程规则

如果宿主要接 callback，请对照下面几条：

- callback 必须在 `engine_new` 前注册
- callback 不能跨 C ABI 抛异常
- 同一线程内，不支持在一个 engine 调用尚未返回时再次重入同一个 engine
- 如果一个进程里需要多套 callback 逻辑：
  - 应分别创建不同 engine
  - 不要指望在 engine 创建后再切换全局 callback
- Go 宿主的 provider callback 不应直接挂临时闭包给进程级 C 回调；应先在宿主层设计明确的 cgo bridge、线程模型和生命周期。

## 9. 标准 C ABI 与公共 `_json` FFI 的最短判断

如果还在犹豫该走哪条路，直接按下面判断：

- 想要更稳定的底层契约：
  - 走标准 C ABI
- 想更快接进 Python / Node / TypeScript：
  - 走公共 `_json` FFI
- 想以后接更多语言绑定：
  - 先把标准 C ABI 跑通
- 想快速验证功能闭环：
  - 先跑公共 `_json` FFI 或 Python 示例

## 10. 示例入口速查

按目标直接选示例：

- 最短标准 ABI 闭环：
  - [examples/ffi/c/demo.c](../examples/ffi/c/demo.c)
  - [examples/ffi/python/demo.py](../examples/ffi/python/demo.py)
  - [examples/ffi/go/demo.go](../examples/ffi/go/demo.go)
  - [examples/ffi/typescript/demo.ts](../examples/ffi/typescript/demo.ts)
- 生命周期切换：
  - [examples/ffi/python/lifecycle_demo.py](../examples/ffi/python/lifecycle_demo.py)
  - [examples/ffi/go/lifecycle_demo/main.go](../examples/ffi/go/lifecycle_demo/main.go)
  - [examples/ffi/typescript/lifecycle_demo.ts](../examples/ffi/typescript/lifecycle_demo.ts)
- 查询辅助接口：
  - [examples/ffi/python/query_demo.py](../examples/ffi/python/query_demo.py)
  - [examples/ffi/go/query_demo/main.go](../examples/ffi/go/query_demo/main.go)
  - [examples/ffi/typescript/query_demo.ts](../examples/ffi/typescript/query_demo.ts)
- 标准 ABI 共用夹具：
  - [examples/ffi/standard_runtime/README.md](../examples/ffi/standard_runtime/README.md)
- 动态安装烟测：
  - [examples/ffi/demo_runtime/README.md](../examples/ffi/demo_runtime/README.md)
- 宿主 provider 接管：
  - [sdk/typescript/examples/provider-callback.mjs](../sdk/typescript/examples/provider-callback.mjs)
  - [sdk/python/examples/provider_callback.py](../sdk/python/examples/provider_callback.py)
  - pip 安装后可运行 `python -m luaskills.examples.provider_callback`
  - [sdk/go/examples/provider_callback/main.go](../sdk/go/examples/provider_callback/main.go)
  - [examples/ffi/host_provider_demo/README.md](../examples/ffi/host_provider_demo/README.md)

## 11. 发布前最小自测

如果宿主准备进入 beta 联调，至少确认下面这些项目都通过：

- `engine_new -> load_from_roots -> list_entries -> call_skill -> run_lua -> engine_free`
- `disable_skill / enable_skill` 能反映到运行时视图
- `is_skill / skill_name_for_tool / prompt_argument_completions` 返回符合预期
- 所有 `error_out` 都能被正确读取和释放
- 所有结构化结果都通过专用 free 回收
- callback 场景下没有跨 ABI 异常
- callback 场景下没有同线程重入
- 普通技能管理工具不会把 `ROOT` 暴露给用户安装、更新或卸载
- 若存在 ROOT 级系统 skill，已确认 PROJECT / USER 同名 skill 不会被加载

只要这组检查全部通过，宿主接入通常就已经具备 beta 联调基础。

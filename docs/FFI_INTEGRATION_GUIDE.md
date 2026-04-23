# vulcan-luaskills FFI 对接文档

## 1. 文档目标

本文档用于说明 `vulcan-luaskills` 当前导出的 FFI 接口设计、启动条件、参数模型、调用顺序、返回规则、内存释放方式，以及 install / update / uninstall 等整条链路的处理逻辑。

如果您希望先按最短路径做宿主自检，而不是从头读完整文档，请先看：

- [FFI_BETA_RELEASE_NOTES.md](FFI_BETA_RELEASE_NOTES.md)
- [FFI_HOST_CHECKLIST.md](FFI_HOST_CHECKLIST.md)

本文档覆盖当前对外公开的两层 FFI：

- 标准 C ABI
- 公共 `_json` FFI

两层接口位于同一个动态库或静态库中，主能力基本一一对应，但定位并不相同。

## 2. 整体设计

### 2.1 同一套核心实现

无论是标准 C ABI 还是公共 `_json` FFI，底层都调用同一套 `LuaEngine` 核运行时逻辑。  
这意味着：

- 行为一致
- 生命周期一致
- 错误语义一致
- 事务语义一致

### 2.2 两层 FFI 角色

#### 标准 C ABI

标准 C ABI 使用：

- C ABI 基本类型
- 结构体指针
- 指针加长度
- `error_out`
- 专用 free 函数

适合：

- C / C++
- Go
- 能稳定处理 C 结构与 out 指针的宿主
- 性能敏感场景
- 希望避免双向 JSON 编解码的宿主

它是：

- 低层正式契约
- 结构更稳定的底层接入面
- 后续社区生成绑定时的基础 ABI

#### 公共 `_json` FFI

公共 `_json` FFI 使用：

- 输入：JSON 文本缓冲
- 输出：统一 JSON 包络

适合：

- Python
- TypeScript / Node.js
- ctypes / cffi / ffi-napi 一类动态桥接
- 原型验证
- 快速接入
- 不想维护复杂 ABI 结构的宿主

它是：

- 高层易用接口
- 面向动态语言和快速集成的正式公共接口
- 对标准 C ABI 的便利封装，而不是临时调试层

### 2.3 为什么两层都保留

原因很直接：

- 标准 C ABI 更稳定，但接入门槛更高
- 公共 `_json` FFI 更易用，但双向编解码成本更高

因此当前协议定为：

- 对外继续同时维护两层能力
- 标准 C ABI 负责承载低层正式契约
- 公共 `_json` FFI 负责承载动态语言与快速接入入口

### 2.4 宿主数据库 provider 也遵循双层规则

当宿主需要自己接管：

- SQLite
- LanceDB

时，当前协议同样保留两层回调形式：

- 标准结构化回调
- JSON 回调

也就是说：

- 宿主如果已经能稳定处理 C ABI 结构和 out 指针，应优先接标准回调
- 宿主如果是 Python / Node / 快速原型，则可直接接 JSON 回调

数据库 provider 的详细对接说明见：

- [HOST_DATABASE_PROVIDER_GUIDE.md](HOST_DATABASE_PROVIDER_GUIDE.md)

### 2.5 固定术语

为了避免正文里同时出现“标准 ABI”“标准接口”“标准 FFI”“JSON 接口”等多套说法，当前统一固定为：

- `标准 C ABI`
  - 指低层、结构化、面向正式宿主契约的 FFI 接口层
- `公共 `_json` FFI`
  - 指高层、JSON 包络、面向动态语言和快速集成的 FFI 接口层
- `标准 C ABI 头文件`
  - 指 [include/vulcan_luaskills_ffi.h](../include/vulcan_luaskills_ffi.h)
- `公共 `_json` FFI 头文件`
  - 指 [include/vulcan_luaskills_json_ffi.h](../include/vulcan_luaskills_json_ffi.h)

如果正文里为了简化阅读出现：

- `标准 C ABI 接口`
- `公共 `_json` FFI 接口`

也都只是上面这两套固定术语的缩写表达。

## 3. 动态库与头文件

当前产物方向：

- `rlib`
- `cdylib`
- `staticlib`

核心文件：

- 标准 C ABI 导出：
  - [src/ffi_standard.rs](../src/ffi_standard.rs)
- 公共 JSON FFI 导出：
  - [src/ffi.rs](../src/ffi.rs)
- 标准头文件：
  - [include/vulcan_luaskills_ffi.h](../include/vulcan_luaskills_ffi.h)
- JSON 头文件：
  - [include/vulcan_luaskills_json_ffi.h](../include/vulcan_luaskills_json_ffi.h)
  - 该头文件会复用标准头文件中的共享结构体与释放辅助函数
- 示例：
  - [examples/ffi/c/demo.c](../examples/ffi/c/demo.c)
  - [examples/ffi/python/demo.py](../examples/ffi/python/demo.py)
  - [examples/ffi/go/demo.go](../examples/ffi/go/demo.go)
  - [examples/ffi/typescript/demo.ts](../examples/ffi/typescript/demo.ts)
  - [examples/ffi/c/README.md](../examples/ffi/c/README.md)
  - [examples/ffi/standard_runtime/README.md](../examples/ffi/standard_runtime/README.md)

当前 FFI 版本字符串统一派生自 crate 包版本：

- `env!("CARGO_PKG_VERSION")`

也就是说：

- 标准 C ABI `vulcan_luaskills_ffi_version`
- 公共 JSON FFI `vulcan_luaskills_ffi_version_json`
- 自描述结果中的 `ffi_version`

都与 `Cargo.toml` 中的 `version` 保持同源。

## 4. 引擎与句柄模型

### 4.1 引擎句柄

FFI 不直接暴露 `LuaEngine` 指针，而是通过内部注册表分配一个稳定的 `u64 engine_id`。

宿主流程：

1. 调用 `engine_new` 创建引擎
2. 获取 `engine_id`
3. 后续所有 load / help / call / install / update / uninstall 都通过这个 `engine_id` 定位引擎
4. 最后调用 `engine_free`

### 4.2 生命周期要求

一个 `engine_id` 的推荐生命周期：

1. `engine_new`
2. `load_from_roots` 或 `load_from_dirs`
3. 若干：
   - `list_entries`
   - `list_skill_help`
   - `render_skill_help_detail`
   - `call_skill`
   - `run_lua`
   - 生命周期操作
4. `engine_free`

如果未先 load，就直接调用依赖 skill 注册表的接口，结果通常是：

- 空列表
- 找不到 skill
- 找不到 entry

因此：

**创建引擎后，必须先完成至少一次加载。**

## 5. 标准 C ABI 接口返回与错误模型

### 5.1 标准 C ABI 接口统一规则

标准 C ABI 接口一般采用：

- 返回值：`int32_t`
- 成功：`0`
- 失败：非 `0`
- 错误消息：通过 `FfiOwnedBuffer *error_out`

若接口需要输出结构化结果，则使用：

- `*_out`
- 再配套 free 函数释放

也就是说：

- 标准 C ABI 接口的失败信息不再通过裸 `char **` 传出
- 调用方应把 `error_out` 当作 UTF-8 错误缓冲读取
- 读取完成后应通过 `vulcan_luaskills_ffi_buffer_free` 释放
- 标准 C ABI 接口中的直接文本输出也在逐步收敛到 `FfiOwnedBuffer`
- 当前 `version_out`、`skill_id_out`、`result_json_out` 都应按拥有型缓冲读取与释放
- 当前 `FfiRuntimeInvocationResult`
  - `content`
  - `template_hint`
  也已改成 `FfiOwnedBuffer`
- 当前 `FfiSkillApplyResult`
  - `skill_id`
  - `status`
  - `message`
  - `version`
  - `source_locator`
  也已改成 `FfiOwnedBuffer`
- 当前 `FfiSkillUninstallResult`
  - `skill_id`
  - `message`
  也已改成 `FfiOwnedBuffer`
- 当前 `FfiRuntimeEntryParameterDescriptor`
  - `name`
  - `param_type`
  - `description`
  也已改成 `FfiOwnedBuffer`
- 当前 `FfiRuntimeEntryDescriptor`
  - `canonical_name`
  - `skill_id`
  - `local_name`
  - `root_name`
  - `skill_dir`
  - `description`
  也已改成 `FfiOwnedBuffer`
- 当前 `FfiRuntimeHelpNodeDescriptor`
  - `flow_name`
  - `description`
  也已改成 `FfiOwnedBuffer`
- 当前 `FfiRuntimeSkillHelpDescriptor`
  - `skill_id`
  - `skill_name`
  - `skill_version`
  - `root_name`
  - `skill_dir`
  也已改成 `FfiOwnedBuffer`
- 当前 `FfiRuntimeHelpDetail`
  - `skill_id`
  - `skill_name`
  - `skill_version`
  - `root_name`
  - `skill_dir`
  - `flow_name`
  - `description`
  - `content_type`
  - `content`
  也已改成 `FfiOwnedBuffer`

### 5.2 `_json` 接口统一规则

`_json` 接口统一返回一个 JSON 包络缓冲：

```json
{"ok":true,"result":{...}}
```

失败时：

```json
{"ok":false,"error":"..."}
```

返回值统一通过：

- `FfiOwnedBuffer`

承载，并通过：

- `vulcan_luaskills_ffi_buffer_free`

释放。

请求输入统一通过：

- `FfiBorrowedBuffer`

传入。

也就是说：

- `_json` 接口的请求输入不再依赖 NUL 终止字符串
- 调用方必须显式提供 `ptr + len`
- `ptr` 只需在当前调用期间保持有效

## 6. 内存所有权与释放规则

### 6.1 字符串辅助函数

由 FFI 返回的独立堆分配字符串必须由调用方释放：

- `vulcan_luaskills_ffi_string_free`

适用于：

- `vulcan_luaskills_ffi_string_clone` 这类字符串辅助函数返回值

说明：

- 这组接口当前属于**公共 JSON FFI / 辅助函数层**
- 标准 C ABI 主结果通道不再推荐依赖裸 `char *`
- 标准 C ABI 应优先使用 `FfiOwnedBuffer` 与结构体专用 free 函数

### 6.2 字符串数组

释放函数：

- `vulcan_luaskills_ffi_string_array_free`

说明：

- `FfiStringArray.items` 当前已经收敛为 `FfiOwnedBuffer *`
- 每个数组元素都是拥有型 UTF-8 文本缓冲
- 但调用方仍然不应手动逐项释放，而应继续把整个数组交给 `vulcan_luaskills_ffi_string_array_free`

### 6.2.1 拥有型缓冲

当 callback 或后续扩展接口返回 `FfiOwnedBuffer` 时：

- 分配应优先使用 `vulcan_luaskills_ffi_buffer_clone`
- 释放应使用 `vulcan_luaskills_ffi_buffer_free`

适用于：

- 标准 C ABI 接口中的 `version_out`
- 标准 C ABI 接口中的 `skill_id_out`
- 标准 C ABI 接口中的 `result_json_out`
- `_json` 接口返回值
- JSON callback 的 `response_out`
- JSON callback 的 `error_out`
- 标准 provider callback 的 `response_json_out`
- 标准 provider callback 的 `meta_json_out`
- 标准 provider callback 的 `data_out`
- 标准 provider callback 的 `error_out`

### 6.3 结构化列表与结果

当前标准 C ABI 接口返回的结构化对象，都有对应 free 函数：

- `vulcan_luaskills_ffi_entry_list_free`
- `vulcan_luaskills_ffi_help_list_free`
- `vulcan_luaskills_ffi_help_detail_free`
- `vulcan_luaskills_ffi_invocation_result_free`
- `vulcan_luaskills_ffi_skill_apply_result_free`
- `vulcan_luaskills_ffi_skill_uninstall_result_free`

说明：

- 这些结构体内部已经开始逐步采用 `FfiOwnedBuffer` 承载文本字段
- 默认仍应把整个结构体交给对应专用 free 函数释放
- 不要在调用专用 free 前手动释放结构体内嵌的 `FfiOwnedBuffer` 字段，否则会导致重复释放
- `related_entries` 这类数组字段当前虽然仍以“数组 + 长度”形式存在，但数组元素本身也已经是 `FfiOwnedBuffer`
- 这类数组字段仍应通过宿主最终调用结构体专用 free 函数统一回收

规则只有一条：

**凡是 FFI 分配出来的结果结构，必须使用对应 free 函数释放。**

### 6.4 Beta 阶段 ABI 迁移要点

当前 `v0.1.x / beta` 阶段已经对现有 FFI 做了一轮直接收敛。  
如果宿主参考的是更早的示例或旧草稿，请优先按下面的对应关系理解：

- 旧：标准 C ABI 接口大量使用 `char **error_out`
  - 新：标准 C ABI 接口统一改成 `FfiOwnedBuffer *error_out`
- 旧：`version_out` / `skill_id_out` / `result_json_out` 这类文本输出按裸字符串读取
  - 新：这些文本输出都应按 `FfiOwnedBuffer` 读取，并通过 `vulcan_luaskills_ffi_buffer_free` 释放
- 旧：`_json` 请求输入依赖 NUL 终止字符串
  - 新：`_json` 请求输入统一改成 `FfiBorrowedBuffer`
- 旧：JSON provider callback 通过裸字符串返回响应
  - 新：JSON provider callback 通过 `FfiOwnedBuffer response_out / error_out` 返回
- 旧：标准 SQLite / LanceDB provider callback 的 `input_json` 是裸字符串
  - 新：标准 provider request 的 `input_json` 统一改成 `FfiBorrowedBuffer`
- 旧：标准 `call_skill / run_lua / render_skill_help_detail` 的请求级 JSON 输入按裸字符串传入
  - 新：这些输入当前统一改成 `FfiBorrowedBuffer`
- 旧：`FfiLuaInvocationContext` 里的三个 JSON 字段按裸字符串传入
  - 新：`FfiLuaInvocationContext` 当前统一改成三段 `FfiBorrowedBuffer`
- 旧：`FfiStringArray.items` 与 `related_entries` 数组元素按 `char **` 理解
  - 新：这类数组元素已经统一收敛为 `FfiOwnedBuffer *`
- 旧：结构体内部文本字段多数按裸 `char *` 处理
  - 新：标准结果结构中的大量文本字段已改成 `FfiOwnedBuffer`

迁移时最容易出错的只有两条：

1. 不要继续把标准错误输出按 `char *` 读取。
2. 不要手动释放结构体内嵌的 `FfiOwnedBuffer` 字段，仍应优先调用结构体专用 free 函数。

### 6.5 beta / v0.1.0 发布边界

当前 FFI 发布面应按以下定位理解：

- 当前版本更适合作为 `beta` / `v0.1.0` 的**受控宿主集成接口**
- 当前版本的主集成方式仍然是 Rust 直连，FFI 主要服务于非 Rust 宿主或跨语言桥接
- FFI 是低层 ABI，不承诺“误用后仍然安全”，宿主必须严格遵守本文档中的所有权、线程与回调规则
- 当前运行时默认把 skill 当作**受信代码**看待，FFI 文档不提供 Lua skill 沙箱安全承诺

### 6.6 必须遵守的 FFI 契约

以下规则应视为强约束，而不是最佳实践建议：

- `vulcan_luaskills_ffi_string_free` 只能释放 **luaskills 自己分配并返回** 的字符串
- `vulcan_luaskills_ffi_string_clone` / `vulcan_luaskills_ffi_bytes_clone` / `vulcan_luaskills_ffi_buffer_clone` 用于把宿主自己的内存复制成 luaskills 自主管理的返回值
- 宿主不能把自己分配的 `malloc/new/string buffer` 直接交给 `vulcan_luaskills_ffi_string_free`
- 宿主不能把自己分配的裸缓冲伪装成 `FfiOwnedBuffer` 交给运行时
- 所有传入 FFI 的裸指针、切片指针、输出指针都必须在调用期间保持有效
- 标准 callback 与 JSON callback 都**不能**把 Rust panic、C++ exception 或其他异常机制穿过 C ABI 边界
- 同一线程内，不支持在一个 engine 的 FFI 调用尚未返回时再次重入同一个 engine
- 若宿主需要数据库 callback 或运行时技能管理 callback，必须先注册 callback，再创建 engine
- callback 返回文本时，文本必须是合法 UTF-8；其中 JSON callback 与标准 provider callback 的 JSON 载荷还必须是合法 JSON 文本

### 6.6 回调与快照规则

当前回调模型需要宿主特别注意：

- database provider callback 会在 `engine_new` 时拍快照
- engine 创建完成后再修改回调注册表，不会 retroactive 地影响已存在 engine
- 若一个进程内需要多套不同 callback 逻辑，应按 callback 集合分别创建 engine，而不是复用同一个 engine 再切换全局回调
- 对动态语言宿主，应优先从 JSON callback 模式接入，因为所有权模型更简单，误用面更小

## 7. 启动条件与前置要求

### 7.1 引擎创建阶段必须提供的关键配置

`LuaEngineOptions` 内部包含：

- VM 池配置
- 宿主目录与路径配置
- 下载能力配置
- 依赖 sibling 目录名配置
- 受保护 skill 配置

其中宿主选项里和运行时目录最直接相关的关键字段包括：

- `temp_dir`
- `resources_dir`
- `lua_packages_dir`
- `host_provided_tool_root`
- `host_provided_lua_root`
- `host_provided_ffi_root`
- `download_cache_root`
- `dependency_dir_name`
- `state_dir_name`
- `database_dir_name`
- `allow_network_download`

### 7.1.1 Space Controller 额外前置要求

当 SQLite 或 LanceDB 的 provider mode 选择 `space_controller` 时，宿主还需要额外准备：

- `LuaRuntimeHostOptions.space_controller.endpoint`
  - 可选
  - 缺失时默认使用共享端点 `http://127.0.0.1:19801`
- `LuaRuntimeHostOptions.space_controller.auto_spawn`
  - 是否允许自动唤起 controller
  - 若为 `true`，`endpoint` 必须使用本地可绑定地址格式，例如 `19801`、`:19801`、`127.0.0.1:19801`、`localhost:19801` 或 `[::1]:19801`
- `LuaRuntimeHostOptions.space_controller.executable_path`
  - 可选
  - 指向宿主已经复制到本地稳定目录的 `vldb-controller` 可执行文件
- `LuaRuntimeHostOptions.space_controller.process_mode`
  - `service`
  - `managed`

这里要特别注意：

- `vulcan-luaskills` 代码层只通过 `git + tag v0.2.1` 固定依赖 `vldb-controller-client`
- 当前上游 Rust SDK 在注册阶段使用 `client_name`，具体 `client_session_id` 由 SDK 内部自动管理并自动回放附着与 backend 期望状态
- `v0.2.1` 额外修复了共享本地 endpoint 在 `auto_spawn` 场景下的重复拉起协调风险
- 真正被拉起的 controller 服务程序，不是通过 Cargo 把二进制嵌进宿主，而是由宿主自行复制并管理
- 也就是说，**Rust SDK 走 git 固定版本，controller 可执行文件走宿主本地复制路径**
- 如果宿主要连接远端 controller 或使用远端主机名端点，必须关闭 `auto_spawn`，避免把远端地址错误地当成本地 bind 地址去拉起新进程

### 7.1.2 callback 注册前置要求

若宿主准备使用：

- `host_callback`
- `vulcan.runtime.skills.*` 的运行时技能管理桥接

则应在 `engine_new` 前完成所有必需 callback 注册。

原因不是初始化顺序习惯，而是当前运行时会在 engine 创建时捕获关键 callback 状态：

- 数据库 provider callback 采用 engine 私有快照
- 技能管理桥接在运行时会显式检查宿主 callback 是否可用

因此：

- 先 `engine_new` 再注册 callback，不应被视为可靠初始化顺序
- 正式宿主接入应把“注册 callback -> 创建 engine -> load/reload -> call”作为固定启动流程

### 7.2 生命周期接口的前置要求

若调用：

- `install`
- `update`
- `uninstall`
- `enable`
- `disable`

则需要：

- 引擎已创建
- 已提供有效的 skill roots
- roots 对应空间满足 sibling 目录协议
- 若是 GitHub 受管安装/更新，还需要允许网络下载

### 7.3 skill 调用前置要求

调用 `call_skill` 前应保证：

- 已成功 `load` 或 `reload`
- 目标工具名存在
- skill help / entry 注册表已经生成

## 8. 标准结构体说明

### 8.1 `FfiLuaVmPoolConfig`

作用：

- 描述 Lua VM 池的大小与回收策略

字段：

- `min_size`
- `max_size`
- `idle_ttl_secs`

### 8.2 `FfiToolCacheConfig`

作用：

- 描述共享工具缓存限制

字段：

- `max_entries`
- `default_ttl_secs`
- `max_ttl_secs`

### 8.2.1 `FfiBorrowedBuffer`

作用：

- 描述一段借用输入缓冲

字段：

- `ptr`
- `len`

约束：

- `ptr` 只在当前 FFI 调用期间有效
- `len > 0` 时，`ptr` 不得为 null

### 8.2.2 `FfiOwnedBuffer`

作用：

- 描述一段由 `luaskills` 拥有并负责释放的输出缓冲

字段：

- `ptr`
- `len`

约束：

- 该结构用于 `_json` 接口返回值、callback 返回值与后续扩展接口
- 该结构也已经用于标准结构体结果中的大量单值文本字段
- 释放必须走 `vulcan_luaskills_ffi_buffer_free`
- 如果 `len > 0`，则 `ptr` 不得为 null

### 8.3 `FfiLuaRuntimeHostOptions`

作用：

- 描述宿主运行时路径、依赖目录名、下载策略、基础库路径等

关键字段：

- 路径字段
- `protected_skill_ids`
- `allow_network_download`
- GitHub base URL
- SQLite / LanceDB 动态库路径
- `sqlite_provider_mode`
- `sqlite_callback_mode`
- `lancedb_provider_mode`
- `lancedb_callback_mode`
- `reserved_entry_names`
- `ignored_skill_ids`
- `enable_skill_management_bridge`

数据库后端模式规则如下：

- `dynamic_library`
  - lib 自己加载数据库动态库
- `host_callback`
  - lib 把数据库请求回调给宿主
- `space_controller`
  - lib 把数据库请求转发给外部空间控制器

当 provider mode 为 `host_callback` 时，还必须继续指定 callback mode：

- `standard`
- `json`

也就是说：

- provider mode 决定数据库请求走哪一类后端
- callback mode 只在 `host_callback` 模式下决定“用标准回调还是 JSON 回调”
- SQLite 与 LanceDB 可以分别设置成不同组合

说明：

- `enable_skill_management_bridge = false`
  - Lua 侧仍可看到 `vulcan.runtime.skills` 命名空间，但安装、更新、启停、卸载桥接会被宿主策略直接拒绝
- `enable_skill_management_bridge = true`
  - 只表示宿主允许 Lua 使用这组桥接能力
  - 真正执行仍依赖宿主已注册运行时技能管理回调
- 如果宿主打开了开关但没有注册回调，Lua 会得到明确错误：
  - `Runtime skill management bridge is enabled but no host callback is registered`

宿主强制忽略规则：

- `ignored_skill_ids` 匹配 skill 目录派生出的 `skill_id`
- 命中后该 skill 会在加载早期被跳过
- 被跳过的 skill 不会触发依赖准备、SQLite/LanceDB 绑定或 entry 注册
- 该字段适合宿主已经用原生、gRPC、VMM 或其他实现替代某个默认 skill 包时使用
- 这不是 skill 自声明的 capability 判定，也不会自动推断宿主已有能力

### 8.4 `FfiLuaEngineOptions`

作用：

- 引擎创建的总配置

组成：

- `pool`
- `host`

### 8.5 `FfiRuntimeSkillRoot`

作用：

- 描述一个命名 skill 根

字段：

- `name`
- `skills_dir`

### 8.6 `FfiLuaInvocationContext`

作用：

- 描述一次调用时的宿主附加上下文

字段：

- `request_context_json`
- `client_budget_json`
- `tool_config_json`

说明：

这些字段在标准 C ABI 接口里仍承载 JSON 内容，但当前已经统一改成：

- `FfiBorrowedBuffer`

也就是说：

- 它们仍然是动态 JSON 结构
- 但不再依赖 NUL 终止字符串
- 宿主应显式提供 `ptr + len`

原因是这些值本身是动态结构，固定 ABI 代价高且易碎。

### 8.7 `FfiSkillInstallRequest`

作用：

- 描述一次受管安装或更新请求

字段：

- `skill_id`
- `source`
- `source_type`

说明：

当前受管安装主链重点支持 GitHub。

`source_type` 在标准 C ABI 中采用稳定整数协议：

- `-1`
  - absent
  - 仅用于结果里表示来源不存在
- `0`
  - github
  - 表示 GitHub 仓库来源
- `1`
  - url
  - 表示 URL metadata 来源

头文件中对应常量为：

- `FFI_SOURCE_TYPE_ABSENT`
- `FFI_SOURCE_TYPE_GITHUB`
- `FFI_SOURCE_TYPE_URL`

### 8.8 `FfiSkillUninstallOptions`

作用：

- 描述一次卸载时是否删除数据库

字段：

- `remove_sqlite`
- `remove_lancedb`

## 9. 接口目录总览

### 9.1 基础接口

标准 C ABI 接口：

- `vulcan_luaskills_ffi_version`
- `vulcan_luaskills_ffi_describe`
- `vulcan_luaskills_ffi_engine_new`
- `vulcan_luaskills_ffi_engine_free`

公共 `_json` FFI 接口：

- `vulcan_luaskills_ffi_version_json`
- `vulcan_luaskills_ffi_describe_json`
- `vulcan_luaskills_ffi_engine_new_json`
- `vulcan_luaskills_ffi_engine_free_json`

### 9.2 加载与重载接口

标准 C ABI 接口：

- `vulcan_luaskills_ffi_load_from_dirs`
- `vulcan_luaskills_ffi_load_from_roots`
- `vulcan_luaskills_ffi_reload_from_dirs`
- `vulcan_luaskills_ffi_reload_from_roots`

公共 `_json` FFI 接口：

- `vulcan_luaskills_ffi_load_from_dirs_json`
- `vulcan_luaskills_ffi_load_from_roots_json`
- `vulcan_luaskills_ffi_reload_from_dirs_json`
- `vulcan_luaskills_ffi_reload_from_roots_json`

### 9.3 描述与帮助接口

标准 C ABI 接口：

- `vulcan_luaskills_ffi_list_entries`
- `vulcan_luaskills_ffi_list_skill_help`
- `vulcan_luaskills_ffi_render_skill_help_detail`
- `vulcan_luaskills_ffi_prompt_argument_completions`
- `vulcan_luaskills_ffi_is_skill`
- `vulcan_luaskills_ffi_skill_name_for_tool`

公共 `_json` FFI 接口：

- `vulcan_luaskills_ffi_list_entries_json`
- `vulcan_luaskills_ffi_list_skill_help_json`
- `vulcan_luaskills_ffi_render_skill_help_detail_json`
- `vulcan_luaskills_ffi_prompt_argument_completions_json`
- `vulcan_luaskills_ffi_is_skill_json`
- `vulcan_luaskills_ffi_skill_name_for_tool_json`

### 9.4 调用接口

标准 C ABI 接口：

- `vulcan_luaskills_ffi_call_skill`
- `vulcan_luaskills_ffi_run_lua`

公共 `_json` FFI 接口：

- `vulcan_luaskills_ffi_call_skill_json`
- `vulcan_luaskills_ffi_run_lua_json`

### 9.5 生命周期接口

标准 C ABI 接口：

- `vulcan_luaskills_ffi_disable_skill_in_dirs`
- `vulcan_luaskills_ffi_disable_skill`
- `vulcan_luaskills_ffi_system_disable_skill_in_dirs`
- `vulcan_luaskills_ffi_system_disable_skill`
- `vulcan_luaskills_ffi_enable_skill`
- `vulcan_luaskills_ffi_system_enable_skill`
- `vulcan_luaskills_ffi_uninstall_skill`
- `vulcan_luaskills_ffi_system_uninstall_skill`
- `vulcan_luaskills_ffi_install_skill`
- `vulcan_luaskills_ffi_system_install_skill`
- `vulcan_luaskills_ffi_update_skill`
- `vulcan_luaskills_ffi_system_update_skill`

公共 `_json` FFI 接口：

- `vulcan_luaskills_ffi_disable_skill_in_dirs_json`
- `vulcan_luaskills_ffi_disable_skill_json`
- `vulcan_luaskills_ffi_system_disable_skill_in_dirs_json`
- `vulcan_luaskills_ffi_system_disable_skill_json`
- `vulcan_luaskills_ffi_enable_skill_json`
- `vulcan_luaskills_ffi_system_enable_skill_json`
- `vulcan_luaskills_ffi_uninstall_skill_json`
- `vulcan_luaskills_ffi_system_uninstall_skill_json`
- `vulcan_luaskills_ffi_install_skill_json`
- `vulcan_luaskills_ffi_system_install_skill_json`
- `vulcan_luaskills_ffi_update_skill_json`
- `vulcan_luaskills_ffi_system_update_skill_json`

## 10. 每类接口的调用逻辑

### 10.1 `version` / `describe`

作用：

- 查询 FFI 版本
- 查询当前导出的入口名字

启动条件：

- 无需创建引擎

调用方式：

- 直接调用即可

调用逻辑：

- 不访问 skill roots
- 不访问运行时注册表
- 不需要任何宿主状态

### 10.2 `engine_new`

作用：

- 创建引擎与内部句柄

参数：

- `LuaEngineOptions`

返回：

- `engine_id`

调用逻辑：

1. 校验配置
2. 构建 `LuaEngine`
3. 放入内部全局注册表
4. 返回稳定句柄

### 10.3 `engine_free`

作用：

- 释放一个引擎句柄

调用逻辑：

1. 从注册表删除句柄
2. 丢弃引擎实例

### 10.4 `load_from_dirs` / `load_from_roots`

作用：

- 扫描技能目录
- 构建 entry 注册表
- 构建 help 树
- 解析 skill manifest
- 注入 provider 绑定

差异：

- `dirs` 是旧目录风格
- `roots` 是当前正式模型

推荐：

- 优先使用 `roots`

### 10.5 `reload_*`

作用：

- 重新扫描技能根并重建运行时视图

调用逻辑：

1. 丢弃旧注册表快照
2. 重新扫描生效 skill
3. 重建 entry / help / provider 绑定

说明：

生命周期接口最终都会依赖 reload 来确认新状态生效。

### 10.6 `list_entries`

作用：

- 列出当前运行时全部工具入口描述

返回内容：

- canonical 名
- 所属 skill
- local name
- root name
- skill_dir
- description
- parameters

### 10.7 `list_skill_help`

作用：

- 列出每个 skill 的 help 树节点描述

返回内容：

- skill id
- skill 版本
- root 名
- skill 目录
- help 节点列表

### 10.8 `render_skill_help_detail`

作用：

- 渲染某个 skill 某个 help 流程节点的详情

参数：

- `skill_id`
- `flow_name`
- 可选请求上下文

说明：

- 标准 C ABI 接口中的请求上下文当前通过 `FfiBorrowedBuffer request_context_json` 传入
- 传空缓冲表示“不附带请求上下文”

### 10.9 `prompt_argument_completions`

作用：

- 取 prompt 参数补全候选

调用条件：

- 目标 prompt 已存在
- 引擎已经 load

### 10.10 `is_skill`

作用：

- 判断一个 canonical tool name 是否属于 Lua skill

### 10.11 `skill_name_for_tool`

作用：

- 解析一个 canonical tool name 所属的 skill id

### 10.12 `call_skill`

作用：

- 调用一个已加载的 skill entry

说明：

- `args_json` 当前通过 `FfiBorrowedBuffer` 传入
- `invocation_context` 中的三个 JSON 字段也都通过 `FfiBorrowedBuffer` 传入
- 这两部分仍承载 JSON 内容，但标准 ABI 不再要求宿主提供 NUL 终止字符串

参数：

- `tool_name`
- `args`
- 可选调用上下文

调用逻辑：

1. 查 entry 注册表
2. 定位所属 skill
3. 构造 `vulcan.context.*`
4. 注入 `vulcan.deps.*`
5. 执行 Lua
6. 结构化返回结果

### 10.13.1 `vulcan.runtime.skills.*`

作用：

- 允许宿主把安装、更新、启停、卸载桥接为 Lua 可调用能力

当前公开方法：

- `vulcan.runtime.skills.status()`
- `vulcan.runtime.skills.install(input)`
- `vulcan.runtime.skills.update(input)`
- `vulcan.runtime.skills.uninstall(input)`
- `vulcan.runtime.skills.enable(input)`
- `vulcan.runtime.skills.disable(input)`

调用逻辑：

1. 先检查宿主能力开关是否允许
2. 再检查宿主是否注册技能管理回调
3. 将 Lua 输入转换为 JSON
4. 通过宿主回调转发结构化管理请求
5. 将宿主回调结果再转换回 Lua

设计意图：

- skill 不直接操控底层安装器
- 最终是否允许执行，由宿主策略决定
- 适合拥有自己 TUI、GUI 或专用管理界面的宿主

### 10.13 `run_lua`

作用：

- 执行一段任意 Lua 代码

适合：

- 调试
- 宿主 smoke test
- 系统能力验证

说明：

- `code` 仍然是普通 UTF-8 字符串
- `args_json` 当前通过 `FfiBorrowedBuffer` 传入
- 返回值 `result_json_out` 继续通过 `FfiOwnedBuffer` 返回

## 11. 生命周期链路处理逻辑

### 11.1 disable

作用：

- 写停用标记
- reload

当前语义：

- 失败会回滚
- system 版本允许操作受保护 skill

### 11.2 enable

作用：

- 删除停用标记
- reload

### 11.3 install

作用：

- 下载并安装一个受管 skill

当前主链：

1. 解析来源
2. 下载 release 资产
3. 校验 `checksums.txt`
4. 解包到 staging
5. 校验 manifest
6. 准备暂存目录
7. reload
8. 成功后提交 install record
9. 失败则回滚

当前特点：

- 已做事务化
- staging 失败会自动清理
- checksum 失败会自愈重下一次

### 11.4 update

作用：

- 基于来源记录更新已安装 skill

当前主链：

1. 读取 install record
2. 查询来源最新版本
3. 下载并校验新包
4. 解包到 staging
5. 备份旧 skill
6. 放置新版本
7. reload
8. 提交 install record
9. 删除旧 backup
10. 差分清理旧依赖

事务语义：

- reload 失败会回滚到旧版本
- install record 提交失败也会回滚
- 旧依赖清理失败只产生 warning，不会把更新误报成失败

### 11.5 uninstall

作用：

- 卸载 skill

当前主链：

1. 暂存要删除的 skill 目录
2. reload
3. 成功后正式提交卸载
4. 清理 install record / disabled 标记
5. 可选删除数据库
6. 清理 skill 私有依赖

事务语义：

- reload 失败会回滚 skill 目录与 install record
- 数据库清理失败只记 warning，不会把已成功卸载误报成失败

## 12. install / update 的来源模型

当前来源记录不属于 `skill.yaml`，而属于安装状态。

受管安装成功后，会写入：

- `state/installs/<skill_id>.yaml`

当前 GitHub 模型的关键字段是：

- source type
- repo locator
- tag
- version

只有通过 install 安装的 skill 才有来源记录。  
手工复制进来的 skill 不参与自动更新。

## 13. skill roots 与空间模型

FFI 宿主接入时，推荐优先使用 `RuntimeSkillRoot[]`。

每个根包含：

- `name`
- `skills_dir`

要求：

- name 唯一
- 物理路径唯一
- 空间父目录不能冲突
- `skills_dir` 必须是目录

当前根链是有序覆盖链：

- 前面的优先级更高

## 14. 标准 C ABI 与公共 `_json` FFI 的选择建议

### 14.1 优先使用标准 C ABI 的情况

- Go
- C#
- 高性能宿主
- 想减少 JSON 编解码
- 想显式控制内存释放

### 14.2 优先使用公共 `_json` FFI 的情况

- Python
- TypeScript / Node.js
- 调试工具
- 快速接入
- 动态值很多的场景

### 14.3 混合使用策略

完全可以采用：

- 引擎创建 / load / list / 生命周期操作：标准 C ABI
- 动态调用 / 调试 / 跨语言壳层：公共 `_json` FFI

当前协议允许这种混合使用。

### 14.4 一页式选型结论

如果宿主正在做第一次技术选型，可以直接按下面的结论判断：

- Rust 宿主：
  - 优先直接引用 Rust API
  - 不建议额外包装成 FFI 再回调自己
- C / C++ / Go / 能稳定处理结构体与 out 指针的宿主：
  - 优先选择标准 C ABI
  - 适合把它当成正式低层契约长期维护
- Python / Node.js / TypeScript / 动态脚本宿主：
  - 优先选择公共 `_json` FFI
  - 适合快速接入、原型验证和减少 ABI 绑定成本
- 一个宿主同时需要“稳定主链”和“动态扩展链”时：
  - 可以混合使用
  - 标准 C ABI 承载 `engine/load/list/call/lifecycle`
  - 公共 `_json` FFI 承载动态安装、调试和宿主快速桥接

一句话总结：

- 标准 C ABI 解决“正式低层接入”
- 公共 `_json` FFI 解决“快速跨语言接入”
- Rust API 解决“同语言直接集成”

## 15. C / Python / Go / TypeScript 示例说明

示例位置：

- C：
  - [examples/ffi/c/demo.c](../examples/ffi/c/demo.c)
- Python：
  - [examples/ffi/python/demo.py](../examples/ffi/python/demo.py)
  - [examples/ffi/python/lifecycle_demo.py](../examples/ffi/python/lifecycle_demo.py)
  - [examples/ffi/python/query_demo.py](../examples/ffi/python/query_demo.py)
- Go：
  - [examples/ffi/go/demo.go](../examples/ffi/go/demo.go)
  - [examples/ffi/go/lifecycle_demo/main.go](../examples/ffi/go/lifecycle_demo/main.go)
  - [examples/ffi/go/query_demo/main.go](../examples/ffi/go/query_demo/main.go)
- TypeScript：
  - [examples/ffi/typescript/demo.ts](../examples/ffi/typescript/demo.ts)
  - [examples/ffi/typescript/lifecycle_demo.ts](../examples/ffi/typescript/lifecycle_demo.ts)
  - [examples/ffi/typescript/query_demo.ts](../examples/ffi/typescript/query_demo.ts)

当前示例主要演示：

- 查询版本
- 创建引擎
- 加载根链
- 读取结构化 `list_entries` 结果
- 通过 `FfiBorrowedBuffer` 调用标准 `call_skill`
- 读取结构化调用结果
- 通过 `FfiBorrowedBuffer` 调用标准 `run_lua`
- 读取 JSON 结果缓冲
- 释放引擎

其中：

- `c/demo.c` 更贴近底层标准 C ABI 契约
- Python / Go / TypeScript 示例更适合展示各语言桥接方式
- `python/lifecycle_demo.py` 额外聚焦标准 ABI 的 `disable_skill / enable_skill` 生命周期切换
- `python/query_demo.py` 额外聚焦标准 ABI 的查询辅助接口
- `go/lifecycle_demo/main.go` 额外聚焦标准 ABI 的 `disable_skill / enable_skill` 生命周期切换
- `go/query_demo/main.go` 额外聚焦标准 ABI 的查询辅助接口
- `typescript/lifecycle_demo.ts` 额外聚焦标准 ABI 的 `disable_skill / enable_skill` 生命周期切换
- `typescript/query_demo.ts` 额外聚焦标准 ABI 的查询辅助接口

这是一组覆盖“版本 -> 引擎 -> 加载 -> 结构化枚举 -> 标准调用 -> 标准 Lua 执行”的最小 smoke test。  
后续如果宿主要做完整接入，建议直接按本文档把：

- install / update / uninstall

接进去。

另外还提供一个可直接运行的完整烟测目录：

- [examples/ffi/standard_runtime/README.md](../examples/ffi/standard_runtime/README.md)
- [examples/ffi/demo_runtime/README.md](../examples/ffi/demo_runtime/README.md)

其中：

- `standard_runtime`
  - 提供标准 ABI 示例共用的最小 skill 夹具
  - 默认包含 `demo-standard-ffi-skill-ping`，可直接演示 `call_skill`
  - 同时提供稳定 runtime 目录布局，便于标准示例继续演示 `run_lua`
  - 也可直接配合 `python/lifecycle_demo.py` 演示 `disable_skill / enable_skill`
- `demo_runtime`
  - 提供动态安装与调用烟测链

`demo_runtime` 会：

- 使用仓库内空 runtime root
- 动态安装 `OpenVulcan/luaskills-demo-skill`
- 调用 `luaskills-demo-skill-demo-status`
- 输出 success

### 15.1 示例选型速查

如果宿主只想先抓一条最短路径，请按目标直接看对应示例：

- 想先理解标准 ABI 的主调用链：
  - [examples/ffi/c/demo.c](../examples/ffi/c/demo.c)
  - [examples/ffi/python/demo.py](../examples/ffi/python/demo.py)
  - [examples/ffi/go/demo.go](../examples/ffi/go/demo.go)
  - [examples/ffi/typescript/demo.ts](../examples/ffi/typescript/demo.ts)
- 想看 `disable_skill / enable_skill`：
  - [examples/ffi/python/lifecycle_demo.py](../examples/ffi/python/lifecycle_demo.py)
  - [examples/ffi/go/lifecycle_demo/main.go](../examples/ffi/go/lifecycle_demo/main.go)
  - [examples/ffi/typescript/lifecycle_demo.ts](../examples/ffi/typescript/lifecycle_demo.ts)
- 想看 `is_skill / skill_name_for_tool / prompt_argument_completions`：
  - [examples/ffi/python/query_demo.py](../examples/ffi/python/query_demo.py)
  - [examples/ffi/go/query_demo/main.go](../examples/ffi/go/query_demo/main.go)
  - [examples/ffi/typescript/query_demo.ts](../examples/ffi/typescript/query_demo.ts)
- 想看标准 ABI 示例共用的最小 skill 夹具：
  - [examples/ffi/standard_runtime/README.md](../examples/ffi/standard_runtime/README.md)
- 想看公共 `_json` FFI 驱动的动态安装烟测：
  - [examples/ffi/demo_runtime/README.md](../examples/ffi/demo_runtime/README.md)
- 想看宿主接管 SQLite / LanceDB provider：
  - [examples/ffi/host_provider_demo/README.md](../examples/ffi/host_provider_demo/README.md)

建议阅读顺序：

1. 先从标准 ABI 的 `demo` 看最短闭环。
2. 再根据需要进入 `lifecycle_demo` 或 `query_demo`。
3. 最后再看 `demo_runtime` 和 `host_provider_demo` 这类更接近宿主集成的扩展场景。

## 16. 推荐接入顺序

推荐宿主集成顺序：

1. 接入 `version`
2. 接入 `engine_new` / `engine_free`
3. 接入 `load_from_roots`
4. 接入 `list_entries`
5. 接入 `call_skill`
6. 接入 `list_skill_help` / `render_skill_help_detail`
7. 接入生命周期接口

这样最稳。

## 17. 常见失败场景

### 17.1 `engine_new` 失败

常见原因：

- 目录配置无效
- 必填 sibling 目录名为空
- 宿主配置不合法

### 17.2 `load` 失败

常见原因：

- `skills_dir` 不是目录
- `skill.yaml` 不合法
- 入口脚本缺失
- help 树解析失败

### 17.3 `call_skill` 失败

常见原因：

- tool name 不存在
- 参数不合法
- Lua 运行时错误
- provider binding 错误

### 17.4 `install / update` 失败

常见原因：

- 网络错误
- release 资产缺失
- checksum 校验失败
- manifest 与来源不一致
- reload 失败触发事务回滚

## 18. 当前约束

### 18.1 标准 ABI 里仍保留 JSON 字段

原因不是偷懒，而是这几类值本来就是动态结构：

- request context
- budget snapshot
- tool config
- arbitrary Lua args
- arbitrary Lua return values

对这类值强行做固定 C ABI，收益很低，破坏性很高。

### 18.2 `_json` 是正式公共接口，不是临时调试接口

`_json` 不是次级能力，而是正式的高层公共接口。  
只是它更偏易用性与跨语言接入，不偏极致性能。

## 19. 对接建议结论

如果宿主：

- 有稳定 FFI 封装能力
- 关注性能
- 希望减少 JSON 编解码

则优先用标准 C ABI。

如果宿主：

- 需要快速接入
- 语言本身更偏动态
- 希望先把链路打通

则优先用公共 `_json` FFI。

最推荐的工程实践是：

- 用标准 C ABI 承载底层稳定主链
- 用公共 `_json` FFI 承载动态语言接入、动态扩展与调试能力

这也是当前 `vulcan-luaskills` FFI 设计的核心目的。

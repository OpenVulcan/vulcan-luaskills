# 宿主数据库 Provider 对接说明

## 1. 文档目标

本文档用于说明 `vulcan-luaskills` 当前如何把 SQLite / LanceDB 数据库操作交给宿主自己处理。

本文档重点覆盖：

- 数据库 provider 模式分类
- 回调模式分类
- `space_label` / `binding_tag` 的语义
- 宿主需要实现的最小必要接口
- 独立 demo 的使用方式

## 2. 当前支持的数据库接入模式

当前每个数据库后端都支持三种 provider mode：

- `dynamic_library`
- `host_callback`
- `space_controller`

它们分别通过下列字段控制：

- `LuaRuntimeHostOptions.sqlite_provider_mode`
- `LuaRuntimeHostOptions.lancedb_provider_mode`
- `LuaRuntimeHostOptions.sqlite_callback_mode`
- `LuaRuntimeHostOptions.lancedb_callback_mode`

这些配置是**按后端分别设置**的，也就是说 SQLite 与 LanceDB 可以使用不同模式。

例如：

- SQLite 使用 `host_callback + json`
- LanceDB 使用 `dynamic_library`

### `dynamic_library`

由 `luaskills` 自己加载本地动态库，并直接调用：

- SQLite backend
- LanceDB backend

这种模式下：

- lib 自己计算默认数据库路径
- lib 自己打开数据库
- skill 只做表操作与向量操作

### `host_callback`

由宿主注册数据库 provider 回调。

这种模式下：

- lib 不再决定数据库最终创建位置
- lib 不再要求数据库一定落在默认 sibling 目录下
- lib 只负责把数据库请求和稳定绑定上下文回调给宿主
- 宿主决定：
  - 数据库放在哪
  - 是否共享
  - 是否走本地文件
  - 是否转发给别的服务

### `space_controller`

由 lib 把数据库请求转发给外部空间控制器。

这种模式下：

- lib 不直接打开数据库
- lib 不依赖宿主本进程内的回调实现
- 数据库 ownership 与共享策略由控制器负责
- 代码层固定依赖 `vldb-controller-client`
- controller 服务程序由宿主自行复制到本地稳定路径，再通过 `LuaRuntimeHostOptions.space_controller.executable_path` 提供给 lib
- 如果宿主没有显式指定 `endpoint`，运行时默认使用共享端点 `http://127.0.0.1:19801`
- 如果宿主指定了不同端点，则可切换到独立 controller 实例

推荐理解方式：

- Rust SDK：走 `git + rev`
- controller 可执行程序：走宿主本地复制与管理
- 共享还是独占：由 `endpoint` 决定

## 3. 为什么要引入宿主 callback 模式

因为记忆型 skill 使用的：

- SQLite
- LanceDB
- BM25 / FTS

本质上都带有状态和共享语义。

如果多个宿主进程都直接打开同一个物理数据库，就会出现：

- SQLite 锁冲突
- LanceDB 并发写入风险
- 多进程下的共享与 ownership 混乱

宿主 `host_callback` 模式的目标就是：

**让 lib 保持统一的数据库调用接口，同时把真正的数据库 ownership 与数据库目录分配交给宿主。**

## 4. 回调模式分类

当 provider mode 选择 `host_callback` 时，还必须继续指定 callback mode：

- `standard`
- `json`

它们分别通过下列字段控制：

- `LuaRuntimeHostOptions.sqlite_callback_mode`
- `LuaRuntimeHostOptions.lancedb_callback_mode`

规则如下：

- provider mode 不等于 callback mode
- 只有 provider mode 为 `host_callback` 时，callback mode 才会参与分发
- lib 不再根据“是否注册了某种回调”来推断当前模式
- lib 始终先看宿主初始化时传入的 mode，再决定走哪条链路

### `standard`

- 走结构化 C ABI 回调
- 适合 C / C++ / Go / Rust 等更偏稳定 ABI 的宿主

### `json`

- 走 JSON 请求 / JSON 响应回调
- 适合 Python / Node / 快速原型验证

## 5. 绑定上下文

宿主 provider 回调不会只拿到一段 SQL 或一段向量请求。

lib 会一并提供稳定绑定上下文：

- `space_label`
- `skill_id`
- `binding_tag`
- `root_name`
- `space_root`
- `skill_dir`
- `skill_dir_name`
- `database_kind`
- `default_database_path`

其中最关键的是：

### `space_label`

由宿主提供的稳定空间标签，例如：

- `ROOT`
- `USER`
- `PROJECT_A`

对于项目级接管，**宿主必须保证这个标签稳定**。  
这是前置条件，不由 lib 自动推导。

### `binding_tag`

由 lib 统一组合：

```text
{space_label}-{skill_id}
```

例如：

- `ROOT-vulcan-work-memory`
- `PROJECT_A-vulcan-ai-memory`

这个字段用于：

- 宿主管理数据库命名空间
- 共享数据库标签
- 多进程稳定复用

### `default_database_path`

这是 lib 按内嵌旧规则推导出的默认数据库路径。

它的作用只是：

- 兼容
- 诊断
- 给宿主做参考

它**不是**宿主必须使用的真实数据库路径。

## 6. 标准回调模式

### SQLite

注册函数：

- `vulcan_luaskills_ffi_set_sqlite_provider_callback`

回调签名：

- 输入：
  - `const FfiSqliteProviderRequest *request`
  - `void *user_data`
- 输出：
  - `char **response_json_out`
  - `char **error_out`

### LanceDB

注册函数：

- `vulcan_luaskills_ffi_set_lancedb_provider_callback`

回调签名：

- 输入：
  - `const FfiLanceDbProviderRequest *request`
  - `void *user_data`
- 输出：
  - `char **meta_json_out`
  - `uint8_t **data_out`
  - `size_t *data_len_out`
  - `char **error_out`

### 标准回调的关键点

标准模式下，请求最外层是固定 C 结构：

- `action`
- `binding`
- `input_json`

也就是说：

- 外层协议是结构化的
- 动作内部参数仍然通过 `input_json` 承载

这样可以同时满足两件事：

1. 宿主拿到的是标准 ABI 请求，而不是一整坨 JSON
2. 内层动态参数不用被硬编码成大量脆弱 C 结构

### 宿主如何返回字符串和字节

标准回调下，如果宿主需要返回：

- `response_json_out`
- `meta_json_out`
- `data_out`

应优先使用 `luaskills` 提供的辅助函数：

- `vulcan_luaskills_ffi_string_clone`
- `vulcan_luaskills_ffi_bytes_clone`

对应释放由 `luaskills` 内部自动完成。

## 7. JSON 回调模式

### SQLite

注册函数：

- `vulcan_luaskills_ffi_set_sqlite_provider_json_callback`

### LanceDB

注册函数：

- `vulcan_luaskills_ffi_set_lancedb_provider_json_callback`

### JSON 回调规则

JSON 回调模式下，宿主收到的是一整份 JSON 请求字符串。

例如 SQLite 请求会是：

```json
{
  "action": "query_json",
  "binding": {
    "space_label": "ROOT",
    "skill_id": "host-provider-sqlite-demo",
    "binding_tag": "ROOT-host-provider-sqlite-demo",
    "root_name": "ROOT",
    "space_root": "D:/.../runtime_root/skills",
    "skill_dir": "D:/.../runtime_root/skills/host-provider-sqlite-demo",
    "skill_dir_name": "host-provider-sqlite-demo",
    "database_kind": "sqlite",
    "default_database_path": "D:/.../databases/sqlite/host-provider-sqlite-demo"
  },
  "input": {
    "sql": "SELECT 1"
  }
}
```

宿主返回的也是 JSON 字符串。

所以：

- JSON 模式对动态语言最友好
- Python / Node / 轻量桥接代码最适合先走这条路径

## 8. 必要接口要求

当宿主选择 `host_callback` 模式时，必须实现对应数据库所需的必要动作。

### SQLite 当前动作集合

- `execute_script`
- `execute_batch`
- `query_json`
- `query_stream`
- `query_stream_wait_metrics`
- `query_stream_chunk`
- `query_stream_close`
- `tokenize_text`
- `upsert_custom_word`
- `remove_custom_word`
- `list_custom_words`
- `ensure_fts_index`
- `rebuild_fts_index`
- `upsert_fts_document`
- `delete_fts_document`
- `search_fts`

### LanceDB 当前动作集合

- `create_table`
- `vector_upsert`
- `vector_search`
- `delete`
- `drop_table`

如果宿主启用了某个数据库的 `host_callback` 模式，却没有注册对应回调，运行时会直接报错，不会静默回退。

## 9. 宿主应该如何决定数据库落点

宿主不应该再直接信任：

- lib 默认数据库路径

而应该根据：

- `binding_tag`
- `space_label`
- `database_kind`

自己决定真实数据库位置。

例如宿主可以把：

```text
ROOT-vulcan-work-memory
```

映射到：

```text
C:/HostManagedData/sqlite/ROOT-vulcan-work-memory.db
```

或者：

```text
D:/project-a/.host-dbs/sqlite/PROJECT_A-vulcan-ai-memory.db
```

或者：

- 一个统一数据库服务中的租户命名空间

## 10. 独立 demo

当前仓库已经提供一个独立演示：

- [../examples/ffi/host_provider_demo/README.md](../examples/ffi/host_provider_demo/README.md)

这个 demo 的特点是：

- skill 依旧通过 `vulcan.sqlite.*` 调数据库
- `luaskills` 切到 `host_callback` 模式
- Python 宿主注册 JSON callback
- 宿主把 SQLite 请求转发给 `vldb-sqlite`
- 宿主最终数据库落点不使用 `default_database_path`
- 而是使用：

```text
runtime_root/host_managed/sqlite/<binding_tag>.db
```

这正好用来验证：

- 宿主接管数据库
- lib 只提供稳定绑定标签
- 真实数据库目录由宿主自己决定

## 11. 当前最推荐的落地顺序

如果宿主是动态语言或先做原型：

1. 先用 JSON callback
2. 先只接 SQLite
3. 先验证 `binding_tag` 路由
4. 再补 LanceDB

如果宿主是 Go / C / Rust / 高性能场景：

1. 用标准 callback
2. 外层请求走结构化 ABI
3. 内层动作参数仍走 `input_json`
4. 用 `string_clone / bytes_clone` 返回拥有型结果

## 12. 一句话总结

当前 `host_callback` 模式的核心原则是：

**lib 保持统一数据库接口与稳定绑定上下文，宿主负责决定数据库真实落点、真实 ownership 与真实执行后端。**

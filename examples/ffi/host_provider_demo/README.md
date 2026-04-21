# Host Callback Demo

这个目录演示如何把 `vulcan-luaskills` 作为 FFI 库接入，同时由宿主自己接管 SQLite 数据库落点与执行。

当前 demo 重点验证三件事：

1. 宿主通过 FFI 注册 SQLite JSON provider 回调
2. `luaskills` 在 `host_callback` 模式下把数据库请求回调给宿主
3. 宿主不复用 lib 默认数据库路径，而是基于稳定 `binding_tag` 自己决定数据库文件位置

## 当前示例结构

- `run_python_host_provider_demo.py`
  - Python 宿主示例
  - 使用 `ctypes` 加载 `vulcan-luaskills` 与 `vldb-sqlite`
  - 注册 SQLite JSON provider 回调
- `runtime_root/`
  - 独立演示运行时目录
  - 内置一个最小 skill
- `backends/`
  - 可选的本地后端动态库存放目录
- `scripts/copy_local_backends.ps1`
  - 从本机工作区复制 `vldb-sqlite` / `vldb-lancedb` 动态库到 `backends/`

## 本地工作区默认后端仓库位置

当前脚本默认会优先尝试这些本地工作区路径：

- `D:\projects\VulcanLocalDataGateway\vldb-sqlite`
- `D:\projects\VulcanLocalDataGateway\vldb-lancedb`

如果您的本地仓库位置不同，可以直接：

- 设置 `VLDB_SQLITE_LIB`
- 设置 `VLDB_LANCEDB_LIB`

或者自行修改脚本。

## 快速准备

### 1. 构建 `vulcan-luaskills`

在仓库根目录执行：

```powershell
cargo build
```

### 2. 可选复制本地后端动态库

如果您本机已经构建过 `vldb-sqlite` 或 `vldb-lancedb`，可以执行：

```powershell
.\scripts\copy_local_backends.ps1
```

### 3. 运行 Python 宿主演示

```powershell
$env:VULCAN_LUASKILLS_LIB = "D:\projects\vulcan-luaskills\target\debug\vulcan_luaskills.dll"
python .\run_python_host_provider_demo.py
```

如果 `vldb-sqlite` 不在默认位置，还可以显式传入：

```powershell
$env:VLDB_SQLITE_LIB = "D:\projects\VulcanLocalDataGateway\vldb-sqlite\target\debug\vldb_sqlite.dll"
python .\run_python_host_provider_demo.py
```

## 运行结果

成功时脚本会：

1. 创建 FFI 引擎
2. 以 `host_callback` 模式加载 demo skill
3. 由宿主回调执行 SQLite 建表与查询
4. 返回一个包含 `success` 的结果

宿主实际数据库文件会落在：

```text
runtime_root/host_managed/sqlite/<binding_tag>.db
```

例如：

```text
runtime_root/host_managed/sqlite/ROOT-host-provider-sqlite-demo.db
```

这说明：

- lib 负责提供稳定 `binding_tag`
- 宿主负责决定数据库真实落点
- 数据库目录不再由 lib 强制决定

## 当前示例为什么先选 SQLite

因为：

- `vldb-sqlite` 当前 JSON FFI 已经覆盖通用 SQL / FTS / BM25 核心路径
- 更适合先验证宿主 provider 接管边界

后续如果切到 `vldb-lancedb`，只需要换掉宿主壳层的回调实现，不需要重做 `luaskills` 的 provider 协议。

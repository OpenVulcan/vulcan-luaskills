# luaskills-sdk

Python SDK，用于通过公共 `_json` FFI 集成 LuaSkills 运行时。

SDK 封装了动态库加载、`FfiBorrowedBuffer` / `FfiOwnedBuffer`、JSON 包络、engine 生命周期、root helper、authority、skill-config、普通管理面与 system 管理面。宿主不需要在业务代码中重复手写 ctypes buffer。

## 安装

```bash
pip install luaskills-sdk
```

当前包不内置 `luaskills` 原生动态库。调用时需要通过 `library_path` 或 `LUASKILLS_LIB` 指向动态库：

```powershell
$env:LUASKILLS_LIB = "D:\path\to\luaskills.dll"
```

## 基础用法

```python
from luaskills import Authority, LuaSkillsClient, RuntimeRoots

runtime_root = "D:/runtime/luaskills"
roots = RuntimeRoots.standard(runtime_root)

with LuaSkillsClient(library_path="D:/path/to/luaskills.dll", runtime_root=runtime_root) as client:
    client.load_from_roots(roots)
    entries = client.list_entries(Authority.DELEGATED_TOOL)
    result = client.call_skill("demo-standard-ffi-skill-ping", {"note": "python-sdk"})

    print(entries)
    print(result["content"])
```

## CLI

安装后可使用：

```bash
luaskills install-runtime --database vldb-controller --runtime-root D:\runtime\luaskills
luaskills install-runtime --database vldb-direct --runtime-root D:\runtime\luaskills
luaskills install-runtime --database none --runtime-root D:\runtime\luaskills
luaskills version --lib D:\path\to\luaskills.dll
luaskills list --lib D:\path\to\luaskills.dll --runtime-root D:\runtime\luaskills
luaskills call demo-standard-ffi-skill-ping "{\"note\":\"python\"}" --lib D:\path\to\luaskills.dll
```

`install-runtime` 会按当前平台生成或安装 runtime native 资产，并写入 `resources/luaskills-sdk-runtime-manifest.json`：

- `none`：不安装数据库 provider，只准备 LuaSkills FFI SDK 资产。
- `vldb-controller`：下载 `vldb-controller-{version}-{target}`，用于 `space_controller` provider mode。
- `vldb-direct`：下载 `vldb-sqlite-lib-{version}-{target}` 与 `vldb-lancedb-lib-{version}-{target}`，用于 `dynamic_library` provider mode。
- `host-callback`：不下载 VLDB 资产，生成 `host_callback + json` 的 host option patch。

排查发布资产名时可先使用：

```bash
luaskills install-runtime --database vldb-direct --dry-run
```

安装完成后，`LuaSkillsClient(runtime_root=...)` 会自动从 `runtime_root/libs` 解析 LuaSkills 动态库，并读取该 manifest 把数据库 provider 的 host option patch 合入默认配置；宿主仍可通过显式 `library_path` 与 `host_options` 覆盖。

管理命令示例：

```bash
luaskills install LuaSkills/luaskills-demo-skill --target-root USER
luaskills update LuaSkills/luaskills-demo-skill --target-root USER
luaskills uninstall luaskills-demo-skill --target-root USER
```

system 管理入口必须由宿主决定 authority：

```bash
luaskills system-install LuaSkills/luaskills-demo-skill --target-root ROOT --authority system
```

如果 system 工具封装给普通 tools，应固定使用 `--authority delegated_tool`。

## JSON Provider Callback

SQLite / LanceDB 的 `host_callback + json` 模式可以直接通过 SDK 注册，宿主无需在业务代码中重复手写 ctypes buffer：

```python
from luaskills import LuaSkillsClient, LuaSkillsJsonFfi

ffi = LuaSkillsJsonFfi("D:/path/to/luaskills.dll")


def sqlite_provider(request):
    return {"ok": True, "request": request}


ffi.set_sqlite_provider_json_callback(sqlite_provider)

try:
    client = LuaSkillsClient(
        library_path="D:/path/to/luaskills.dll",
        runtime_root="D:/runtime/luaskills",
        host_options={
            "sqlite_provider_mode": "host_callback",
            "sqlite_callback_mode": "json",
        },
    )
    client.close()
finally:
    ffi.clear_sqlite_provider_json_callback()
```

callback 必须在 `engine_new` 前注册；engine 创建后再切换 callback 不会 retroactive 影响已存在 engine。

pip 安装后的 wheel 内置可运行示例：

```bash
python -m luaskills.examples.basic
python -m luaskills.examples.provider_callback
```

源码仓库中同样保留 `examples/basic.py` 与 `examples/provider_callback.py`，便于直接阅读。

## 权限与边界

- 查询类接口默认使用 `DelegatedTool`，因此不会返回 `ROOT` skills。
- `System` 只表示可管理 ROOT 层，不表示可绕过 ROOT 同名占用规则。
- `call_skill` 与 `run_lua` 是运行时执行面，不作为 ROOT 可见性过滤。
- skill-config 按 `skill_id + key` 读写，只有 Lua skill 实际读取配置时才影响行为。
- 如果宿主不希望用户执行任意 Lua，不应直接暴露 `run_lua`。

## 验证

源码环境可运行：

```bash
python -m compileall sdk/python/src
PYTHONPATH=sdk/python/src python -m luaskills.cli version --lib target/debug/luaskills.dll
```

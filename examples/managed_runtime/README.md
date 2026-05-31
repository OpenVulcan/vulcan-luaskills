# Managed Runtime Example

This directory contains a minimal LuaSkills package that calls managed Python and Node.js handlers from Lua.
本目录包含一个最小 LuaSkills 包，用于从 Lua 调用受管 Python 与 Node.js handler。

## Prepare Runtimes

Run the managed runtime fetch script first:
先运行受管运行时拉取脚本：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/deps/fetch_managed_runtimes.ps1 -RuntimeRoot target/managed-runtime-fetch-check-uv01117-all -Target all -Force
```

## Debug Call

Call the example through the normal `luaskills-debug` path:
通过正式 `luaskills-debug` 路径调用示例：

```powershell
cargo run --bin luaskills-debug -- call --runtime-root target/managed-runtime-fetch-check-uv01117-all --skill-path examples/managed_runtime/managed-child-runtime-debug --tool smoke --args-json '{"text":"debug-call"}' --output content
```

## Isolated Smoke Test

Run the isolated smoke script when you want the test to create its own runtime root and fetch dependencies independently:
当希望测试自行创建隔离运行时根目录并独立拉取依赖时，运行隔离冒烟脚本：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/debug-tools/managed_runtime_smoke.ps1
```

Use `-SkipFetch` only when intentionally reusing an existing runtime root during local iteration:
仅在本地迭代时刻意复用已有运行时根目录时使用 `-SkipFetch`：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/debug-tools/managed_runtime_smoke.ps1 -RuntimeRoot target/managed-runtime-fetch-check-uv01117-all -SkipFetch -KeepRuntimeRoot
```

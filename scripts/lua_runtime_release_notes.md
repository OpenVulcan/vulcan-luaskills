## LuaSkills core release packages

This Release now publishes only the main-repo artifacts that still belong to `luaskills`: the FFI SDK and the runnable demo packages. Lua runtime packages and native dependency bundles are published separately by [`LuaSkills/luaskills-packages`](https://github.com/LuaSkills/luaskills-packages).

### Assets

- `luaskills-ffi-sdk-{platform}.tar.gz`: FFI SDK package for C ABI or dynamic-library host integration. It contains headers under `include/`, luaskills runtime/import libraries under `lib/`, and the project license.
- `luaskills-demo-ffi-{platform}.tar.gz`: Runnable FFI-mode demo package that shows an external host loading luaskills through the dynamic library. It includes the full `examples/ffi/` tree for C, Go, Python, TypeScript, standard runtime, install smoke tests, and host-provider demos, plus platform-matching runner scripts and dependency fetch scripts.
- `luaskills-demo-rust-{platform}.tar.gz`: Runnable non-FFI Rust demo package that shows a Rust host using the `luaskills` crate. It includes platform-matching runner scripts and dependency fetch scripts.

### Runtime dependencies

Demo packages no longer bundle `lua-runtime-{platform}.tar.gz` or `lua-deps-{platform}.tar.gz` from this repository. Instead, their bundled `fetch_runtime_deps.ps1` and `fetch_runtime_deps.sh` scripts download the runtime packages below from `LuaSkills/luaskills-packages`:

- `lua-runtime-packages-{platform}.tar.gz`: Default Lua runtime package layout containing `lua_packages/`, `libs/`, `resources/`, and `licenses/`.
- `lua-deps-{platform}.tar.gz`: Native dependency bundle used by advanced local builds or other package workflows.

### Demo dependency fetch targets

Demo packages provide standalone dependency upgrade scripts with three targets. The `run` script only runs the demo and does not download dependencies automatically. Windows packages include `upgrade_deps.bat`, `scripts/fetch_runtime_deps.ps1`, and `run.ps1`; Linux/macOS packages include `upgrade_deps.sh`, `scripts/fetch_runtime_deps.sh`, and `run.sh`.

- `all`: Fetch `lua-runtime-packages-{platform}.tar.gz`, `luaskills-ffi-sdk-{platform}.tar.gz`, and vldb-controller.
- `lua`: Fetch `lua-runtime-packages-{platform}.tar.gz` plus `luaskills-ffi-sdk-{platform}.tar.gz` and install them into the demo `runtime/` directory.
- `vldb`: Fetch only vldb-controller and place it under the demo runtime `bin/` directory.

In most demo scenarios, run `all` through `upgrade_deps.bat` or `upgrade_deps.sh` first. Use `lua` when you only need to validate Lua package capabilities. Use `vldb` when a runtime already exists and only vldb-controller is missing.

## LuaSkills 主仓库发布资产说明

本 Release 现在只发布仍然属于 `luaskills` 主仓库的核心资产：FFI SDK 与可运行 demo 包。Lua runtime 包和原生依赖包已经拆分到 [`LuaSkills/luaskills-packages`](https://github.com/LuaSkills/luaskills-packages) 独立发布。

### 资产用途

- `luaskills-ffi-sdk-{platform}.tar.gz`：面向 C ABI / 动态库宿主集成的 FFI SDK 包，包含 `include/` 头文件、`lib/` 下的 luaskills 动态库或导入库，以及项目许可证。
- `luaskills-demo-ffi-{platform}.tar.gz`：面向 FFI 模式的可运行 demo 包，演示外部宿主通过动态库加载 luaskills，并携带 `examples/ffi/` 下完整 C、Go、Python、TypeScript、标准 runtime、安装烟测和宿主 provider 示例，以及平台匹配的运行脚本与依赖拉取脚本。
- `luaskills-demo-rust-{platform}.tar.gz`：面向非 FFI / Rust 直连模式的可运行 demo 包，演示 Rust 宿主通过 `luaskills` crate 使用运行时，并携带平台匹配的运行脚本与依赖拉取脚本。

### Runtime 依赖来源

demo 包不再从本仓库发布 `lua-runtime-{platform}.tar.gz` 或 `lua-deps-{platform}.tar.gz`。取而代之，包内自带的 `fetch_runtime_deps.ps1` 与 `fetch_runtime_deps.sh` 会从 `LuaSkills/luaskills-packages` 下载以下资产：

- `lua-runtime-packages-{platform}.tar.gz`：默认 Lua runtime 目录结构，包含 `lua_packages/`、`libs/`、`resources/` 与 `licenses/`。
- `lua-deps-{platform}.tar.gz`：供高级本地构建或其他 packages 工作流复用的原生依赖包。

### Demo 依赖拉取方式

demo 包内的独立依赖升级脚本支持三个目标。`run` 脚本只负责运行 demo，不会自动下载依赖。Windows 包携带 `upgrade_deps.bat`、`scripts/fetch_runtime_deps.ps1` 和 `run.ps1`；Linux/macOS 包携带 `upgrade_deps.sh`、`scripts/fetch_runtime_deps.sh` 和 `run.sh`。

- `all`：同时拉取 `lua-runtime-packages-{platform}.tar.gz`、`luaskills-ffi-sdk-{platform}.tar.gz` 与 vldb-controller。
- `lua`：只拉取并安装 `lua-runtime-packages-{platform}.tar.gz` 与 `luaskills-ffi-sdk-{platform}.tar.gz` 到 demo 的 `runtime/` 目录。
- `vldb`：只拉取 vldb-controller，并放入 demo runtime 的 `bin/` 目录。

一般使用 demo 时先通过 `upgrade_deps.bat` 或 `upgrade_deps.sh` 执行 `all`；只验证 Lua 包能力时执行 `lua`；已有 runtime、只缺 vldb-controller 时执行 `vldb`。

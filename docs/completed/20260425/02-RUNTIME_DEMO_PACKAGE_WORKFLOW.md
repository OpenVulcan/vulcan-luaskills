# 任务目标

围绕 `vulcan-luaskills` 建立 GitHub 端可发布的 Lua runtime、FFI SDK、FFI demo、Rust demo 与源码依赖包设计和脚本基础，使产物不再只是 `deps-v1` 的 C 依赖包，而是能按运行期默认目录导出、携带授权材料，并提供 demo 一键拉取 `lua` / `vldb` / `all` 依赖的入口。

# 详细执行步骤

1. 梳理 `vulcan-mcp-client/scripts` 中 `lua_packages.txt`、LuaRocks override、`install_lua_deps`、`install_host_deps`、`build` 的职责边界，确认哪些属于 luaskills runtime，哪些属于 host/vldb 拉取逻辑。
2. 在 `vulcan-luaskills` 中新增 `scripts/` 基础目录，迁入 Lua 包清单与 Windows LuaRocks override，作为 GitHub 自动构建和源码依赖包的统一输入。
3. 新增 runtime 包装脚本，将已构建产物裁剪为运行期默认目录，只导出 `lua_packages/lib/lua`、`lua_packages/share/lua`、必要原生运行库、资源清单、manifest 与授权材料，不导出 LuaJIT SDK、LuaRocks、构建工具和中间目录。
4. 新增统一拉取脚本，支持 `all` / `lua` / `vldb` 三类目标，将 Lua runtime 产物落到 demo 默认目录，将 `vldb-controller(.exe)` 放到 `output/bin/` 或指定 runtime 根目录的 `bin/` 下。
5. 新增 source-deps 包装脚本，生成面向 Git 源码构建用户的依赖清单包，包含依赖锁定、Lua 包清单、override、拉取脚本和授权入口。
6. 调整 demo 目录结构，新增 `examples/demo-ffi` 与 `examples/demo-rust` 的轻量入口说明和脚本，明确 FFI 模式与非 FFI/Rust 模式的边界。
7. 调整 GitHub workflow，使其在现有 C 依赖编译基础上预留并执行 runtime / FFI SDK / demo / source-deps 包装步骤，产物命名从单一 `lua-deps-{platform}.tar.gz` 扩展为多类发布资产。
8. 执行静态校验，至少覆盖 PowerShell 解析、Bash 语法检查、YAML 解析、关键文件存在性检查。
9. 对照计划逐项验证，确认不修改 Rust runtime 源码，仅新增或调整脚本、demo 与 workflow。
10. 在计划文件末尾追加执行变更总结，并在完成后迁移到 `docs/completed/20260425/02-RUNTIME_DEMO_PACKAGE_WORKFLOW.md`。

# 技术选型

- 继续使用 GitHub Actions `workflow_dispatch` 与现有多平台矩阵。
- 使用 PowerShell 与 Bash 双入口脚本，覆盖 Windows 与 Unix-like 环境。
- 使用 `manifest.json` 作为机器可读产物索引，使用 `licenses/manifest.json` 记录授权材料。
- 运行期 Lua 包只导出 LuaRocks 的 `lib/lua` 与 `share/lua` 目录，避免把构建期 LuaJIT SDK、LuaRocks 与工具链混入 runtime 包。
- `vldb-controller` 继续从 `OpenVulcan/vldb-controller` 发布产物获取，不打入 luaskills runtime 包，由 demo/source 脚本按目标单独拉取。

# 验收标准

1. `scripts/lua_packages.txt` 与 `scripts/luarocks_overrides/` 存在于 `vulcan-luaskills`。
2. 新增 runtime 包装脚本能够从 `third_party/lua_packages` 与 `third_party/deps` 导出 runtime 默认目录结构。
3. 新增拉取脚本支持 `all` / `lua` / `vldb`，并将 `vldb-controller(.exe)` 放入默认 `output/bin/`。
4. 新增 source-deps 包装脚本能够生成源码构建依赖包目录与 manifest。
5. demo 目录明确区分 FFI 与非 FFI/Rust 两种模式。
6. workflow 产物命名包含 runtime、FFI SDK、demo 与 source-deps，而不是只发布 `lua-deps`。
7. 所有新增脚本通过本地静态解析或语法检查。
8. 未修改 `src/` 下 Rust runtime 源码。

# 执行变更总结

## 1. 核心修复与调整概述

- 将 Lua runtime 构建职责从单一 `lua-deps` 扩展为 runtime 包、FFI SDK 包、FFI demo 包、Rust demo 包与 source-deps 包。
- 迁入 Lua 包清单与 LuaRocks override，使 GitHub workflow 能从 luaskills 仓库自身生成完整 Lua 需求库包。
- 新增统一依赖拉取脚本，支持 `all` / `lua` / `vldb`，并将 `vldb-controller(.exe)` 安装到指定 runtime 根目录的 `bin/` 下。
- 新增 runtime 包装脚本，按运行期默认目录裁剪产物，仅导出运行期需要的 `lua_packages`、`libs`、`resources` 与 `licenses`。
- 新增 FFI 与 Rust 两类 demo 入口，区分 FFI 模式与非 FFI/Rust 模式，并支持用户一键拉取依赖后运行。
- 修正 PowerShell 脚本在 Windows PowerShell 5.1 下的 UTF-8 中文注释解析问题，统一保存为 UTF-8 BOM。

## 2. 📂文件变更清单

- 新增：`scripts/lua_packages.txt`
- 新增：`scripts/luarocks_overrides/`
- 新增：`scripts/install_lua_deps.ps1`
- 新增：`scripts/install_lua_deps.sh`
- 新增：`scripts/package_lua_runtime.ps1`
- 新增：`scripts/package_lua_runtime.sh`
- 新增：`scripts/fetch_runtime_deps.ps1`
- 新增：`scripts/fetch_runtime_deps.sh`
- 新增：`scripts/package_source_deps.ps1`
- 新增：`scripts/package_source_deps.sh`
- 新增：`scripts/package_ffi_sdk.ps1`
- 新增：`scripts/package_ffi_sdk.sh`
- 新增：`scripts/package_demo.ps1`
- 新增：`scripts/package_demo.sh`
- 新增：`examples/demo-ffi/README.md`
- 新增：`examples/demo-ffi/run.ps1`
- 新增：`examples/demo-ffi/run.sh`
- 新增：`examples/demo-rust/Cargo.toml`
- 新增：`examples/demo-rust/Cargo.lock`
- 新增：`examples/demo-rust/src/main.rs`
- 新增：`examples/demo-rust/README.md`
- 新增：`examples/demo-rust/run.ps1`
- 新增：`examples/demo-rust/run.sh`
- 修改：`.github/workflows/build-lua-deps.yml`
- 修改：`README.md`

## 3. 💻关键代码调整详情

- `package_lua_runtime`：新增 runtime 默认格式导出逻辑，复制 LuaRocks `lib/lua` 与 `share/lua`，筛选原生运行库，排除 `lua51.dll`、`luajit.exe`、LuaRocks 与工具链，并生成 `lua-runtime-manifest.json` 与授权清单。
- `fetch_runtime_deps`：新增跨平台 release asset 解析、下载与解压逻辑，按 `lua`、`vldb`、`all` 三种目标落盘，其中 `vldb-controller` 固定进入 runtime 根目录 `bin/`。
- `package_source_deps`：新增源码依赖包封装，携带 Lua 包清单、override、依赖安装脚本、拉取脚本、包装脚本与授权文件，服务直接 Git 源码构建用户。
- `package_ffi_sdk`：新增 FFI SDK 包装，导出头文件、已构建动态/静态库、README 与授权文件。
- `package_demo`：新增 FFI/Rust demo 包装，携带 demo 源码、默认 runtime 目录、拉取脚本与 manifest，并排除本地 `target/` 构建缓存。
- `build-lua-deps.yml`：在既有 C deps 构建后增加 Rust release 构建、LuaRocks 安装、runtime/SDK/demo/source-deps 包装与统一上传步骤。
- `examples/demo-rust`：新增最小 Rust 集成 demo，通过 `vulcan-luaskills` crate 直接加载默认 runtime root，作为非 FFI 模式示例。

## 4. ⚠️遗留问题与注意事项

- 本地已完成 mock `third_party` 打包冒烟验证，但未在当前环境真实执行完整多平台 LuaRocks 构建，最终仍需以 GitHub Actions release workflow 结果作为跨平台确认。
- `vldb-controller` 不打入 luaskills runtime 包，仍由 `fetch_runtime_deps` 从 `OpenVulcan/vldb-controller` 独立拉取。
- `lua-deps-{platform}.tar.gz` 旧产物仍保留上传，便于兼容现有使用方；新的 runtime 默认包为 `lua-runtime-{platform}.tar.gz`。
- 本次未修改 `src/` 下 Rust runtime 源码，变更范围限定为 workflow、README、脚本与 demo。

## 5. Review 修复补充

- Unix 拉取脚本使用 `python3 -c` 从 `curl` 管道读取 GitHub release JSON，避免 here-doc 占用 stdin。
- 发布 tag 默认值统一为 `v0.1.0`，workflow、下载脚本、demo/source 包装脚本保持一致。
- demo 发布包生成包内专用 `run.ps1` / `run.sh`，不再依赖仓库源码相对路径。
- runtime 包新增 `resources/runtime-env.sh` 与 `resources/runtime-env.ps1`，demo 启动时自动设置 `LD_LIBRARY_PATH` / `DYLD_LIBRARY_PATH` / `PATH`。
- macOS/Linux 原生依赖扫描改为迭代模式，会继续扫描新复制进 `libs/` 的 dylib/so，覆盖 libcurl 等库的下游依赖。
- 复制到 `libs/` 的运行库会写入 `resources/bundled-libs.json`；如果没有找到随库提供的授权文件，会在对应 `licenses/native/<component>/LICENSE.reference.txt` 中记录 license 标识和来源路径。

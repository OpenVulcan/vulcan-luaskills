## Lua runtime and demo release packages / Lua runtime 与 demo 发布包说明

本 Release 用于发布 vulcan-luaskills 的跨平台 Lua runtime、FFI SDK、示例 demo 与构建期原生依赖包。每个平台会生成一组同名后缀的 `.tar.gz` 资产，例如 `linux-x64`、`linux-arm64`、`macos-x64`、`macos-arm64`、`windows-x64`。

This Release publishes cross-platform vulcan-luaskills Lua runtime packages, the FFI SDK, runnable demos, and native dependency bundles for build workflows. Each enabled platform emits a matching set of `.tar.gz` assets, such as `linux-x64`, `linux-arm64`, `macos-x64`, `macos-arm64`, and `windows-x64`.

### Assets / 资产用途

- `lua-runtime-{platform}.tar.gz`：面向最终运行期使用的 Lua runtime 默认目录包。它导出 `lua_packages/lib/lua`、`lua_packages/share/lua`、`libs`、`resources`、`licenses`，并包含运行 LuaSkills 常用 LuaRocks 模块及其原生运行库。Windows 包只携带 `runtime-env.ps1`，Linux/macOS 包只携带 `runtime-env.sh`。 / Default Lua runtime package for end users. It exports `lua_packages/lib/lua`, `lua_packages/share/lua`, `libs`, `resources`, and `licenses`, and includes the LuaRocks modules and native runtime libraries needed by LuaSkills. Windows packages include only `runtime-env.ps1`; Linux/macOS packages include only `runtime-env.sh`.
- `lua-deps-{platform}.tar.gz`：面向构建链路的原生依赖包，包含 OpenSSL、curl、zlib、pcre2、libyaml 等用于编译 LuaRocks C 模块的头文件、库文件和构建产物。这个包主要给 CI、源码构建或高级用户复用，不是 runtime 默认目录。 / Native dependency bundle for build workflows. It contains headers, libraries, and build outputs for OpenSSL, curl, zlib, pcre2, libyaml, and other dependencies used to compile LuaRocks C modules. This is mainly for CI, source builds, or advanced reuse, not the default runtime layout.
- `luaskills-ffi-sdk-{platform}.tar.gz`：面向 C ABI / 动态库宿主集成的 FFI SDK 包，包含 `include/` 头文件、`lib/` 下的 vulcan-luaskills 动态库或导入库，以及项目许可证。 / FFI SDK package for C ABI or dynamic-library host integration. It contains headers under `include/`, vulcan-luaskills runtime/import libraries under `lib/`, and the project license.
- `luaskills-demo-ffi-{platform}.tar.gz`：面向 FFI 模式的可运行 demo 包，演示外部宿主通过动态库加载 luaskills，并携带 `examples/ffi/` 下完整 C、Go、Python、TypeScript、标准 runtime、安装烟测和宿主 provider 示例，以及包内平台匹配的运行脚本、独立依赖升级脚本和依赖拉取脚本。 / Runnable FFI-mode demo package that shows an external host loading luaskills through the dynamic library. It includes the full `examples/ffi/` tree for C, Go, Python, TypeScript, standard runtime, install smoke tests, and host-provider demos, plus platform-matching runner scripts, standalone dependency upgrade scripts, and dependency fetch scripts.
- `luaskills-demo-rust-{platform}.tar.gz`：面向非 FFI / Rust 直连模式的可运行 demo 包，演示 Rust 宿主直接通过 `vulcan-luaskills` crate 使用默认 runtime root，并同样只携带平台匹配的运行与依赖升级脚本。 / Runnable non-FFI Rust demo package that shows a Rust host using the default runtime root through the `vulcan-luaskills` crate. It also includes platform-matching runner and dependency upgrade scripts.

### Bundled Lua packages / Lua runtime 内置 Lua 包

- `lua-cjson`：JSON 编码与解码。 / JSON encoding and decoding.
- `luafilesystem`：跨平台文件系统访问。 / Cross-platform filesystem access.
- `luasocket`：TCP、UDP 与基础网络能力。 / TCP, UDP, and basic networking support.
- `luasec`：基于 OpenSSL 的 TLS/HTTPS 支持。 / OpenSSL-backed TLS and HTTPS support.
- `lua-curl`：基于 libcurl 的 HTTP、HTTPS 与传输能力。 / libcurl-backed HTTP, HTTPS, and transfer support.
- `lrexlib-pcre2`：基于 PCRE2 的正则表达式能力。 / PCRE2-backed regular expression support.
- `luaossl`：OpenSSL 加密、证书与安全协议绑定。 / OpenSSL bindings for cryptography, certificates, and security protocols.
- `lyaml`：YAML 解析与生成。 / YAML parsing and generation.
- `lua-toml`：TOML 解析。 / TOML parsing.
- `serpent`：Lua 表序列化与调试输出。 / Lua table serialization and debugging output.
- `lua-zlib`：zlib 压缩与解压。 / zlib compression and decompression.

### Native libraries and licenses / 原生运行库与授权

runtime 包会携带 Lua C 模块实际需要的原生运行库，并在 `licenses/` 下带入授权材料。固定原生组件包括：

The runtime package includes the native libraries actually required by Lua C modules, with license materials under `licenses/`. Fixed native components include:

- `OpenSSL 3.4.1`：TLS、证书、加密能力。 / TLS, certificates, and cryptography.
- `curl 8.13.0`：HTTP/HTTPS 传输能力。 / HTTP and HTTPS transfer support.
- `zlib 1.3.1`：压缩能力。 / Compression support.
- `PCRE2 10.45`：正则表达式引擎。 / Regular expression engine.
- `libyaml 0.2.5`：YAML C 解析库。 / YAML C parser library.

### Demo dependency fetch targets / Demo 依赖拉取方式

demo 包内的独立依赖升级脚本支持三个目标。`run` 脚本只负责运行 demo，不会自动下载依赖。Windows 包携带 `upgrade_deps.bat`、`scripts/fetch_runtime_deps.ps1` 和 `run.ps1`，Linux/macOS 包携带 `upgrade_deps.sh`、`scripts/fetch_runtime_deps.sh` 和 `run.sh`。

Demo packages provide standalone dependency upgrade scripts with three targets. The `run` script only runs the demo and does not download dependencies automatically. Windows packages include `upgrade_deps.bat`, `scripts/fetch_runtime_deps.ps1`, and `run.ps1`; Linux/macOS packages include `upgrade_deps.sh`, `scripts/fetch_runtime_deps.sh`, and `run.sh`.

- `all`：同时拉取 Lua runtime 与 vldb-controller。 / Fetch both the Lua runtime and vldb-controller.
- `lua`：只拉取并安装 `lua-runtime-{platform}.tar.gz` 到 demo 的 `runtime/` 目录。 / Fetch only `lua-runtime-{platform}.tar.gz` and install it into the demo `runtime/` directory.
- `vldb`：只拉取 vldb-controller，并放入 demo runtime 的 `bin/` 目录。 / Fetch only vldb-controller and place it under the demo runtime `bin/` directory.

一般使用 demo 时先通过 `upgrade_deps.bat` 或 `upgrade_deps.sh` 执行 `all`；只验证 Lua 包能力时执行 `lua`；已有 runtime、只缺 vldb-controller 时执行 `vldb`。

In most demo scenarios, run `all` through `upgrade_deps.bat` or `upgrade_deps.sh` first. Use `lua` when you only need to validate Lua package capabilities. Use `vldb` when a runtime already exists and only vldb-controller is missing.

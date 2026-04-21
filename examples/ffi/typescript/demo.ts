/**
Minimal TypeScript example for the standard LuaSkills FFI surface using koffi.
使用 koffi 调用 LuaSkills 标准 FFI 接口的最小 TypeScript 示例。
 */

import koffi from "koffi";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

/**
Resolve the dynamic library path from one explicit environment variable.
从一个显式环境变量解析动态库路径。
 */
function resolveLibraryPath(): string {
  const libraryPath = process.env.VULCAN_LUASKILLS_LIB;
  if (!libraryPath) {
    throw new Error("VULCAN_LUASKILLS_LIB is not set");
  }
  return libraryPath;
}

/**
Resolve the shared demo runtime root bundled under examples/ffi/demo_runtime.
解析位于 examples/ffi/demo_runtime 下的共享演示运行时根目录。
 */
function resolveDemoRuntimeRoot(): string {
  const currentFile = fileURLToPath(import.meta.url);
  return path.join(path.dirname(path.dirname(currentFile)), "demo_runtime", "runtime_root");
}

/**
Ensure the shared demo runtime directory layout exists before engine creation.
在创建引擎前确保共享演示运行时目录结构存在。
 */
function ensureDemoRuntimeLayout(root: string): void {
  for (const relativePath of [
    "skills",
    "dependencies",
    "state",
    "databases",
    "temp",
    "resources",
    "lua_packages",
    path.join("bin", "tools"),
    "libs",
  ]) {
    fs.mkdirSync(path.join(root, relativePath), { recursive: true });
  }
}

/**
Read one nullable UTF-8 C string pointer into one JavaScript string.
将一个可空 UTF-8 C 字符串指针读取为一个 JavaScript 字符串。
 */
function readCString(pointer: string | Buffer | null): string {
  if (!pointer) {
    return "";
  }
  return pointer.toString();
}

/**
Raise one JavaScript error when the standard FFI call reports failure.
当标准 FFI 调用报告失败时抛出一个 JavaScript 错误。
 */
function mustOK(status: number, errorPtr: string | Buffer | null, freeString: (...args: unknown[]) => void): void {
  if (status === 0) {
    return;
  }
  const message = readCString(errorPtr);
  if (errorPtr) {
    freeString(errorPtr);
  }
  throw new Error(message);
}

/**
Run one simple engine create/free roundtrip against the standard FFI layer.
对标准 FFI 层执行一次简单的引擎创建与释放往返调用。
 */
function main(): void {
  const library = koffi.load(resolveLibraryPath());
  const runtimeRoot = resolveDemoRuntimeRoot();
  ensureDemoRuntimeLayout(runtimeRoot);

  const FfiLuaVmPoolConfig = koffi.struct("FfiLuaVmPoolConfig", {
    min_size: "size_t",
    max_size: "size_t",
    idle_ttl_secs: "uint64_t",
  });

  const FfiLuaRuntimeHostOptions = koffi.struct("FfiLuaRuntimeHostOptions", {
    temp_dir: "str",
    resources_dir: "str",
    lua_packages_dir: "str",
    luaexec_program: "str",
    host_provided_tool_root: "str",
    host_provided_lua_root: "str",
    host_provided_ffi_root: "str",
    download_cache_root: "str",
    dependency_dir_name: "str",
    state_dir_name: "str",
    database_dir_name: "str",
    protected_skill_ids: "void*",
    protected_skill_ids_len: "size_t",
    allow_network_download: "uint8_t",
    github_base_url: "str",
    github_api_base_url: "str",
    sqlite_library_path: "str",
    lancedb_library_path: "str",
    cache_config: "void*",
    reserved_entry_names: "void*",
    reserved_entry_names_len: "size_t",
    enable_skill_management_bridge: "uint8_t",
  });

  const FfiLuaEngineOptions = koffi.struct("FfiLuaEngineOptions", {
    pool: FfiLuaVmPoolConfig,
    host: FfiLuaRuntimeHostOptions,
  });

  const freeString = library.func("void vulcan_luaskills_ffi_string_free(char *value)");
  const version = library.func("int vulcan_luaskills_ffi_version(char **version_out, char **error_out)");
  const engineNew = library.func("int vulcan_luaskills_ffi_engine_new(const FfiLuaEngineOptions *options, uint64_t *engine_id_out, char **error_out)");
  const engineFree = library.func("int vulcan_luaskills_ffi_engine_free(uint64_t engine_id, char **error_out)");

  const versionOut = [null];
  const versionError = [null];
  mustOK(version(versionOut, versionError), versionError[0], freeString);
  console.log("Version:", readCString(versionOut[0]));
  if (versionOut[0]) {
    freeString(versionOut[0]);
  }

  const options = {
    pool: { min_size: 1, max_size: 1, idle_ttl_secs: 30 },
    host: {
      temp_dir: path.join(runtimeRoot, "temp"),
      resources_dir: path.join(runtimeRoot, "resources"),
      lua_packages_dir: path.join(runtimeRoot, "lua_packages"),
      luaexec_program: null,
      host_provided_tool_root: path.join(runtimeRoot, "bin", "tools"),
      host_provided_lua_root: path.join(runtimeRoot, "lua_packages"),
      host_provided_ffi_root: path.join(runtimeRoot, "libs"),
      download_cache_root: path.join(runtimeRoot, "temp", "downloads"),
      dependency_dir_name: "dependencies",
      state_dir_name: "state",
      database_dir_name: "databases",
      protected_skill_ids: null,
      protected_skill_ids_len: 0,
      allow_network_download: 0,
      github_base_url: null,
      github_api_base_url: null,
      sqlite_library_path: null,
      lancedb_library_path: null,
      cache_config: null,
      reserved_entry_names: null,
      reserved_entry_names_len: 0,
      enable_skill_management_bridge: 0,
    },
  };

  const engineIdOut = [0n];
  const engineError = [null];
  mustOK(engineNew(options, engineIdOut, engineError), engineError[0], freeString);
  console.log("Engine created:", engineIdOut[0].toString());

  const freeError = [null];
  mustOK(engineFree(engineIdOut[0], freeError), freeError[0], freeString);
  console.log("Engine freed");
}

main();

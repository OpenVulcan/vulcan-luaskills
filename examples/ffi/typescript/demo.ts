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
Resolve the dedicated standard-ABI fixture runtime root bundled under standard_runtime.
解析位于 standard_runtime 下供标准 ABI 示例共用的专用夹具运行时根目录。
 */
function resolveStandardFixtureRuntimeRoot(): string {
  const currentFile = fileURLToPath(import.meta.url);
  return path.join(path.dirname(path.dirname(currentFile)), "standard_runtime", "runtime_root");
}

/**
Ensure the shared standard-ABI fixture runtime directory layout exists.
确保标准 ABI 共用夹具运行时目录结构存在。
 */
function ensureStandardFixtureLayout(root: string): void {
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
Read one nested owned UTF-8 buffer into one JavaScript string without freeing it immediately.
将一个嵌套拥有型 UTF-8 缓冲读取为一个 JavaScript 字符串但不立即释放。
 */
function readOwnedBuffer(buffer: { ptr: Buffer | null; len: number | bigint } | null): string {
  if (!buffer?.ptr) {
    return "";
  }
  return Buffer.from(buffer.ptr).subarray(0, Number(buffer.len)).toString("utf8");
}

/**
Raise one JavaScript error when the standard FFI call reports failure.
当标准 FFI 调用报告失败时抛出一个 JavaScript 错误。
 */
function mustOK(
  status: number,
  errorBuffer: { ptr: Buffer | null; len: number | bigint } | null,
  freeBuffer: (buffer: { ptr: Buffer | null; len: number | bigint }) => void,
): void {
  if (status === 0) {
    return;
  }
  const message = readOwnedBuffer(errorBuffer);
  if (errorBuffer) {
    freeBuffer(errorBuffer);
  }
  throw new Error(message || "Unknown FFI error");
}

/**
Run one version query, one root load, and one structured entry-list read through the standard ABI.
通过标准 ABI 执行一次版本查询、一次根链加载和一次结构化入口列表读取。
 */
function main(): void {
  const library = koffi.load(resolveLibraryPath());
  const runtimeRoot = resolveStandardFixtureRuntimeRoot();
  ensureStandardFixtureLayout(runtimeRoot);

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
    protected_skill_ids: "void *",
    protected_skill_ids_len: "size_t",
    allow_network_download: "uint8_t",
    github_base_url: "str",
    github_api_base_url: "str",
    sqlite_library_path: "str",
    sqlite_provider_mode: "int32_t",
    sqlite_callback_mode: "int32_t",
    lancedb_library_path: "str",
    lancedb_provider_mode: "int32_t",
    lancedb_callback_mode: "int32_t",
    space_controller_endpoint: "str",
    space_controller_auto_spawn: "uint8_t",
    space_controller_executable_path: "str",
    space_controller_process_mode: "int32_t",
    cache_config: "void *",
    reserved_entry_names: "void *",
    reserved_entry_names_len: "size_t",
    enable_skill_management_bridge: "uint8_t",
  });

  const FfiLuaEngineOptions = koffi.struct("FfiLuaEngineOptions", {
    pool: FfiLuaVmPoolConfig,
    host: FfiLuaRuntimeHostOptions,
  });

  const FfiOwnedBuffer = koffi.struct("FfiOwnedBuffer", {
    ptr: "void *",
    len: "size_t",
  });

  const FfiRuntimeSkillRoot = koffi.struct("FfiRuntimeSkillRoot", {
    name: "str",
    skills_dir: "str",
  });

  const FfiRuntimeEntryParameterDescriptor = koffi.struct("FfiRuntimeEntryParameterDescriptor", {
    name: FfiOwnedBuffer,
    param_type: FfiOwnedBuffer,
    description: FfiOwnedBuffer,
    required: "uint8_t",
  });

  const FfiRuntimeEntryDescriptor = koffi.struct("FfiRuntimeEntryDescriptor", {
    canonical_name: FfiOwnedBuffer,
    skill_id: FfiOwnedBuffer,
    local_name: FfiOwnedBuffer,
    root_name: FfiOwnedBuffer,
    skill_dir: FfiOwnedBuffer,
    description: FfiOwnedBuffer,
    parameters: "void *",
    parameters_len: "size_t",
  });

  const FfiRuntimeEntryDescriptorList = koffi.struct("FfiRuntimeEntryDescriptorList", {
    items: "void *",
    len: "size_t",
  });

  const freeBuffer = library.func("void vulcan_luaskills_ffi_buffer_free(FfiOwnedBuffer value)");
  const version = library.func("int vulcan_luaskills_ffi_version(FfiOwnedBuffer *version_out, FfiOwnedBuffer *error_out)");
  const engineNew = library.func("int vulcan_luaskills_ffi_engine_new(const FfiLuaEngineOptions *options, uint64_t *engine_id_out, FfiOwnedBuffer *error_out)");
  const loadFromRoots = library.func("int vulcan_luaskills_ffi_load_from_roots(uint64_t engine_id, const FfiRuntimeSkillRoot *skill_roots, size_t skill_roots_len, FfiOwnedBuffer *error_out)");
  const listEntries = library.func("int vulcan_luaskills_ffi_list_entries(uint64_t engine_id, void **entries_out, FfiOwnedBuffer *error_out)");
  const freeEntryList = library.func("void vulcan_luaskills_ffi_entry_list_free(void *value)");
  const engineFree = library.func("int vulcan_luaskills_ffi_engine_free(uint64_t engine_id, FfiOwnedBuffer *error_out)");

  const versionOut = [{ ptr: null, len: 0 }];
  const versionError = [{ ptr: null, len: 0 }];
  mustOK(version(versionOut, versionError), versionError[0], freeBuffer);
  console.log("Version:", readOwnedBuffer(versionOut[0]));
  freeBuffer(versionOut[0]);

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
      sqlite_provider_mode: 0,
      sqlite_callback_mode: 0,
      lancedb_library_path: null,
      lancedb_provider_mode: 0,
      lancedb_callback_mode: 0,
      space_controller_endpoint: null,
      space_controller_auto_spawn: 0,
      space_controller_executable_path: null,
      space_controller_process_mode: 0,
      cache_config: null,
      reserved_entry_names: null,
      reserved_entry_names_len: 0,
      enable_skill_management_bridge: 0,
    },
  };

  const engineIdOut = [0n];
  const engineError = [{ ptr: null, len: 0 }];
  mustOK(engineNew(options, engineIdOut, engineError), engineError[0], freeBuffer);
  console.log("Engine created:", engineIdOut[0].toString());

  const rootArray = [
    {
      name: "ROOT",
      skills_dir: path.join(runtimeRoot, "skills"),
    },
  ];
  const loadError = [{ ptr: null, len: 0 }];
  mustOK(loadFromRoots(engineIdOut[0], rootArray, rootArray.length, loadError), loadError[0], freeBuffer);
  console.log("Loaded roots from:", path.join(runtimeRoot, "skills"));

  const entriesOut = [null];
  const entriesError = [{ ptr: null, len: 0 }];
  mustOK(listEntries(engineIdOut[0], entriesOut, entriesError), entriesError[0], freeBuffer);
  if (entriesOut[0]) {
    const entryList = koffi.decode(entriesOut[0], FfiRuntimeEntryDescriptorList) as {
      items: Buffer | null;
      len: number | bigint;
    };
    const entries = entryList.items
      ? (koffi.decode(
          entryList.items,
          koffi.array(FfiRuntimeEntryDescriptor, Number(entryList.len)),
        ) as Array<{
          canonical_name: { ptr: Buffer | null; len: number | bigint };
          skill_id: { ptr: Buffer | null; len: number | bigint };
          description: { ptr: Buffer | null; len: number | bigint };
          parameters: Buffer | null;
          parameters_len: number | bigint;
        }>)
      : [];
    console.log("Entry count:", entries.length);
    if (entries.length > 0) {
      const firstEntry = entries[0];
      console.log("First canonical entry:", readOwnedBuffer(firstEntry.canonical_name));
      console.log("First entry skill id:", readOwnedBuffer(firstEntry.skill_id));
      console.log("First entry description:", readOwnedBuffer(firstEntry.description));
      const parameters = firstEntry.parameters
        ? (koffi.decode(
            firstEntry.parameters,
            koffi.array(FfiRuntimeEntryParameterDescriptor, Number(firstEntry.parameters_len)),
          ) as Array<{
            name: { ptr: Buffer | null; len: number | bigint };
            param_type: { ptr: Buffer | null; len: number | bigint };
            required: number | bigint;
          }>)
        : [];
      console.log("First entry parameter count:", parameters.length);
      if (parameters.length > 0) {
        const firstParameter = parameters[0];
        console.log("First parameter name:", readOwnedBuffer(firstParameter.name));
        console.log("First parameter type:", readOwnedBuffer(firstParameter.param_type));
        console.log("First parameter required:", Number(firstParameter.required) !== 0);
      }
    } else {
      console.log("No entries were returned by the current fixture root.");
    }
    freeEntryList(entriesOut[0]);
  }

  const freeError = [{ ptr: null, len: 0 }];
  mustOK(engineFree(engineIdOut[0], freeError), freeError[0], freeBuffer);
  console.log("Engine freed");
}

main();

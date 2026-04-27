"""
Minimal Python ctypes example for the standard LuaSkills FFI surface.
LuaSkills 标准 FFI 接口的最小 Python ctypes 示例。
"""

import ctypes
import os
from pathlib import Path

# Full host-system authority for standard FFI query examples.
# 标准 FFI 查询示例使用的完整宿主系统权限。
LUASKILLS_SKILL_AUTHORITY_SYSTEM = 0


class FfiLuaVmPoolConfig(ctypes.Structure):
    """
    Plain engine-pool config passed into the standard FFI surface.
    传入标准 FFI 接口的原生引擎池配置。
    """

    _fields_ = [
        ("min_size", ctypes.c_size_t),
        ("max_size", ctypes.c_size_t),
        ("idle_ttl_secs", ctypes.c_uint64),
    ]


class FfiLuaRuntimeHostOptions(ctypes.Structure):
    """
    Plain host options passed into the standard FFI surface.
    传入标准 FFI 接口的原生宿主选项。
    """

    _fields_ = [
        ("temp_dir", ctypes.c_char_p),
        ("resources_dir", ctypes.c_char_p),
        ("lua_packages_dir", ctypes.c_char_p),
        ("host_provided_tool_root", ctypes.c_char_p),
        ("host_provided_lua_root", ctypes.c_char_p),
        ("host_provided_ffi_root", ctypes.c_char_p),
        ("download_cache_root", ctypes.c_char_p),
        ("dependency_dir_name", ctypes.c_char_p),
        ("state_dir_name", ctypes.c_char_p),
        ("database_dir_name", ctypes.c_char_p),
        ("skill_config_file_path", ctypes.c_char_p),
        ("allow_network_download", ctypes.c_uint8),
        ("github_base_url", ctypes.c_char_p),
        ("github_api_base_url", ctypes.c_char_p),
        ("sqlite_library_path", ctypes.c_char_p),
        ("sqlite_provider_mode", ctypes.c_int32),
        ("sqlite_callback_mode", ctypes.c_int32),
        ("lancedb_library_path", ctypes.c_char_p),
        ("lancedb_provider_mode", ctypes.c_int32),
        ("lancedb_callback_mode", ctypes.c_int32),
        ("space_controller_endpoint", ctypes.c_char_p),
        ("space_controller_auto_spawn", ctypes.c_uint8),
        ("space_controller_executable_path", ctypes.c_char_p),
        ("space_controller_process_mode", ctypes.c_int32),
        ("cache_config", ctypes.c_void_p),
        ("runlua_pool_config", ctypes.c_void_p),
        ("reserved_entry_names", ctypes.POINTER(ctypes.c_char_p)),
        ("reserved_entry_names_len", ctypes.c_size_t),
        ("ignored_skill_ids", ctypes.POINTER(ctypes.c_char_p)),
        ("ignored_skill_ids_len", ctypes.c_size_t),
        ("enable_skill_management_bridge", ctypes.c_uint8),
    ]


class FfiLuaEngineOptions(ctypes.Structure):
    """
    Plain engine options passed into the standard FFI surface.
    传入标准 FFI 接口的原生引擎选项。
    """

    _fields_ = [("pool", FfiLuaVmPoolConfig), ("host", FfiLuaRuntimeHostOptions)]


class FfiOwnedBuffer(ctypes.Structure):
    """
    Owned byte-buffer container returned by standard FFI outputs.
    标准 FFI 输出返回的拥有型字节缓冲容器。
    """

    _fields_ = [
        ("ptr", ctypes.POINTER(ctypes.c_uint8)),
        ("len", ctypes.c_size_t),
    ]


class FfiBorrowedBuffer(ctypes.Structure):
    """
    Borrowed byte-buffer container passed into standard FFI request fields.
    传入标准 FFI 请求字段的借用型字节缓冲容器。
    """

    _fields_ = [
        ("ptr", ctypes.POINTER(ctypes.c_uint8)),
        ("len", ctypes.c_size_t),
    ]


class FfiLuaInvocationContext(ctypes.Structure):
    """
    Plain invocation-context object passed into standard runtime invocation APIs.
    传入标准运行时调用接口的原生调用上下文对象。
    """

    _fields_ = [
        ("request_context_json", FfiBorrowedBuffer),
        ("client_budget_json", FfiBorrowedBuffer),
        ("tool_config_json", FfiBorrowedBuffer),
    ]


class FfiRuntimeInvocationResult(ctypes.Structure):
    """
    Plain invocation-result object returned by the standard invocation API.
    标准调用接口返回的原生调用结果对象。
    """

    _fields_ = [
        ("content", FfiOwnedBuffer),
        ("overflow_mode", ctypes.c_int32),
        ("template_hint", FfiOwnedBuffer),
        ("content_bytes", ctypes.c_size_t),
        ("content_lines", ctypes.c_size_t),
    ]


class FfiRuntimeSkillRoot(ctypes.Structure):
    """
    Plain skill-root descriptor used by the standard root-chain loader.
    标准根链加载器使用的原生技能根描述结构。
    """

    _fields_ = [
        ("name", ctypes.c_char_p),
        ("skills_dir", ctypes.c_char_p),
    ]


class FfiRuntimeEntryParameterDescriptor(ctypes.Structure):
    """
    Plain entry-parameter descriptor returned by the standard entry-list API.
    标准入口列表接口返回的原生入口参数描述结构。
    """

    _fields_ = [
        ("name", FfiOwnedBuffer),
        ("param_type", FfiOwnedBuffer),
        ("description", FfiOwnedBuffer),
        ("required", ctypes.c_uint8),
    ]


class FfiRuntimeEntryDescriptor(ctypes.Structure):
    """
    Plain entry descriptor returned by the standard entry-list API.
    标准入口列表接口返回的原生入口描述结构。
    """

    _fields_ = [
        ("canonical_name", FfiOwnedBuffer),
        ("skill_id", FfiOwnedBuffer),
        ("local_name", FfiOwnedBuffer),
        ("root_name", FfiOwnedBuffer),
        ("skill_dir", FfiOwnedBuffer),
        ("description", FfiOwnedBuffer),
        ("parameters", ctypes.POINTER(FfiRuntimeEntryParameterDescriptor)),
        ("parameters_len", ctypes.c_size_t),
    ]


class FfiRuntimeEntryDescriptorList(ctypes.Structure):
    """
    Plain entry-descriptor list returned by the standard entry-list API.
    标准入口列表接口返回的原生入口描述列表结构。
    """

    _fields_ = [
        ("items", ctypes.POINTER(FfiRuntimeEntryDescriptor)),
        ("len", ctypes.c_size_t),
    ]


def load_library() -> ctypes.CDLL:
    """
    Load the luaskills dynamic library from one explicit environment variable.
    从一个显式环境变量加载 luaskills 动态库。
    """

    library_path = os.environ.get("LUASKILLS_LIB")
    if not library_path:
        raise RuntimeError("LUASKILLS_LIB is not set")
    return ctypes.CDLL(str(Path(library_path)))


def standard_fixture_runtime_root() -> Path:
    """
    Resolve the dedicated standard-ABI fixture runtime root bundled under standard_runtime.
    解析位于 standard_runtime 下供标准 ABI 示例共用的专用夹具运行时根目录。
    """

    return Path(__file__).resolve().parent.parent / "standard_runtime" / "runtime_root"


def ensure_standard_fixture_layout(root: Path) -> None:
    """
    Ensure the shared standard-ABI fixture runtime directory layout exists.
    确保标准 ABI 共用夹具运行时目录结构存在。
    """

    for relative_path in [
        "skills",
        "dependencies",
        "state",
        "databases",
        "temp",
        "resources",
        "lua_packages",
        "bin/tools",
        "libs",
    ]:
        (root / relative_path).mkdir(parents=True, exist_ok=True)


def read_owned_buffer_text_and_free(buffer: FfiOwnedBuffer, library: ctypes.CDLL) -> str:
    """
    Read one owned UTF-8 buffer into one Python string and free it.
    将一个拥有型 UTF-8 缓冲读取为 Python 字符串并释放。
    """

    if not buffer.ptr:
        return ""
    text = ctypes.string_at(buffer.ptr, buffer.len).decode("utf-8")
    library.luaskills_ffi_buffer_free(buffer)
    return text


def read_owned_buffer_text(buffer: FfiOwnedBuffer) -> str:
    """
    Read one nested owned UTF-8 buffer without freeing it immediately.
    读取一个嵌套拥有型 UTF-8 缓冲但不立即释放。
    """

    if not buffer.ptr:
        return ""
    return ctypes.string_at(buffer.ptr, buffer.len).decode("utf-8")


def make_borrowed_buffer(text: str) -> tuple[ctypes.Array[ctypes.c_uint8], FfiBorrowedBuffer]:
    """
    Build one borrowed UTF-8 buffer whose payload stays alive in Python for one FFI call.
    构造一个在一次 FFI 调用期间由 Python 保持有效的借用型 UTF-8 缓冲。
    """

    payload = text.encode("utf-8")
    if not payload:
        return (ctypes.c_uint8 * 0)(), FfiBorrowedBuffer()
    payload_array = (ctypes.c_uint8 * len(payload))(*payload)
    return payload_array, FfiBorrowedBuffer(
        ptr=ctypes.cast(payload_array, ctypes.POINTER(ctypes.c_uint8)),
        len=len(payload),
    )


def must_ok(status: int, error_buffer: FfiOwnedBuffer, library: ctypes.CDLL) -> None:
    """
    Raise one Python exception when the standard FFI call reports failure.
    当标准 FFI 调用报告失败时抛出一个 Python 异常。
    """

    if status == 0:
        return
    message = read_owned_buffer_text_and_free(error_buffer, library) or "Unknown FFI error"
    raise RuntimeError(message)


def main() -> None:
    """
    Demonstrate version, engine lifecycle, root loading, entry listing, one standard call_skill roundtrip, and one standard run_lua roundtrip.
    演示版本查询、引擎生命周期、根链加载、入口列举、一次标准 call_skill 往返调用以及一次标准 run_lua 往返调用。
    """

    library = load_library()
    library.luaskills_ffi_buffer_free.argtypes = [FfiOwnedBuffer]
    library.luaskills_ffi_buffer_free.restype = None
    library.luaskills_ffi_version.argtypes = [
        ctypes.POINTER(FfiOwnedBuffer),
        ctypes.POINTER(FfiOwnedBuffer),
    ]
    library.luaskills_ffi_engine_new.argtypes = [
        ctypes.POINTER(FfiLuaEngineOptions),
        ctypes.POINTER(ctypes.c_uint64),
        ctypes.POINTER(FfiOwnedBuffer),
    ]
    library.luaskills_ffi_load_from_roots.argtypes = [
        ctypes.c_uint64,
        ctypes.POINTER(FfiRuntimeSkillRoot),
        ctypes.c_size_t,
        ctypes.POINTER(FfiOwnedBuffer),
    ]
    library.luaskills_ffi_list_entries.argtypes = [
        ctypes.c_uint64,
        ctypes.c_int32,
        ctypes.POINTER(ctypes.POINTER(FfiRuntimeEntryDescriptorList)),
        ctypes.POINTER(FfiOwnedBuffer),
    ]
    library.luaskills_ffi_call_skill.argtypes = [
        ctypes.c_uint64,
        ctypes.c_char_p,
        FfiBorrowedBuffer,
        ctypes.POINTER(FfiLuaInvocationContext),
        ctypes.POINTER(ctypes.POINTER(FfiRuntimeInvocationResult)),
        ctypes.POINTER(FfiOwnedBuffer),
    ]
    library.luaskills_ffi_run_lua.argtypes = [
        ctypes.c_uint64,
        ctypes.c_char_p,
        FfiBorrowedBuffer,
        ctypes.POINTER(FfiLuaInvocationContext),
        ctypes.POINTER(FfiOwnedBuffer),
        ctypes.POINTER(FfiOwnedBuffer),
    ]
    library.luaskills_ffi_entry_list_free.argtypes = [
        ctypes.POINTER(FfiRuntimeEntryDescriptorList),
    ]
    library.luaskills_ffi_entry_list_free.restype = None
    library.luaskills_ffi_invocation_result_free.argtypes = [
        ctypes.POINTER(FfiRuntimeInvocationResult),
    ]
    library.luaskills_ffi_invocation_result_free.restype = None
    library.luaskills_ffi_engine_free.argtypes = [
        ctypes.c_uint64,
        ctypes.POINTER(FfiOwnedBuffer),
    ]

    version_buffer = FfiOwnedBuffer()
    error_buffer = FfiOwnedBuffer()
    must_ok(
        library.luaskills_ffi_version(
            ctypes.byref(version_buffer), ctypes.byref(error_buffer)
        ),
        error_buffer,
        library,
    )
    print("Version:", read_owned_buffer_text_and_free(version_buffer, library))

    root = standard_fixture_runtime_root()
    ensure_standard_fixture_layout(root)

    host = FfiLuaRuntimeHostOptions()
    host.temp_dir = str((root / "temp").resolve()).replace("\\", "/").encode("utf-8")
    host.resources_dir = str((root / "resources").resolve()).replace("\\", "/").encode("utf-8")
    host.lua_packages_dir = str((root / "lua_packages").resolve()).replace("\\", "/").encode("utf-8")
    host.host_provided_tool_root = str((root / "bin" / "tools").resolve()).replace("\\", "/").encode("utf-8")
    host.host_provided_lua_root = str((root / "lua_packages").resolve()).replace("\\", "/").encode("utf-8")
    host.host_provided_ffi_root = str((root / "libs").resolve()).replace("\\", "/").encode("utf-8")
    host.download_cache_root = str((root / "temp" / "downloads").resolve()).replace("\\", "/").encode("utf-8")
    host.dependency_dir_name = b"dependencies"
    host.state_dir_name = b"state"
    host.database_dir_name = b"databases"
    host.skill_config_file_path = None
    host.allow_network_download = 0
    host.github_base_url = None
    host.github_api_base_url = None
    host.sqlite_library_path = None
    host.sqlite_provider_mode = 0
    host.sqlite_callback_mode = 0
    host.lancedb_library_path = None
    host.lancedb_provider_mode = 0
    host.lancedb_callback_mode = 0
    host.space_controller_endpoint = None
    host.space_controller_auto_spawn = 0
    host.space_controller_executable_path = None
    host.space_controller_process_mode = 0
    host.cache_config = None
    host.runlua_pool_config = None
    host.reserved_entry_names = None
    host.reserved_entry_names_len = 0
    host.ignored_skill_ids = None
    host.ignored_skill_ids_len = 0
    host.enable_skill_management_bridge = 0

    options = FfiLuaEngineOptions(
        pool=FfiLuaVmPoolConfig(min_size=1, max_size=1, idle_ttl_secs=30),
        host=host,
    )
    engine_id = ctypes.c_uint64()
    error_buffer = FfiOwnedBuffer()
    must_ok(
        library.luaskills_ffi_engine_new(
            ctypes.byref(options), ctypes.byref(engine_id), ctypes.byref(error_buffer)
        ),
        error_buffer,
        library,
    )
    print("Engine created:", engine_id.value)

    skill_roots = (FfiRuntimeSkillRoot * 1)(
        FfiRuntimeSkillRoot(
            name=b"ROOT",
            skills_dir=str((root / "skills").resolve()).replace("\\", "/").encode("utf-8"),
        )
    )
    error_buffer = FfiOwnedBuffer()
    must_ok(
        library.luaskills_ffi_load_from_roots(
            engine_id.value, skill_roots, len(skill_roots), ctypes.byref(error_buffer)
        ),
        error_buffer,
        library,
    )
    print("Loaded roots from:", root / "skills")

    entries_ptr = ctypes.POINTER(FfiRuntimeEntryDescriptorList)()
    error_buffer = FfiOwnedBuffer()
    must_ok(
        library.luaskills_ffi_list_entries(
            engine_id.value,
            LUASKILLS_SKILL_AUTHORITY_SYSTEM,
            ctypes.byref(entries_ptr),
            ctypes.byref(error_buffer),
        ),
        error_buffer,
        library,
    )
    try:
        entries_list = entries_ptr.contents
        print("Entry count:", entries_list.len)
        if entries_list.len > 0:
            first_entry = entries_list.items[0]
            print("First canonical entry:", read_owned_buffer_text(first_entry.canonical_name))
            print("First entry skill id:", read_owned_buffer_text(first_entry.skill_id))
            print("First entry description:", read_owned_buffer_text(first_entry.description))
            print("First entry parameter count:", first_entry.parameters_len)
            if first_entry.parameters_len > 0:
                first_parameter = first_entry.parameters[0]
                print("First parameter name:", read_owned_buffer_text(first_parameter.name))
                print("First parameter type:", read_owned_buffer_text(first_parameter.param_type))
                print(
                    "First parameter required:",
                    bool(first_parameter.required),
                )
        else:
            print("No entries were returned by the current fixture root.")
    finally:
        if entries_ptr:
            library.luaskills_ffi_entry_list_free(entries_ptr)

    args_storage, args_buffer = make_borrowed_buffer('{"note":"python"}')
    request_storage, request_buffer = make_borrowed_buffer('{"transport_name":"python-demo"}')
    budget_storage, budget_buffer = make_borrowed_buffer('{"budget":1}')
    tool_storage, tool_buffer = make_borrowed_buffer('{"mode":"standard-demo"}')
    invocation_context = FfiLuaInvocationContext(
        request_context_json=request_buffer,
        client_budget_json=budget_buffer,
        tool_config_json=tool_buffer,
    )
    borrowed_payloads = (
        args_storage,
        request_storage,
        budget_storage,
        tool_storage,
    )

    invocation_result_ptr = ctypes.POINTER(FfiRuntimeInvocationResult)()
    error_buffer = FfiOwnedBuffer()
    must_ok(
        library.luaskills_ffi_call_skill(
            engine_id.value,
            b"demo-standard-ffi-skill-ping",
            args_buffer,
            ctypes.byref(invocation_context),
            ctypes.byref(invocation_result_ptr),
            ctypes.byref(error_buffer),
        ),
        error_buffer,
        library,
    )
    try:
        invocation_result = invocation_result_ptr.contents
        print("Call content:", read_owned_buffer_text(invocation_result.content))
        print("Call content bytes:", invocation_result.content_bytes)
        print("Call content lines:", invocation_result.content_lines)
        print("Call template hint:", read_owned_buffer_text(invocation_result.template_hint))
    finally:
        if invocation_result_ptr:
            library.luaskills_ffi_invocation_result_free(invocation_result_ptr)

    run_lua_args_storage, run_lua_args_buffer = make_borrowed_buffer('{"note":"python-lua"}')
    run_lua_payloads = (run_lua_args_storage,)
    result_json_buffer = FfiOwnedBuffer()
    error_buffer = FfiOwnedBuffer()
    must_ok(
        library.luaskills_ffi_run_lua(
            engine_id.value,
            b'return { note = args.note, transport = vulcan.context.request.transport_name, budget = vulcan.context.client_budget.budget, mode = vulcan.context.tool_config.mode }',
            run_lua_args_buffer,
            ctypes.byref(invocation_context),
            ctypes.byref(result_json_buffer),
            ctypes.byref(error_buffer),
        ),
        error_buffer,
        library,
    )
    print("Run Lua result JSON:", read_owned_buffer_text_and_free(result_json_buffer, library))

    error_buffer = FfiOwnedBuffer()
    must_ok(
        library.luaskills_ffi_engine_free(engine_id, ctypes.byref(error_buffer)),
        error_buffer,
        library,
    )
    print("Engine freed")


if __name__ == "__main__":
    main()

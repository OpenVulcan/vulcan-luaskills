"""
Minimal Python ctypes example for the standard LuaSkills FFI surface.
LuaSkills 标准 FFI 接口的最小 Python ctypes 示例。
"""

import ctypes
import os
from pathlib import Path


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
        ("luaexec_program", ctypes.c_char_p),
        ("host_provided_tool_root", ctypes.c_char_p),
        ("host_provided_lua_root", ctypes.c_char_p),
        ("host_provided_ffi_root", ctypes.c_char_p),
        ("download_cache_root", ctypes.c_char_p),
        ("dependency_dir_name", ctypes.c_char_p),
        ("state_dir_name", ctypes.c_char_p),
        ("database_dir_name", ctypes.c_char_p),
        ("protected_skill_ids", ctypes.POINTER(ctypes.c_char_p)),
        ("protected_skill_ids_len", ctypes.c_size_t),
        ("allow_network_download", ctypes.c_uint8),
        ("github_base_url", ctypes.c_char_p),
        ("github_api_base_url", ctypes.c_char_p),
        ("sqlite_library_path", ctypes.c_char_p),
        ("lancedb_library_path", ctypes.c_char_p),
        ("cache_config", ctypes.c_void_p),
        ("reserved_entry_names", ctypes.POINTER(ctypes.c_char_p)),
        ("reserved_entry_names_len", ctypes.c_size_t),
    ]


class FfiLuaEngineOptions(ctypes.Structure):
    """
    Plain engine options passed into the standard FFI surface.
    传入标准 FFI 接口的原生引擎选项。
    """

    _fields_ = [("pool", FfiLuaVmPoolConfig), ("host", FfiLuaRuntimeHostOptions)]


def load_library() -> ctypes.CDLL:
    """
    Load the vulcan-luaskills dynamic library from one explicit environment variable.
    从一个显式环境变量加载 vulcan-luaskills 动态库。
    """

    library_path = os.environ.get("VULCAN_LUASKILLS_LIB")
    if not library_path:
        raise RuntimeError("VULCAN_LUASKILLS_LIB is not set")
    return ctypes.CDLL(str(Path(library_path)))


def demo_runtime_root() -> Path:
    """
    Resolve the shared demo runtime root bundled under examples/ffi/demo_runtime.
    解析位于 examples/ffi/demo_runtime 下的共享演示运行时根目录。
    """

    return Path(__file__).resolve().parent.parent / "demo_runtime" / "runtime_root"


def must_ok(status: int, error_ptr: ctypes.c_void_p, library: ctypes.CDLL) -> None:
    """
    Raise one Python exception when the standard FFI call reports failure.
    当标准 FFI 调用报告失败时抛出一个 Python 异常。
    """

    if status == 0:
        return
    message = ctypes.string_at(error_ptr).decode("utf-8") if error_ptr else "Unknown FFI error"
    if error_ptr:
        library.vulcan_luaskills_ffi_string_free(error_ptr)
    raise RuntimeError(message)


def main() -> None:
    """
    Demonstrate one version query and one engine create/free roundtrip.
    演示一次版本查询以及一次引擎创建与释放往返调用。
    """

    library = load_library()
    library.vulcan_luaskills_ffi_version.argtypes = [
        ctypes.POINTER(ctypes.c_void_p),
        ctypes.POINTER(ctypes.c_void_p),
    ]
    library.vulcan_luaskills_ffi_engine_new.argtypes = [
        ctypes.POINTER(FfiLuaEngineOptions),
        ctypes.POINTER(ctypes.c_uint64),
        ctypes.POINTER(ctypes.c_void_p),
    ]
    library.vulcan_luaskills_ffi_engine_free.argtypes = [
        ctypes.c_uint64,
        ctypes.POINTER(ctypes.c_void_p),
    ]
    library.vulcan_luaskills_ffi_string_free.argtypes = [ctypes.c_void_p]

    version_ptr = ctypes.c_void_p()
    error_ptr = ctypes.c_void_p()
    must_ok(
        library.vulcan_luaskills_ffi_version(
            ctypes.byref(version_ptr), ctypes.byref(error_ptr)
        ),
        error_ptr,
        library,
    )
    print("Version:", ctypes.string_at(version_ptr).decode("utf-8"))
    library.vulcan_luaskills_ffi_string_free(version_ptr)

    root = demo_runtime_root()
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

    host = FfiLuaRuntimeHostOptions()
    host.temp_dir = str((root / "temp").resolve()).replace("\\", "/").encode("utf-8")
    host.resources_dir = str((root / "resources").resolve()).replace("\\", "/").encode("utf-8")
    host.lua_packages_dir = str((root / "lua_packages").resolve()).replace("\\", "/").encode("utf-8")
    host.luaexec_program = None
    host.host_provided_tool_root = str((root / "bin" / "tools").resolve()).replace("\\", "/").encode("utf-8")
    host.host_provided_lua_root = str((root / "lua_packages").resolve()).replace("\\", "/").encode("utf-8")
    host.host_provided_ffi_root = str((root / "libs").resolve()).replace("\\", "/").encode("utf-8")
    host.download_cache_root = str((root / "temp" / "downloads").resolve()).replace("\\", "/").encode("utf-8")
    host.dependency_dir_name = b"dependencies"
    host.state_dir_name = b"state"
    host.database_dir_name = b"databases"
    host.protected_skill_ids = None
    host.protected_skill_ids_len = 0
    host.allow_network_download = 0
    host.github_base_url = None
    host.github_api_base_url = None
    host.sqlite_library_path = None
    host.lancedb_library_path = None
    host.cache_config = None
    host.reserved_entry_names = None
    host.reserved_entry_names_len = 0

    options = FfiLuaEngineOptions(
        pool=FfiLuaVmPoolConfig(min_size=1, max_size=1, idle_ttl_secs=30),
        host=host,
    )
    engine_id = ctypes.c_uint64()
    error_ptr = ctypes.c_void_p()
    must_ok(
        library.vulcan_luaskills_ffi_engine_new(
            ctypes.byref(options), ctypes.byref(engine_id), ctypes.byref(error_ptr)
        ),
        error_ptr,
        library,
    )
    print("Engine created:", engine_id.value)

    error_ptr = ctypes.c_void_p()
    must_ok(
        library.vulcan_luaskills_ffi_engine_free(engine_id, ctypes.byref(error_ptr)),
        error_ptr,
        library,
    )
    print("Engine freed")


if __name__ == "__main__":
    main()

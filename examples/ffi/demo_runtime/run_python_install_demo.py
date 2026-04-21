"""
Runnable smoke demo that installs one managed LuaSkill and calls one tool through FFI.
通过 FFI 安装一个受管 LuaSkill 并调用一个工具的可运行烟测示例。
"""

import ctypes
import json
import os
import shutil
from pathlib import Path


DEMO_SKILL_ID = "luaskills-demo-skill"
DEMO_SKILL_REPO = "OpenVulcan/luaskills-demo-skill"
DEMO_TOOL_NAME = "luaskills-demo-skill-demo-status"


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
        ("enable_skill_management_bridge", ctypes.c_uint8),
    ]


class FfiLuaEngineOptions(ctypes.Structure):
    """
    Plain engine options passed into the standard FFI surface.
    传入标准 FFI 接口的原生引擎选项。
    """

    _fields_ = [("pool", FfiLuaVmPoolConfig), ("host", FfiLuaRuntimeHostOptions)]


def runtime_root() -> Path:
    """
    Resolve the shared demo runtime root path.
    解析共享演示运行时根目录路径。
    """

    return Path(__file__).resolve().parent / "runtime_root"


def ensure_runtime_layout(root: Path) -> None:
    """
    Create the required runtime sibling directories when they are missing.
    在缺失时创建必需的运行时兄弟目录。
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


def reset_demo_skill_state(root: Path) -> None:
    """
    Remove one previous demo-skill install so the smoke test starts from an empty state.
    删除先前的 demo skill 安装痕迹，使烟测从空状态开始。
    """

    for path in [
        root / "skills" / DEMO_SKILL_ID,
        root / "dependencies" / "tools" / DEMO_SKILL_ID,
        root / "dependencies" / "lua" / DEMO_SKILL_ID,
        root / "dependencies" / "ffi" / DEMO_SKILL_ID,
        root / "databases" / "sqlite" / DEMO_SKILL_ID,
        root / "databases" / "lancedb" / DEMO_SKILL_ID,
    ]:
        if path.exists():
            shutil.rmtree(path, ignore_errors=False)

    for path in [
        root / "state" / "installs" / f"{DEMO_SKILL_ID}.yaml",
        root / "state" / "skills" / "disabled" / f"{DEMO_SKILL_ID}.json",
    ]:
        if path.exists():
            path.unlink()


def load_library() -> ctypes.CDLL:
    """
    Load the vulcan-luaskills dynamic library from one explicit environment variable.
    从一个显式环境变量加载 vulcan-luaskills 动态库。
    """

    library_path = os.environ.get("VULCAN_LUASKILLS_LIB")
    if not library_path:
        raise RuntimeError("VULCAN_LUASKILLS_LIB is not set")
    return ctypes.CDLL(str(Path(library_path)))


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


def decode_json_response(raw_ptr: ctypes.c_void_p, library: ctypes.CDLL) -> dict:
    """
    Decode one JSON envelope returned by one `_json` FFI function.
    解码一个由 `_json` FFI 函数返回的 JSON 包络。
    """

    if not raw_ptr:
        raise RuntimeError("FFI JSON call returned null")
    text = ctypes.string_at(raw_ptr).decode("utf-8")
    library.vulcan_luaskills_ffi_string_free(raw_ptr)
    payload = json.loads(text)
    if payload.get("ok") is not True:
        raise RuntimeError(payload.get("error") or "Unknown JSON FFI error")
    return payload["result"]


def call_json_ffi(library: ctypes.CDLL, function_name: str, payload: dict) -> dict:
    """
    Call one `_json` FFI function with one JSON payload.
    使用一个 JSON 载荷调用一个 `_json` FFI 函数。
    """

    ffi_function = getattr(library, function_name)
    ffi_function.argtypes = [ctypes.c_char_p]
    ffi_function.restype = ctypes.c_void_p
    input_bytes = json.dumps(payload).encode("utf-8")
    return decode_json_response(ffi_function(ctypes.c_char_p(input_bytes)), library)


def normalized_path(path: Path) -> str:
    """
    Convert one path into one normalized POSIX-like string for FFI JSON requests.
    将一个路径转换为供 FFI JSON 请求使用的规范 POSIX 风格字符串。
    """

    return str(path.resolve()).replace("\\", "/")


def build_engine_options(root: Path) -> FfiLuaEngineOptions:
    """
    Build one deterministic engine configuration for the smoke runtime root.
    为烟测运行时根构造一份确定性的引擎配置。
    """

    host = FfiLuaRuntimeHostOptions()
    host.temp_dir = normalized_path(root / "temp").encode("utf-8")
    host.resources_dir = normalized_path(root / "resources").encode("utf-8")
    host.lua_packages_dir = normalized_path(root / "lua_packages").encode("utf-8")
    host.luaexec_program = None
    host.host_provided_tool_root = normalized_path(root / "bin" / "tools").encode("utf-8")
    host.host_provided_lua_root = normalized_path(root / "lua_packages").encode("utf-8")
    host.host_provided_ffi_root = normalized_path(root / "libs").encode("utf-8")
    host.download_cache_root = normalized_path(root / "temp" / "downloads").encode("utf-8")
    host.dependency_dir_name = b"dependencies"
    host.state_dir_name = b"state"
    host.database_dir_name = b"databases"
    host.protected_skill_ids = None
    host.protected_skill_ids_len = 0
    host.allow_network_download = 1
    host.github_base_url = None
    host.github_api_base_url = None
    host.sqlite_library_path = None
    host.lancedb_library_path = None
    host.cache_config = None
    host.reserved_entry_names = None
    host.reserved_entry_names_len = 0
    host.enable_skill_management_bridge = 0

    return FfiLuaEngineOptions(
        pool=FfiLuaVmPoolConfig(min_size=1, max_size=1, idle_ttl_secs=30),
        host=host,
    )


def main() -> None:
    """
    Run one install-plus-call smoke test against the shared demo runtime root.
    针对共享演示运行时根执行一次安装加调用的烟测。
    """

    root = runtime_root()
    ensure_runtime_layout(root)
    reset_demo_skill_state(root)

    library = load_library()
    library.vulcan_luaskills_ffi_string_free.argtypes = [ctypes.c_void_p]
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

    version_ptr = ctypes.c_void_p()
    error_ptr = ctypes.c_void_p()
    must_ok(
        library.vulcan_luaskills_ffi_version(
            ctypes.byref(version_ptr), ctypes.byref(error_ptr)
        ),
        error_ptr,
        library,
    )
    print("FFI version:", ctypes.string_at(version_ptr).decode("utf-8"))
    library.vulcan_luaskills_ffi_string_free(version_ptr)

    engine_id = ctypes.c_uint64()
    options = build_engine_options(root)
    error_ptr = ctypes.c_void_p()
    must_ok(
        library.vulcan_luaskills_ffi_engine_new(
            ctypes.byref(options), ctypes.byref(engine_id), ctypes.byref(error_ptr)
        ),
        error_ptr,
        library,
    )

    try:
        roots_payload = {
            "engine_id": engine_id.value,
            "skill_roots": [
                {
                    "name": "ROOT",
                    "skills_dir": normalized_path(root / "skills"),
                }
            ],
        }
        call_json_ffi(library, "vulcan_luaskills_ffi_load_from_roots_json", roots_payload)

        install_result = call_json_ffi(
            library,
            "vulcan_luaskills_ffi_install_skill_json",
            {
                "engine_id": engine_id.value,
                "skill_roots": roots_payload["skill_roots"],
                "request": {
                    "source": DEMO_SKILL_REPO,
                    "source_type": "github",
                },
            },
        )
        print("Install status:", install_result["status"])

        invocation_result = call_json_ffi(
            library,
            "vulcan_luaskills_ffi_call_skill_json",
            {
                "engine_id": engine_id.value,
                "tool_name": DEMO_TOOL_NAME,
                "args": {"name": "ffi-demo"},
            },
        )

        payload = json.loads(invocation_result["content"])
        if payload.get("ok") is not True:
            raise RuntimeError("Demo skill returned non-success payload")

        print("Skill call success")
        print(json.dumps(payload, ensure_ascii=False, indent=2))
        print("success")
    finally:
        error_ptr = ctypes.c_void_p()
        must_ok(
            library.vulcan_luaskills_ffi_engine_free(
                engine_id, ctypes.byref(error_ptr)
            ),
            error_ptr,
            library,
        )


if __name__ == "__main__":
    main()

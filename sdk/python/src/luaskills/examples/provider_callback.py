"""
Python SDK JSON provider callback registration example shipped with the wheel.
随 wheel 分发的 Python SDK JSON provider callback 注册示例。
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from luaskills import LuaSkillsClient, LuaSkillsJsonFfi


def resolve_library_path() -> Path:
    """
    Resolve the demo LuaSkills dynamic library path.
    解析演示用 LuaSkills 动态库路径。
    """

    return Path(os.environ.get("LUASKILLS_LIB") or Path.cwd() / "target" / "debug" / "luaskills.dll")


def sqlite_provider(request: Any) -> dict[str, Any]:
    """
    Return one minimal host-side SQLite provider response for demo requests.
    为演示请求返回一个最小宿主侧 SQLite provider 响应。
    """

    return {"ok": True, "request": request}


def main() -> None:
    """
    Register one SQLite JSON provider callback before engine creation.
    在创建引擎前注册单个 SQLite JSON provider callback。
    """

    library_path = resolve_library_path()
    runtime_root = Path(os.environ.get("LUASKILLS_RUNTIME_ROOT") or Path.cwd() / "luaskills-runtime")
    ffi = LuaSkillsJsonFfi(library_path)
    ffi.set_sqlite_provider_json_callback(sqlite_provider)
    try:
        client = LuaSkillsClient(
            library_path=library_path,
            runtime_root=runtime_root,
            host_options={
                "sqlite_provider_mode": "host_callback",
                "sqlite_callback_mode": "json",
            },
        )
        client.close()
        print("SQLite JSON provider callback registered before engine creation.")
    finally:
        ffi.clear_sqlite_provider_json_callback()


if __name__ == "__main__":
    main()

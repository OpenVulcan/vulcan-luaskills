"""
Basic Python SDK version query example.
Python SDK 基础版本查询示例。
"""

from __future__ import annotations

import os
from pathlib import Path

from luaskills import LuaSkillsClient


def resolve_library_path() -> Path:
    """
    Resolve the demo LuaSkills dynamic library path.
    解析演示用 LuaSkills 动态库路径。
    """

    return Path(os.environ.get("LUASKILLS_LIB") or Path(__file__).resolve().parents[3] / "target" / "debug" / "luaskills.dll")


def main() -> None:
    """
    Print the LuaSkills JSON FFI version through the Python SDK.
    通过 Python SDK 输出 LuaSkills JSON FFI 版本。
    """

    print(LuaSkillsClient.version(library_path=resolve_library_path()))


if __name__ == "__main__":
    main()

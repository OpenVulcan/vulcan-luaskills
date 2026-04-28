"""
Runtime root helpers for the formal LuaSkills ROOT, PROJECT, USER chain.
正式 LuaSkills ROOT、PROJECT、USER 链的运行时 root 辅助工具。
"""

from __future__ import annotations

from pathlib import Path

from .types import RuntimeSkillRoot


class RuntimeRoots:
    """
    Helper utilities for building and locating formal runtime skill roots.
    用于构造和定位正式运行时 skill root 的辅助工具。
    """

    @staticmethod
    def standard(
        runtime_root: str | Path,
        *,
        include_project: bool = True,
        include_user: bool = True,
        root_skills_dir_name: str = "root_skills",
        project_skills_dir_name: str = "project_skills",
        user_skills_dir_name: str = "user_skills",
    ) -> list[RuntimeSkillRoot]:
        """
        Build one standard formal root chain from a shared runtime root.
        基于共享 runtime root 构造一条标准正式 root 链。
        """

        root = Path(runtime_root).expanduser().resolve()
        roots = [RuntimeSkillRoot("ROOT", normalized_path(root / root_skills_dir_name))]
        if include_project:
            roots.append(RuntimeSkillRoot("PROJECT", normalized_path(root / project_skills_dir_name)))
        if include_user:
            roots.append(RuntimeSkillRoot("USER", normalized_path(root / user_skills_dir_name)))
        return roots

    @staticmethod
    def root_only(runtime_root: str | Path) -> list[RuntimeSkillRoot]:
        """
        Build one ROOT-only chain for system-only hosts.
        为仅系统层宿主构造一条仅 ROOT 的 root 链。
        """

        return RuntimeRoots.standard(runtime_root, include_project=False, include_user=False)

    @staticmethod
    def find_by_label(skill_roots: list[RuntimeSkillRoot | dict[str, str]], label: str) -> RuntimeSkillRoot | dict[str, str]:
        """
        Find one root by formal label using trim and uppercase normalization.
        使用 trim 与大写归一化按正式标签查找单个 root。
        """

        normalized_label = label.strip().upper()
        for root in skill_roots:
            name = root.name if isinstance(root, RuntimeSkillRoot) else root["name"]
            if name.strip().upper() == normalized_label:
                return root
        raise ValueError(f"Runtime skill root '{normalized_label}' is not present in the configured root chain")

    @staticmethod
    def ensure_layout(runtime_root: str | Path, skill_roots: list[RuntimeSkillRoot | dict[str, str]] | None = None) -> None:
        """
        Create runtime directories needed by default SDK host options and root chain.
        创建默认 SDK 宿主选项和 root 链所需的运行时目录。
        """

        root = Path(runtime_root).expanduser().resolve()
        roots = skill_roots or RuntimeRoots.standard(root)
        directories = [
            root,
            root / "temp",
            root / "temp" / "downloads",
            root / "resources",
            root / "lua_packages",
            root / "bin" / "tools",
            root / "libs",
            root / "dependencies",
            root / "state",
            root / "databases",
        ]
        for skill_root in roots:
            skills_dir = skill_root.skills_dir if isinstance(skill_root, RuntimeSkillRoot) else skill_root["skills_dir"]
            directories.append(Path(skills_dir))
        for directory in directories:
            directory.mkdir(parents=True, exist_ok=True)


def normalized_path(path: str | Path) -> str:
    """
    Return one resolved POSIX-style path string for JSON FFI payloads.
    返回供 JSON FFI 载荷使用的已解析 POSIX 风格路径字符串。
    """

    return str(Path(path).expanduser().resolve()).replace("\\", "/")

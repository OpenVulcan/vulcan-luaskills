"""
High-level Python client for the LuaSkills public JSON FFI surface.
LuaSkills 公共 JSON FFI 表面的高级 Python 客户端。
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from .ffi import LuaSkillsJsonFfi
from .roots import RuntimeRoots, normalized_path
from .runtime_assets import host_options_from_runtime_manifest, load_runtime_install_manifest
from .types import Authority, JsonValue, LuaInvocationContext, RuntimeSkillRoot, authority_value, roots_to_json


class LuaSkillsClient:
    """
    High-level LuaSkills SDK client over the public JSON FFI surface.
    基于公共 JSON FFI 表面的高级 LuaSkills SDK 客户端。
    """

    def __init__(
        self,
        *,
        library_path: str | os.PathLike[str] | None = None,
        runtime_root: str | os.PathLike[str] | None = None,
        engine_options: dict[str, Any] | None = None,
        host_options: dict[str, Any] | None = None,
        pool_config: dict[str, Any] | None = None,
        ensure_runtime_layout: bool = True,
    ) -> None:
        """
        Create one native LuaSkills engine and wrap it in a high-level client.
        创建一个原生 LuaSkills 引擎并封装为高级客户端。
        """

        runtime_root_path = Path(runtime_root or Path.cwd() / "luaskills-runtime").expanduser().resolve()
        self.ffi = LuaSkillsJsonFfi(library_path, runtime_root_path)
        options = engine_options or create_engine_options(runtime_root_path, host_options=host_options, pool_config=pool_config)
        if engine_options is None and ensure_runtime_layout:
            RuntimeRoots.ensure_layout(runtime_root_path)
        handle = self.ffi.call_json("luaskills_ffi_engine_new_json", {"options": options})
        self.engine_id = int(handle["engine_id"])
        self.closed = False
        self.config = SkillConfigClient(self)
        self.skills = SkillManagementClient(self, system_plane=False)

    def __enter__(self) -> "LuaSkillsClient":
        """
        Return this client when used as a context manager.
        作为上下文管理器使用时返回当前客户端。
        """

        return self

    def __exit__(self, exc_type: object, exc: object, traceback: object) -> None:
        """
        Close the native engine handle when leaving a context manager.
        离开上下文管理器时关闭原生引擎句柄。
        """

        self.close()

    @staticmethod
    def version(
        *,
        library_path: str | os.PathLike[str] | None = None,
        runtime_root: str | os.PathLike[str] | None = None,
    ) -> dict[str, Any]:
        """
        Query the JSON FFI version without creating a runtime engine.
        不创建运行时引擎并查询 JSON FFI 版本。
        """

        return LuaSkillsJsonFfi(library_path, runtime_root).call_json_no_input("luaskills_ffi_version_json")

    @staticmethod
    def describe(
        *,
        library_path: str | os.PathLike[str] | None = None,
        runtime_root: str | os.PathLike[str] | None = None,
    ) -> dict[str, Any]:
        """
        Query the JSON FFI self-description without creating a runtime engine.
        不创建运行时引擎并查询 JSON FFI 自描述。
        """

        return LuaSkillsJsonFfi(library_path, runtime_root).call_json_no_input("luaskills_ffi_describe_json")

    def system(self, authority: Authority | str = Authority.SYSTEM) -> "SystemSkillManagementClient":
        """
        Return one system-management namespace bound to host-injected authority.
        返回绑定到宿主注入权限的 system 管理命名空间。
        """

        return SystemSkillManagementClient(self, authority)

    def load_from_dirs(self, base_dir: str | os.PathLike[str], override_dir: str | os.PathLike[str] | None = None) -> dict[str, Any]:
        """
        Load skills from legacy directory-style root options.
        从旧目录风格 root 选项加载 skills。
        """

        return self._call("luaskills_ffi_load_from_dirs_json", {
            "engine_id": self.engine_id,
            "base_dir": normalized_path(base_dir),
            "override_dir": normalized_path(override_dir) if override_dir else None,
        })

    def load_from_roots(self, skill_roots: list[RuntimeSkillRoot | dict[str, str]]) -> dict[str, Any]:
        """
        Load skills from the formal ordered root chain.
        从正式有序 root 链加载 skills。
        """

        return self._call("luaskills_ffi_load_from_roots_json", {
            "engine_id": self.engine_id,
            "skill_roots": roots_to_json(skill_roots),
        })

    def reload_from_roots(self, skill_roots: list[RuntimeSkillRoot | dict[str, str]]) -> dict[str, Any]:
        """
        Reload skills from the formal ordered root chain.
        从正式有序 root 链重载 skills。
        """

        return self._call("luaskills_ffi_reload_from_roots_json", {
            "engine_id": self.engine_id,
            "skill_roots": roots_to_json(skill_roots),
        })

    def list_entries(self, authority: Authority | str = Authority.DELEGATED_TOOL) -> list[dict[str, Any]]:
        """
        List runtime entries visible to the selected authority.
        列出指定权限可见的运行时入口。
        """

        return self._call("luaskills_ffi_list_entries_json", {"engine_id": self.engine_id, "authority": authority_value(authority)})

    def list_skill_help(self, authority: Authority | str = Authority.DELEGATED_TOOL) -> list[dict[str, Any]]:
        """
        List runtime help trees visible to the selected authority.
        列出指定权限可见的运行时帮助树。
        """

        return self._call("luaskills_ffi_list_skill_help_json", {"engine_id": self.engine_id, "authority": authority_value(authority)})

    def render_skill_help_detail(
        self,
        skill_id: str,
        flow_name: str = "main",
        *,
        authority: Authority | str = Authority.DELEGATED_TOOL,
        request_context: JsonValue | None = None,
    ) -> dict[str, Any] | None:
        """
        Render one help flow detail visible to the selected authority.
        渲染指定权限可见的单个帮助流程详情。
        """

        return self._call("luaskills_ffi_render_skill_help_detail_json", {
            "engine_id": self.engine_id,
            "skill_id": skill_id,
            "flow_name": flow_name,
            "request_context": request_context,
            "authority": authority_value(authority),
        })

    def prompt_argument_completions(
        self,
        prompt_name: str,
        argument_name: str,
        authority: Authority | str = Authority.DELEGATED_TOOL,
    ) -> list[str] | None:
        """
        Query prompt argument completions visible to the selected authority.
        查询指定权限可见的 prompt 参数补全项。
        """

        return self._call("luaskills_ffi_prompt_argument_completions_json", {
            "engine_id": self.engine_id,
            "prompt_name": prompt_name,
            "argument_name": argument_name,
            "authority": authority_value(authority),
        })

    def is_skill(self, tool_name: str, authority: Authority | str = Authority.DELEGATED_TOOL) -> bool:
        """
        Return whether one canonical tool name is visible as a skill entry.
        返回指定 canonical 工具名是否可见为 skill 入口。
        """

        result = self._call("luaskills_ffi_is_skill_json", {
            "engine_id": self.engine_id,
            "tool_name": tool_name,
            "authority": authority_value(authority),
        })
        return bool(result["value"])

    def skill_name_for_tool(self, tool_name: str, authority: Authority | str = Authority.DELEGATED_TOOL) -> str | None:
        """
        Resolve the owning skill id for one visible canonical tool name.
        解析单个可见 canonical 工具名称所属的 skill id。
        """

        result = self._call("luaskills_ffi_skill_name_for_tool_json", {
            "engine_id": self.engine_id,
            "tool_name": tool_name,
            "authority": authority_value(authority),
        })
        return result.get("skill_id")

    def call_skill(
        self,
        tool_name: str,
        args: JsonValue | None = None,
        invocation_context: LuaInvocationContext | dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """
        Call one active skill entry by canonical tool name.
        按 canonical 工具名称调用单个已激活 skill 入口。
        """

        return self._call("luaskills_ffi_call_skill_json", {
            "engine_id": self.engine_id,
            "tool_name": tool_name,
            "args": args or {},
            "invocation_context": invocation_context_to_json(invocation_context),
        })

    def run_lua(
        self,
        code: str,
        args: JsonValue | None = None,
        invocation_context: LuaInvocationContext | dict[str, Any] | None = None,
    ) -> Any:
        """
        Execute one inline Lua snippet against the active runtime.
        针对当前活动运行时执行单段内联 Lua。
        """

        return self._call("luaskills_ffi_run_lua_json", {
            "engine_id": self.engine_id,
            "code": code,
            "args": args or {},
            "invocation_context": invocation_context_to_json(invocation_context),
        })

    def close(self) -> dict[str, Any] | None:
        """
        Release the native engine handle.
        释放原生引擎句柄。
        """

        if self.closed:
            return None
        result = self.ffi.call_json("luaskills_ffi_engine_free_json", {"engine_id": self.engine_id})
        self.closed = True
        return result

    def _call(self, function_name: str, payload: dict[str, Any]) -> Any:
        """
        Call one JSON FFI function after checking the engine handle state.
        检查引擎句柄状态后调用一个 JSON FFI 函数。
        """

        if self.closed:
            raise RuntimeError(f"LuaSkills engine {self.engine_id} is already closed")
        return self.ffi.call_json(function_name, payload)


class SkillConfigClient:
    """
    Skill-config namespace backed by the unified runtime config store.
    基于统一运行时配置存储的 skill 配置命名空间。
    """

    def __init__(self, client: LuaSkillsClient) -> None:
        """
        Create one skill-config namespace for a parent SDK client.
        为父级 SDK 客户端创建一个 skill 配置命名空间。
        """

        self.client = client

    def list(self, skill_id: str | None = None) -> list[dict[str, Any]]:
        """
        List flattened config records, optionally limited to one skill id.
        列出扁平化配置记录，并可选限制到单个 skill id。
        """

        return self.client._call("luaskills_ffi_skill_config_list_json", {"engine_id": self.client.engine_id, "skill_id": skill_id})

    def get(self, skill_id: str, key: str) -> dict[str, Any]:
        """
        Get one config value by skill id and key.
        按 skill id 与 key 获取单个配置值。
        """

        return self.client._call("luaskills_ffi_skill_config_get_json", {"engine_id": self.client.engine_id, "skill_id": skill_id, "key": key})

    def set(self, skill_id: str, key: str, value: str) -> dict[str, Any]:
        """
        Set one config value by skill id and key.
        按 skill id 与 key 设置单个配置值。
        """

        return self.client._call("luaskills_ffi_skill_config_set_json", {
            "engine_id": self.client.engine_id,
            "skill_id": skill_id,
            "key": key,
            "value": value,
        })

    def delete(self, skill_id: str, key: str) -> dict[str, Any]:
        """
        Delete one config value by skill id and key.
        按 skill id 与 key 删除单个配置值。
        """

        return self.client._call("luaskills_ffi_skill_config_delete_json", {"engine_id": self.client.engine_id, "skill_id": skill_id, "key": key})


class SkillManagementClient:
    """
    Ordinary and system lifecycle namespace over JSON FFI management entrypoints.
    覆盖 JSON FFI 管理入口的普通与 system 生命周期命名空间。
    """

    def __init__(
        self,
        client: LuaSkillsClient,
        *,
        system_plane: bool,
        authority: Authority | str = Authority.SYSTEM,
    ) -> None:
        """
        Create one lifecycle namespace for a parent SDK client.
        为父级 SDK 客户端创建一个生命周期命名空间。
        """

        self.client = client
        self.system_plane = system_plane
        self.authority = authority

    def disable(self, skill_roots: list[RuntimeSkillRoot | dict[str, str]], skill_id: str, reason: str | None = None) -> dict[str, Any]:
        """
        Disable one skill through formal root-chain lifecycle state.
        通过正式 root 链生命周期状态停用单个 skill。
        """

        return self.client._call(self._function_name("disable_skill"), {
            "engine_id": self.client.engine_id,
            "skill_roots": roots_to_json(skill_roots),
            "skill_id": skill_id,
            "reason": reason,
            **self._authority_payload(),
        })

    def enable(self, skill_roots: list[RuntimeSkillRoot | dict[str, str]], skill_id: str) -> dict[str, Any]:
        """
        Enable one skill through formal root-chain lifecycle state.
        通过正式 root 链生命周期状态启用单个 skill。
        """

        return self.client._call(self._function_name("enable_skill"), {
            "engine_id": self.client.engine_id,
            "skill_roots": roots_to_json(skill_roots),
            "skill_id": skill_id,
            **self._authority_payload(),
        })

    def install(
        self,
        skill_roots: list[RuntimeSkillRoot | dict[str, str]],
        request: dict[str, Any],
        *,
        target_root: RuntimeSkillRoot | dict[str, str] | None = None,
        authority: Authority | str | None = None,
    ) -> dict[str, Any]:
        """
        Install one managed skill through the current lifecycle namespace.
        通过当前生命周期命名空间安装单个受管 skill。
        """

        return self._apply("install_skill", skill_roots, request, target_root=target_root, authority=authority)

    def update(
        self,
        skill_roots: list[RuntimeSkillRoot | dict[str, str]],
        request: dict[str, Any],
        *,
        target_root: RuntimeSkillRoot | dict[str, str] | None = None,
        authority: Authority | str | None = None,
    ) -> dict[str, Any]:
        """
        Update one managed skill through the current lifecycle namespace.
        通过当前生命周期命名空间更新单个受管 skill。
        """

        return self._apply("update_skill", skill_roots, request, target_root=target_root, authority=authority)

    def uninstall(
        self,
        skill_roots: list[RuntimeSkillRoot | dict[str, str]],
        skill_id: str,
        *,
        options: dict[str, Any] | None = None,
        target_root: RuntimeSkillRoot | dict[str, str] | None = None,
        authority: Authority | str | None = None,
    ) -> dict[str, Any]:
        """
        Uninstall one skill and optionally clean its databases.
        卸载单个 skill，并可选清理其数据库。
        """

        return self.client._call(self._function_name("uninstall_skill"), {
            "engine_id": self.client.engine_id,
            "skill_roots": roots_to_json(skill_roots),
            "skill_id": skill_id,
            "options": options or {},
            "target_root": root_to_json(target_root),
            **self._authority_payload(authority),
        })

    def _apply(
        self,
        action_name: str,
        skill_roots: list[RuntimeSkillRoot | dict[str, str]],
        request: dict[str, Any],
        *,
        target_root: RuntimeSkillRoot | dict[str, str] | None,
        authority: Authority | str | None,
    ) -> dict[str, Any]:
        """
        Execute one install or update JSON FFI action.
        执行单个 install 或 update JSON FFI 动作。
        """

        return self.client._call(self._function_name(action_name), {
            "engine_id": self.client.engine_id,
            "skill_roots": roots_to_json(skill_roots),
            "request": request,
            "target_root": root_to_json(target_root),
            **self._authority_payload(authority),
        })

    def _function_name(self, base_name: str) -> str:
        """
        Build the concrete JSON FFI function name for the current namespace.
        为当前命名空间构造具体 JSON FFI 函数名称。
        """

        prefix = "system_" if self.system_plane else ""
        return f"luaskills_ffi_{prefix}{base_name}_json"

    def _authority_payload(self, authority: Authority | str | None = None) -> dict[str, str]:
        """
        Build the authority payload required by system JSON FFI entrypoints.
        构造 system JSON FFI 入口要求的权限载荷。
        """

        if not self.system_plane:
            return {}
        return {"authority": authority_value(authority or self.authority)}


class SystemSkillManagementClient(SkillManagementClient):
    """
    System lifecycle namespace with host-injected authority.
    携带宿主注入权限的 system 生命周期命名空间。
    """

    def __init__(self, client: LuaSkillsClient, authority: Authority | str) -> None:
        """
        Create one system lifecycle namespace for a parent SDK client.
        为父级 SDK 客户端创建一个 system 生命周期命名空间。
        """

        super().__init__(client, system_plane=True, authority=authority)


def create_engine_options(
    runtime_root: str | os.PathLike[str],
    *,
    host_options: dict[str, Any] | None = None,
    pool_config: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """
    Build complete engine options from SDK defaults and caller overrides.
    基于 SDK 默认值和调用方覆盖构造完整引擎选项。
    """

    return {
        "pool_config": {**default_pool_config(), **(pool_config or {})},
        "host_options": merge_host_options(default_host_options(runtime_root), host_options or {}),
    }


def default_pool_config() -> dict[str, int]:
    """
    Return the SDK default VM pool configuration.
    返回 SDK 默认虚拟机池配置。
    """

    return {"min_size": 1, "max_size": 4, "idle_ttl_secs": 60}


def default_host_options(runtime_root: str | os.PathLike[str]) -> dict[str, Any]:
    """
    Return the SDK default host options for one runtime root.
    返回单个 runtime root 对应的 SDK 默认宿主选项。
    """

    root = Path(runtime_root).expanduser().resolve()
    base_options = {
        "temp_dir": normalized_path(root / "temp"),
        "resources_dir": normalized_path(root / "resources"),
        "lua_packages_dir": normalized_path(root / "lua_packages"),
        "host_provided_tool_root": normalized_path(root / "bin" / "tools"),
        "host_provided_lua_root": normalized_path(root / "lua_packages"),
        "host_provided_ffi_root": normalized_path(root / "libs"),
        "download_cache_root": normalized_path(root / "temp" / "downloads"),
        "dependency_dir_name": "dependencies",
        "state_dir_name": "state",
        "database_dir_name": "databases",
        "skill_config_file_path": None,
        "allow_network_download": True,
        "github_base_url": None,
        "github_api_base_url": None,
        "sqlite_library_path": None,
        "sqlite_provider_mode": "dynamic_library",
        "sqlite_callback_mode": "standard",
        "lancedb_library_path": None,
        "lancedb_provider_mode": "dynamic_library",
        "lancedb_callback_mode": "standard",
        "space_controller": default_space_controller_options(),
        "cache_config": None,
        "runlua_pool_config": None,
        "reserved_entry_names": [],
        "ignored_skill_ids": [],
        "capabilities": {"enable_skill_management_bridge": False},
    }
    manifest = load_runtime_install_manifest(root)
    return merge_host_options(base_options, host_options_from_runtime_manifest(manifest)) if manifest else base_options


def default_space_controller_options() -> dict[str, Any]:
    """
    Return the SDK default space-controller options.
    返回 SDK 默认 space-controller 选项。
    """

    return {
        "endpoint": None,
        "auto_spawn": False,
        "executable_path": None,
        "process_mode": "managed",
        "minimum_uptime_secs": 300,
        "idle_timeout_secs": 900,
        "default_lease_ttl_secs": 120,
        "connect_timeout_secs": 5,
        "startup_timeout_secs": 15,
        "startup_retry_interval_ms": 250,
        "lease_renew_interval_secs": 30,
    }


def merge_host_options(base: dict[str, Any], overrides: dict[str, Any]) -> dict[str, Any]:
    """
    Merge caller-provided host overrides over one complete host option object.
    将调用方提供的宿主覆盖合并到一个完整宿主选项对象上。
    """

    merged = {**base, **overrides}
    if "space_controller" in overrides:
        merged["space_controller"] = {**base["space_controller"], **overrides["space_controller"]}
    if "capabilities" in overrides:
        merged["capabilities"] = {**base["capabilities"], **overrides["capabilities"]}
    return merged


def invocation_context_to_json(context: LuaInvocationContext | dict[str, Any] | None) -> dict[str, Any] | None:
    """
    Convert an optional invocation context into a JSON FFI object.
    将可选调用上下文转换为 JSON FFI 对象。
    """

    if context is None:
        return None
    if isinstance(context, LuaInvocationContext):
        return context.to_json()
    return {
        "request_context": context.get("request_context"),
        "client_budget": context.get("client_budget") or {},
        "tool_config": context.get("tool_config") or {},
    }


def root_to_json(root: RuntimeSkillRoot | dict[str, str] | None) -> dict[str, str] | None:
    """
    Convert an optional runtime root value into one JSON FFI object.
    将可选运行时 root 值转换为 JSON FFI 对象。
    """

    if root is None:
        return None
    return root.to_json() if isinstance(root, RuntimeSkillRoot) else root

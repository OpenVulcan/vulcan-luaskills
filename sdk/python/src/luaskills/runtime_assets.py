"""
Runtime asset planning and installation helpers for the Python LuaSkills SDK.
Python LuaSkills SDK 的运行时资产规划与安装辅助工具。
"""

from __future__ import annotations

import hashlib
import json
import os
import platform
import shutil
import tarfile
import tempfile
import urllib.request
import zipfile
from enum import Enum
from pathlib import Path
from typing import Any, Callable

from .roots import normalized_path

DEFAULT_LUASKILLS_VERSION = "v0.2.2"
"""
Default LuaSkills release tag used by SDK runtime installation.
SDK 运行时安装使用的默认 LuaSkills 发布标签。
"""

DEFAULT_VLDB_CONTROLLER_VERSION = "v0.2.1"
"""
Default vldb-controller release tag used by SDK runtime installation.
SDK 运行时安装使用的默认 vldb-controller 发布标签。
"""

DEFAULT_VLDB_SQLITE_VERSION = "v0.1.5"
"""
Default vldb-sqlite release tag used by SDK runtime installation.
SDK 运行时安装使用的默认 vldb-sqlite 发布标签。
"""

DEFAULT_VLDB_LANCEDB_VERSION = "v0.1.5"
"""
Default vldb-lancedb release tag used by SDK runtime installation.
SDK 运行时安装使用的默认 vldb-lancedb 发布标签。
"""

RUNTIME_MANIFEST_FILE_NAME = "luaskills-sdk-runtime-manifest.json"
"""
Manifest file name written into the runtime resources directory.
写入 runtime resources 目录的清单文件名。
"""


class RuntimeDatabasePreset(str, Enum):
    """
    Database integration preset selected by SDK users.
    SDK 用户选择的数据库集成预设。
    """

    NONE = "none"
    """
    Do not install or configure database providers.
    不安装也不配置数据库 provider。
    """

    VLDB_CONTROLLER = "vldb-controller"
    """
    Use vldb-controller through space_controller mode.
    通过 space_controller 模式使用 vldb-controller。
    """

    VLDB_DIRECT = "vldb-direct"
    """
    Use vldb-sqlite-lib and vldb-lancedb-lib dynamic libraries directly.
    直接使用 vldb-sqlite-lib 与 vldb-lancedb-lib 动态库。
    """

    HOST_CALLBACK = "host-callback"
    """
    Let the host provide JSON callbacks instead of native VLDB assets.
    由宿主提供 JSON callback，而不是安装原生 VLDB 资产。
    """


def resolve_runtime_platform_target(system: str | None = None, machine: str | None = None) -> dict[str, str]:
    """
    Return the runtime platform target for the current Python process.
    返回当前 Python 进程对应的运行时平台目标。
    """

    os_name = (system or platform.system()).lower()
    arch_name = normalize_arch(machine or platform.machine())
    if os_name == "windows" and arch_name == "x86_64":
        return {
            "platform_key": "windows-x64",
            "target_triple": "x86_64-pc-windows-msvc",
            "archive_ext": ".zip",
            "controller_binary_name": "vldb-controller.exe",
            "dynamic_library_ext": ".dll",
            "luaskills_library_name": "luaskills.dll",
            "sqlite_library_name": "vldb_sqlite.dll",
            "lancedb_library_name": "vldb_lancedb.dll",
        }
    if os_name == "darwin" and arch_name in {"x86_64", "aarch64"}:
        return darwin_target(arch_name, "macos-x64" if arch_name == "x86_64" else "macos-arm64")
    if os_name == "linux" and arch_name in {"x86_64", "aarch64"}:
        return linux_target(arch_name, "linux-x64" if arch_name == "x86_64" else "linux-arm64")
    raise ValueError(f"unsupported runtime platform: {os_name}/{arch_name}")


def build_runtime_install_manifest(
    *,
    runtime_root: str | os.PathLike[str],
    database: RuntimeDatabasePreset | str = RuntimeDatabasePreset.NONE,
    luaskills_version: str = DEFAULT_LUASKILLS_VERSION,
    vldb_controller_version: str = DEFAULT_VLDB_CONTROLLER_VERSION,
    vldb_sqlite_version: str = DEFAULT_VLDB_SQLITE_VERSION,
    vldb_lancedb_version: str = DEFAULT_VLDB_LANCEDB_VERSION,
    include_luaskills_ffi: bool = True,
    luaskills_repo: str = "LuaSkills/luaskills",
    vldb_controller_repo: str = "OpenVulcan/vldb-controller",
    vldb_sqlite_repo: str = "OpenVulcan/vldb-sqlite",
    vldb_lancedb_repo: str = "OpenVulcan/vldb-lancedb",
) -> dict[str, Any]:
    """
    Build one deterministic runtime installation manifest.
    构造一个确定性的运行时安装清单。
    """

    resolved_root = Path(runtime_root).expanduser().resolve()
    preset = normalize_database_preset(database)
    target = resolve_runtime_platform_target()
    assets = build_runtime_asset_descriptors(
        target=target,
        database=preset,
        luaskills_version=luaskills_version,
        vldb_controller_version=vldb_controller_version,
        vldb_sqlite_version=vldb_sqlite_version,
        vldb_lancedb_version=vldb_lancedb_version,
        include_luaskills_ffi=include_luaskills_ffi,
        luaskills_repo=luaskills_repo,
        vldb_controller_repo=vldb_controller_repo,
        vldb_sqlite_repo=vldb_sqlite_repo,
        vldb_lancedb_repo=vldb_lancedb_repo,
    )
    return {
        "schema_version": 1,
        "generated_at": utc_now_iso(),
        "runtime_root": normalized_path(resolved_root),
        "database_mode": preset.value,
        "platform": target,
        "assets": assets,
        "host_options_patch": build_host_options_patch(resolved_root, preset, target, assets),
    }


def install_runtime_assets(**options: Any) -> dict[str, Any]:
    """
    Install native runtime assets and write the shared manifest.
    安装原生运行时资产并写入共享清单。
    """

    manifest = build_runtime_install_manifest(**options)
    runtime_root = Path(manifest["runtime_root"])
    ensure_runtime_directories(runtime_root)
    with tempfile.TemporaryDirectory(prefix="luaskills-runtime-assets-") as temporary_root:
        for asset in manifest["assets"]:
            install_one_asset(runtime_root, asset, Path(temporary_root), manifest["platform"])
    manifest["host_options_patch"] = build_host_options_patch(runtime_root, normalize_database_preset(manifest["database_mode"]), manifest["platform"], manifest["assets"])
    write_runtime_install_manifest(manifest)
    return manifest


def write_runtime_install_manifest(manifest: dict[str, Any]) -> Path:
    """
    Write one runtime install manifest into the runtime resources directory.
    将单个运行时安装清单写入 runtime resources 目录。
    """

    manifest_path = runtime_manifest_path(manifest["runtime_root"])
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return manifest_path


def load_runtime_install_manifest(runtime_root: str | os.PathLike[str]) -> dict[str, Any] | None:
    """
    Load one runtime install manifest from the runtime resources directory.
    从 runtime resources 目录加载单个运行时安装清单。
    """

    manifest_path = runtime_manifest_path(runtime_root)
    if not manifest_path.exists():
        return None
    return json.loads(manifest_path.read_text(encoding="utf-8"))


def runtime_manifest_path(runtime_root: str | os.PathLike[str]) -> Path:
    """
    Return the absolute runtime manifest path for one runtime root.
    返回单个 runtime root 对应的绝对运行时清单路径。
    """

    return Path(runtime_root).expanduser().resolve() / "resources" / RUNTIME_MANIFEST_FILE_NAME


def host_options_from_runtime_manifest(manifest: dict[str, Any]) -> dict[str, Any]:
    """
    Convert one runtime manifest into host option overrides.
    将单个运行时清单转换为宿主选项覆盖。
    """

    return dict(manifest.get("host_options_patch") or {})


def resolve_luaskills_library_path_from_runtime(runtime_root: str | os.PathLike[str], target: dict[str, str] | None = None) -> Path | None:
    """
    Resolve an installed LuaSkills dynamic library from one runtime root.
    从单个 runtime root 解析已安装的 LuaSkills 动态库。
    """

    resolved_target = target or resolve_runtime_platform_target()
    libs_dir = Path(runtime_root).expanduser().resolve() / "libs"
    for candidate in luaskills_library_candidates(resolved_target):
        candidate_path = libs_dir / candidate
        if candidate_path.exists():
            return candidate_path
    return None


def normalize_database_preset(value: RuntimeDatabasePreset | str) -> RuntimeDatabasePreset:
    """
    Normalize one database preset string.
    归一化单个数据库预设字符串。
    """

    try:
        return value if isinstance(value, RuntimeDatabasePreset) else RuntimeDatabasePreset(value)
    except ValueError as error:
        raise ValueError(f"unsupported database preset: {value}") from error


def normalize_arch(value: str) -> str:
    """
    Normalize one architecture string to release asset naming.
    将单个架构字符串归一化为发布资产命名。
    """

    lowered = value.lower()
    if lowered in {"x86_64", "amd64", "x64"}:
        return "x86_64"
    if lowered in {"aarch64", "arm64"}:
        return "aarch64"
    raise ValueError(f"unsupported architecture: {value}")


def darwin_target(arch_name: str, platform_key: str) -> dict[str, str]:
    """
    Build one macOS runtime platform descriptor.
    构造单个 macOS 运行时平台描述。
    """

    return {
        "platform_key": platform_key,
        "target_triple": f"{arch_name}-apple-darwin",
        "archive_ext": ".tar.gz",
        "controller_binary_name": "vldb-controller",
        "dynamic_library_ext": ".dylib",
        "luaskills_library_name": "libluaskills.dylib",
        "sqlite_library_name": "libvldb_sqlite.dylib",
        "lancedb_library_name": "libvldb_lancedb.dylib",
    }


def linux_target(arch_name: str, platform_key: str) -> dict[str, str]:
    """
    Build one Linux runtime platform descriptor.
    构造单个 Linux 运行时平台描述。
    """

    return {
        "platform_key": platform_key,
        "target_triple": f"{arch_name}-unknown-linux-gnu",
        "archive_ext": ".tar.gz",
        "controller_binary_name": "vldb-controller",
        "dynamic_library_ext": ".so",
        "luaskills_library_name": "libluaskills.so",
        "sqlite_library_name": "libvldb_sqlite.so",
        "lancedb_library_name": "libvldb_lancedb.so",
    }


def build_runtime_asset_descriptors(
    *,
    target: dict[str, str],
    database: RuntimeDatabasePreset,
    luaskills_version: str,
    vldb_controller_version: str,
    vldb_sqlite_version: str,
    vldb_lancedb_version: str,
    include_luaskills_ffi: bool,
    luaskills_repo: str,
    vldb_controller_repo: str,
    vldb_sqlite_repo: str,
    vldb_lancedb_repo: str,
) -> list[dict[str, Any]]:
    """
    Build every asset descriptor required by one manifest.
    构造单个清单所需的全部资产描述。
    """

    assets: list[dict[str, Any]] = []
    if include_luaskills_ffi:
        asset_name = f"luaskills-ffi-sdk-{target['platform_key']}.tar.gz"
        assets.append(release_asset("luaskills_ffi", luaskills_repo, luaskills_version, asset_name, f"libs/{target['luaskills_library_name']}"))
    if database == RuntimeDatabasePreset.VLDB_CONTROLLER:
        asset_name = f"vldb-controller-{vldb_controller_version}-{target['target_triple']}{target['archive_ext']}"
        assets.append(release_asset("vldb_controller", vldb_controller_repo, vldb_controller_version, asset_name, f"bin/{target['controller_binary_name']}"))
    if database == RuntimeDatabasePreset.VLDB_DIRECT:
        sqlite_asset = f"vldb-sqlite-lib-{vldb_sqlite_version}-{target['target_triple']}{target['archive_ext']}"
        lancedb_asset = f"vldb-lancedb-lib-{vldb_lancedb_version}-{target['target_triple']}{target['archive_ext']}"
        assets.append(release_asset("vldb_sqlite_lib", vldb_sqlite_repo, vldb_sqlite_version, sqlite_asset, f"libs/{target['sqlite_library_name']}"))
        assets.append(release_asset("vldb_lancedb_lib", vldb_lancedb_repo, vldb_lancedb_version, lancedb_asset, f"libs/{target['lancedb_library_name']}"))
    return assets


def release_asset(role: str, repository: str, version: str, asset_name: str, installed_path: str | None) -> dict[str, Any]:
    """
    Build one release asset descriptor from exact naming inputs.
    从精确命名输入构造单个发布资产描述。
    """

    base_url = f"https://github.com/{repository}/releases/download/{version}/{asset_name}"
    return {
        "role": role,
        "repository": repository,
        "version": version,
        "asset_name": asset_name,
        "sha256_asset_name": f"{asset_name}.sha256",
        "download_url": base_url,
        "sha256_url": f"{base_url}.sha256",
        "installed_path": installed_path,
    }


def build_host_options_patch(runtime_root: str | os.PathLike[str], database: RuntimeDatabasePreset, target: dict[str, str], assets: list[dict[str, Any]]) -> dict[str, Any]:
    """
    Build host option overrides for one database mode.
    为单个数据库模式构造宿主选项覆盖。
    """

    root = Path(runtime_root).expanduser().resolve()
    if database == RuntimeDatabasePreset.HOST_CALLBACK:
        return {
            "sqlite_provider_mode": "host_callback",
            "sqlite_callback_mode": "json",
            "lancedb_provider_mode": "host_callback",
            "lancedb_callback_mode": "json",
        }
    if database == RuntimeDatabasePreset.VLDB_CONTROLLER:
        return {
            "sqlite_provider_mode": "space_controller",
            "lancedb_provider_mode": "space_controller",
            "space_controller": {
                "endpoint": None,
                "auto_spawn": True,
                "executable_path": normalized_path(root / "bin" / target["controller_binary_name"]),
                "process_mode": "managed",
                "minimum_uptime_secs": 300,
                "idle_timeout_secs": 900,
                "default_lease_ttl_secs": 120,
                "connect_timeout_secs": 5,
                "startup_timeout_secs": 15,
                "startup_retry_interval_ms": 250,
                "lease_renew_interval_secs": 30,
            },
        }
    if database == RuntimeDatabasePreset.VLDB_DIRECT:
        return {
            "sqlite_library_path": resolve_installed_asset(root, assets, "vldb_sqlite_lib"),
            "sqlite_provider_mode": "dynamic_library",
            "lancedb_library_path": resolve_installed_asset(root, assets, "vldb_lancedb_lib"),
            "lancedb_provider_mode": "dynamic_library",
        }
    return {}


def luaskills_library_candidates(target: dict[str, str]) -> list[str]:
    """
    Return candidate LuaSkills dynamic library names for one platform.
    返回单个平台对应的 LuaSkills 动态库候选名称。
    """

    names = [target["luaskills_library_name"]]
    dynamic_ext = target["dynamic_library_ext"]
    if dynamic_ext == ".dll":
        names.append("libluaskills.dll")
    elif dynamic_ext == ".dylib":
        names.append("luaskills.dylib")
    else:
        names.append("luaskills.so")
    return list(dict.fromkeys(names))


def resolve_installed_asset(runtime_root: Path, assets: list[dict[str, Any]], role: str) -> str | None:
    """
    Resolve the absolute path for one installed asset role.
    解析单个已安装资产角色对应的绝对路径。
    """

    for asset in assets:
        if asset["role"] == role and asset.get("installed_path"):
            return normalized_path(runtime_root / asset["installed_path"])
    return None


def ensure_runtime_directories(runtime_root: Path) -> None:
    """
    Ensure runtime directories used by SDK-managed assets exist.
    确保 SDK 管理资产使用的 runtime 目录存在。
    """

    for directory_name in ["bin", "libs", "include", "licenses", "resources"]:
        (runtime_root / directory_name).mkdir(parents=True, exist_ok=True)


def install_one_asset(runtime_root: Path, asset: dict[str, Any], temporary_root: Path, target: dict[str, str]) -> None:
    """
    Download, verify, extract, and install one asset.
    下载、校验、解压并安装单个资产。
    """

    asset_directory = temporary_root / asset["role"]
    archive_path = asset_directory / asset["asset_name"]
    extract_directory = asset_directory / "extract"
    asset_directory.mkdir(parents=True, exist_ok=True)
    sha256_text = download_text(asset["sha256_url"])
    urllib.request.urlretrieve(asset["download_url"], archive_path)
    verify_sha256(archive_path, sha256_text)
    extract_archive(archive_path, extract_directory)
    if asset["role"] == "luaskills_ffi":
        install_luaskills_ffi(runtime_root, extract_directory, target, asset)
    elif asset["role"] == "vldb_controller":
        install_controller(runtime_root, extract_directory, target, asset)
    elif asset["role"] == "vldb_sqlite_lib":
        install_dynamic_library(runtime_root, extract_directory, target, "sqlite", asset)
    elif asset["role"] == "vldb_lancedb_lib":
        install_dynamic_library(runtime_root, extract_directory, target, "lancedb", asset)


def download_text(url: str) -> str:
    """
    Download one UTF-8 text file.
    下载单个 UTF-8 文本文件。
    """

    with urllib.request.urlopen(url) as response:
        return response.read().decode("utf-8")


def verify_sha256(file_path: Path, sha256_text: str) -> None:
    """
    Verify one downloaded archive against a SHA-256 sidecar.
    使用 SHA-256 旁路文件校验单个已下载归档。
    """

    expected_hash = sha256_text.strip().split()[0].lower()
    actual_hash = file_sha256(file_path)
    if expected_hash != actual_hash:
        raise ValueError(f"SHA-256 mismatch for {file_path}: expected {expected_hash}, got {actual_hash}")


def file_sha256(file_path: Path) -> str:
    """
    Compute the SHA-256 hash for one file.
    计算单个文件的 SHA-256 哈希。
    """

    digest = hashlib.sha256()
    with file_path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def extract_archive(archive_path: Path, destination: Path) -> None:
    """
    Extract one .zip or .tar.gz archive.
    解压单个 .zip 或 .tar.gz 归档。
    """

    destination.mkdir(parents=True, exist_ok=True)
    if archive_path.name.endswith(".zip"):
        with zipfile.ZipFile(archive_path) as archive:
            validate_zip_members(destination, archive)
            archive.extractall(destination)
        return
    with tarfile.open(archive_path) as archive:
        validate_tar_members(destination, archive)
        archive.extractall(destination)


def validate_zip_members(destination: Path, archive: zipfile.ZipFile) -> None:
    """
    Validate that every zip member extracts inside the destination directory.
    校验每个 zip 成员都会解压到目标目录内部。
    """

    for member in archive.infolist():
        validate_archive_member_path(destination, member.filename)


def validate_tar_members(destination: Path, archive: tarfile.TarFile) -> None:
    """
    Validate that every tar member and link target stays inside the destination directory.
    校验每个 tar 成员及其链接目标都保持在目标目录内部。
    """

    for member in archive.getmembers():
        validate_archive_member_path(destination, member.name)
        if member.issym():
            validate_archive_symlink_target(destination, member.name, member.linkname)
        if member.islnk():
            validate_archive_member_path(destination, member.linkname)


def validate_archive_symlink_target(destination: Path, member_name: str, link_name: str) -> None:
    """
    Validate that one archive symbolic link target resolves inside the destination directory.
    校验单个归档符号链接目标会解析到目标目录内部。
    """

    link_path = Path(link_name)
    if link_path.is_absolute():
        validate_archive_member_path(destination, link_name)
        return
    validate_archive_member_path(destination, str(Path(member_name).parent / link_path))


def validate_archive_member_path(destination: Path, member_name: str) -> None:
    """
    Reject archive members whose resolved extraction path escapes the destination.
    拒绝解析后解压路径逃逸目标目录的归档成员。
    """

    resolved_destination = destination.resolve()
    resolved_member_path = (resolved_destination / member_name).resolve()
    if resolved_member_path != resolved_destination and resolved_destination not in resolved_member_path.parents:
        raise ValueError(f"archive member escapes extraction directory: {member_name}")


def install_luaskills_ffi(runtime_root: Path, extract_directory: Path, target: dict[str, str], asset: dict[str, Any]) -> None:
    """
    Install a LuaSkills FFI SDK archive into runtime include/libs/licenses directories.
    将 LuaSkills FFI SDK 归档安装到 runtime include/libs/licenses 目录。
    """

    copy_directory_if_present(extract_directory / "include", runtime_root / "include")
    copy_directory_if_present(extract_directory / "lib", runtime_root / "libs")
    copy_directory_if_present(extract_directory / "licenses", runtime_root / "licenses" / "luaskills-ffi")
    installed_path = resolve_luaskills_library_path_from_runtime(runtime_root, target)
    if installed_path is None:
        raise FileNotFoundError(f"LuaSkills dynamic library was not found after installing {asset['asset_name']}")
    asset["installed_path"] = str(installed_path.relative_to(runtime_root)).replace("\\", "/")


def install_controller(runtime_root: Path, extract_directory: Path, target: dict[str, str], asset: dict[str, Any]) -> None:
    """
    Install vldb-controller into the runtime bin directory.
    将 vldb-controller 安装到 runtime bin 目录。
    """

    source = find_file(extract_directory, lambda name: name == target["controller_binary_name"])
    if source is None:
        raise FileNotFoundError(f"{target['controller_binary_name']} was not found in {asset['asset_name']}")
    destination = runtime_root / "bin" / target["controller_binary_name"]
    shutil.copy2(source, destination)
    destination.chmod(0o755)
    asset["installed_path"] = f"bin/{target['controller_binary_name']}"


def install_dynamic_library(runtime_root: Path, extract_directory: Path, target: dict[str, str], name_hint: str, asset: dict[str, Any]) -> None:
    """
    Install one VLDB dynamic library into the runtime libs directory.
    将单个 VLDB 动态库安装到 runtime libs 目录。
    """

    library_ext = target["dynamic_library_ext"]
    source = find_file(extract_directory, lambda name: name.endswith(library_ext) and name_hint in name.lower())
    if source is None:
        raise FileNotFoundError(f"dynamic library for {asset['role']} was not found in {asset['asset_name']}")
    destination = runtime_root / "libs" / source.name
    shutil.copy2(source, destination)
    asset["installed_path"] = f"libs/{source.name}"


def copy_directory_if_present(source: Path, destination: Path) -> None:
    """
    Copy one directory only when it exists.
    仅在目录存在时复制单个目录。
    """

    if source.exists():
        shutil.copytree(source, destination, dirs_exist_ok=True)


def find_file(root: Path, predicate: Callable[[str], bool]) -> Path | None:
    """
    Find one file under a directory by base-name predicate.
    根据基础文件名谓词在目录下查找单个文件。
    """

    for path in root.rglob("*"):
        if path.is_file() and predicate(path.name):
            return path
    return None


def utc_now_iso() -> str:
    """
    Return a compact UTC ISO timestamp.
    返回紧凑的 UTC ISO 时间戳。
    """

    from datetime import datetime, timezone

    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")

#!/usr/bin/env python3
"""Generate runtime-facing luaskills-packages metadata files.
生成面向运行时的 luaskills-packages 元数据文件。
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
from pathlib import Path
from typing import Any


SUPPORTED_TARGETS = [
    "windows-x64",
    "linux-x64",
    "linux-arm64",
    "macos-x64",
    "macos-arm64",
]


def write_text(path: Path, content: str) -> None:
    """Write one UTF-8 text file and create parent directories first.
    写入一个 UTF-8 文本文件，并在写入前创建父目录。
    """

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def write_json(path: Path, value: Any) -> None:
    """Write one JSON file with stable indentation.
    以稳定缩进格式写入一个 JSON 文件。
    """

    write_text(path, json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def copy_tree(source: Path, destination: Path) -> None:
    """Copy one directory tree into a fresh destination directory.
    将一个目录树复制到全新的目标目录。
    """

    if destination.exists():
        shutil.rmtree(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source, destination)


def parse_lua_packages(lua_packages_path: Path) -> list[dict[str, Any]]:
    """Parse one legacy lua_packages.txt file into structured package records.
    将一个 legacy lua_packages.txt 文件解析为结构化包记录。
    """

    packages: list[dict[str, Any]] = []
    current: dict[str, Any] | None = None
    for raw_line in lua_packages_path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("pkg "):
            tokens = line.split()
            current = {
                "rock_name": tokens[1],
                "version": tokens[2] if len(tokens) > 2 else None,
                "config": [],
            }
            packages.append(current)
            continue
        if raw_line.startswith("  ") and current is not None:
            tokens = line.split()
            current["config"].append(
                {
                    "kind": tokens[0],
                    "tokens": tokens[1:],
                    "raw": line,
                }
            )
    return packages


def load_packages_source(project_root: Path) -> dict[str, Any]:
    """Load one staged runtime-packages source description when it exists.
    在暂存的 runtime-packages 来源存在时加载该描述。
    """

    active_path = project_root / "target" / "runtime-packages" / "active.json"
    if active_path.exists():
        payload = json.loads(active_path.read_text(encoding="utf-8"))
        return {
            "source_repository": payload.get("repository", "LuaSkills/luaskills-packages"),
            "bundle_id": payload.get("bundle_id", "official"),
            "bundle_version": payload.get("bundle_version", normalize_tag(payload.get("resolved_tag", "v0.0.0"))),
            "series": payload.get("series", "0.0"),
            "generation_mode": payload.get("generation_mode", "release-bundle"),
            "bundle_root": Path(payload["bundle_root"]).resolve(),
            "dist_root": Path(payload["dist_root"]).resolve(),
            "compat_lua_packages": Path(payload["compat_lua_packages"]).resolve(),
            "resolved_tag": payload.get("resolved_tag", ""),
        }

    provenance_path = project_root / "scripts" / "lua_packages.generated-from.json"
    if provenance_path.exists():
        payload = json.loads(provenance_path.read_text(encoding="utf-8"))
        return {
            "source_repository": payload.get("source_repository", "LuaSkills/luaskills-packages"),
            "bundle_id": payload.get("bundle_id", "compat-generated"),
            "bundle_version": payload.get("bundle_version", "0.0.0-local"),
            "series": payload.get("series", derive_series(payload.get("bundle_version", "0.0.0-local"))),
            "generation_mode": payload.get("generation_mode", "compat-generated"),
            "bundle_root": None,
            "dist_root": None,
            "compat_lua_packages": project_root / "scripts" / "lua_packages.txt",
            "resolved_tag": payload.get("resolved_tag", ""),
        }

    return {
        "source_repository": "LuaSkills/luaskills-packages",
        "bundle_id": "compat-generated",
        "bundle_version": "0.0.0-local",
        "series": "compat",
        "generation_mode": "compat-generated",
        "bundle_root": None,
        "dist_root": None,
        "compat_lua_packages": project_root / "scripts" / "lua_packages.txt",
        "resolved_tag": "",
    }


def normalize_tag(tag: str) -> str:
    """Normalize one optional tag string to a semantic version-like value.
    将一个可选标签字符串规范化为类似语义化版本的值。
    """

    if tag.startswith("v"):
        return tag[1:]
    return tag or "0.0.0-local"


def derive_series(bundle_version: str) -> str:
    """Derive one major.minor series label from one semantic version string.
    从语义化版本字符串推导一个 major.minor 系列标识。
    """

    match = re.match(r"^(\d+\.\d+)", bundle_version)
    if match:
        return match.group(1)
    return "compat"


def copy_if_present(source: Path, destination: Path) -> bool:
    """Copy one file or directory when it exists and report whether it was copied.
    在文件或目录存在时复制，并返回是否发生了复制。
    """

    if not source.exists():
        return False
    if source.is_dir():
        copy_tree(source, destination)
    else:
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
    return True


def normalize_bundle_license_index(index_path: Path) -> None:
    """Rewrite bundle license index paths so they point at runtime layout locations.
    重写 bundle 授权索引路径，使其指向 runtime 布局中的位置。
    """

    if not index_path.exists():
        return
    payload = json.loads(index_path.read_text(encoding="utf-8"))
    for package_record in payload.get("packages", []):
        output_path = package_record.get("output_path", "")
        if output_path.startswith("dist/licenses/"):
            package_record["output_path"] = "licenses/luaskills-packages/" + output_path.removeprefix("dist/licenses/")
    write_json(index_path, payload)


def parse_luarocks_license_manifest(manifest_path: Path) -> list[dict[str, str]]:
    """Parse the runtime LuaRocks TSV manifest into structured rows.
    将运行时 LuaRocks TSV 清单解析为结构化行记录。
    """

    if not manifest_path.exists():
        return []

    rows: list[dict[str, str]] = []
    for line in manifest_path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        parts = line.split("\t")
        parts.extend([""] * (5 - len(parts)))
        rows.append(
            {
                "name": parts[0],
                "version": parts[1],
                "license": parts[2],
                "source": parts[3],
                "homepage": parts[4],
                "license_root": f"licenses/luarocks/{parts[0]}",
            }
        )
    return rows


def parse_native_license_manifest(manifest_path: Path) -> list[dict[str, Any]]:
    """Parse the runtime native-license manifest and keep only third-party components.
    解析运行时原生授权清单，并仅保留第三方组件。
    """

    if not manifest_path.exists():
        return []
    payload = json.loads(manifest_path.read_text(encoding="utf-8"))
    components = payload.get("components", [])
    filtered: list[dict[str, Any]] = []
    for item in components:
        if item.get("name") == "luaskills":
            continue
        filtered.append(item)
    return filtered


def build_package_help_entries(
    packages: list[dict[str, Any]],
    help_packages_root: Path,
) -> list[dict[str, str]]:
    """Generate one minimal package-help document for every package record.
    为每个包记录生成一个最小包帮助文档。
    """

    entries: list[dict[str, str]] = []
    for package in packages:
        file_name = f"{package['rock_name']}.json"
        help_path = help_packages_root / file_name
        summary_en = "Runtime package metadata imported from lua_packages.txt."
        summary_zh = "从 lua_packages.txt 导入的运行时包元数据。"
        payload = {
            "schema_version": 1,
            "help_kind": "package",
            "package_name": package["rock_name"],
            "version": package["version"],
            "summary_en": summary_en,
            "summary_zh": summary_zh,
            "config_kinds": [item["kind"] for item in package.get("config", [])],
            "config": package.get("config", []),
        }
        write_json(help_path, payload)
        entries.append(
            {
                "name": package["rock_name"],
                "version": package["version"] or "",
                "help_path": f"resources/luaskills-packages/help/packages/{file_name}",
            }
        )
    return entries


def build_third_party_notices(
    native_components: list[dict[str, Any]],
    luarocks_packages: list[dict[str, str]],
) -> str:
    """Render one Markdown notice summary for runtime-shipped third-party content.
    为运行时携带的第三方内容渲染一个 Markdown notice 摘要。
    """

    lines = [
        "# Third-Party Notices",
        "",
        "## Native Components",
        "",
    ]
    for component in native_components:
        lines.append(
            f"- `{component.get('name', '')}`: {component.get('license', '')}"
        )
    lines.extend(["", "## LuaRocks Packages", ""])
    for package in luarocks_packages:
        license_text = package["license"] or "unknown"
        source_text = package["source"] or ""
        lines.append(
            f"- `{package['name']}` {package['version']}: {license_text}"
            + (f" ({source_text})" if source_text else "")
        )
    lines.append("")
    return "\n".join(lines)


def generate_runtime_packages_metadata(
    project_root: Path,
    runtime_root: Path,
    platform: str,
) -> None:
    """Generate the runtime-facing packages metadata tree under one runtime root.
    在一个 runtime 根目录下生成面向运行时的 packages 元数据目录树。
    """

    resources_root = runtime_root / "resources"
    packages_root = resources_root / "luaskills-packages"
    help_root = packages_root / "help"
    help_packages_root = help_root / "packages"
    help_modules_root = help_root / "modules"
    licenses_root = runtime_root / "licenses"
    packages_license_root = licenses_root / "luaskills-packages"

    package_source = load_packages_source(project_root)
    lua_packages_path = package_source["compat_lua_packages"]
    packages = parse_lua_packages(lua_packages_path)
    bundle_version = package_source.get("bundle_version") or "0.0.0-local"

    compat_lua_packages_path = packages_root / "lua_packages.txt"
    write_text(compat_lua_packages_path, lua_packages_path.read_text(encoding="utf-8"))

    dist_root = package_source.get("dist_root")

    install_manifest_path = packages_root / "install-manifest.json"
    if not (dist_root and copy_if_present(dist_root / "install-manifest.json", install_manifest_path)):
        install_manifest = {
            "schema_version": 1,
            "platform": platform,
            "source": {
                "repository": package_source.get("source_repository", "LuaSkills/luaskills-packages"),
                "bundle_id": package_source.get("bundle_id", "compat-generated"),
                "bundle_version": bundle_version,
                "series": package_source.get("series", derive_series(bundle_version)),
            },
            "packages": packages,
        }
        write_json(install_manifest_path, install_manifest)

    help_index_path = help_root / "index.json"
    if dist_root and copy_if_present(dist_root / "help", help_root):
        if not help_index_path.exists():
            raise RuntimeError(f"runtime packages bundle is missing help/index.json under {dist_root}")
    else:
        package_help_entries = build_package_help_entries(packages, help_packages_root)
        help_modules_root.mkdir(parents=True, exist_ok=True)
        help_index = {
            "schema_version": 1,
            "packages": package_help_entries,
            "modules": [],
        }
        write_json(help_index_path, help_index)

    if not (dist_root and copy_if_present(dist_root / "platform-support.json", packages_root / "platform-support.json")):
        platform_support = {
            "schema_version": 1,
            "current_platform": platform,
            "supported_targets": SUPPORTED_TARGETS,
        }
        write_json(packages_root / "platform-support.json", platform_support)

    if not (
        dist_root
        and copy_if_present(
            dist_root / "THIRD_PARTY_LICENSES.json",
            packages_root / "THIRD_PARTY_LICENSES.json",
        )
    ):
        native_components = parse_native_license_manifest(licenses_root / "manifest.json")
        luarocks_packages = parse_luarocks_license_manifest(
            licenses_root / "luarocks" / "manifest.tsv"
        )
        third_party_licenses = {
            "schema_version": 1,
            "native_components": native_components,
            "luarocks_packages": luarocks_packages,
        }
        write_json(packages_root / "THIRD_PARTY_LICENSES.json", third_party_licenses)
    else:
        native_components = []
        luarocks_packages = []

    if not (
        dist_root
        and copy_if_present(
            dist_root / "THIRD_PARTY_NOTICES.md",
            packages_root / "THIRD_PARTY_NOTICES.md",
        )
    ):
        if not native_components and not luarocks_packages:
            native_components = parse_native_license_manifest(licenses_root / "manifest.json")
            luarocks_packages = parse_luarocks_license_manifest(
                licenses_root / "luarocks" / "manifest.tsv"
            )
        write_text(
            packages_root / "THIRD_PARTY_NOTICES.md",
            build_third_party_notices(native_components, luarocks_packages),
        )

    if dist_root and copy_if_present(dist_root / "licenses", packages_license_root):
        normalize_bundle_license_index(packages_license_root / "index.json")
    else:
        if not native_components and not luarocks_packages:
            native_components = parse_native_license_manifest(licenses_root / "manifest.json")
            luarocks_packages = parse_luarocks_license_manifest(
                licenses_root / "luarocks" / "manifest.tsv"
            )
        license_index = {
            "schema_version": 1,
            "native_components": [
                {
                    "name": item.get("name", ""),
                    "license": item.get("license", ""),
                    "license_files": item.get("license_files", []),
                }
                for item in native_components
            ],
            "luarocks_packages": luarocks_packages,
        }
        write_json(packages_license_root / "index.json", license_index)

    runtime_packages_manifest = {
        "schema_version": 1,
        "repository": package_source.get("source_repository", "LuaSkills/luaskills-packages"),
        "bundle_id": package_source.get("bundle_id", "compat-generated"),
        "bundle_version": bundle_version,
        "series": package_source.get("series", derive_series(bundle_version)),
        "resolved_tag": package_source.get("resolved_tag", ""),
        "platform": platform,
        "layout": "luaskills-packages-runtime-v1",
        "generation_mode": package_source.get("generation_mode", "compat-generated"),
        "paths": {
            "install_manifest": "resources/luaskills-packages/install-manifest.json",
            "compat_lua_packages_txt": "resources/luaskills-packages/lua_packages.txt",
            "platform_support": "resources/luaskills-packages/platform-support.json",
            "third_party_licenses": "resources/luaskills-packages/THIRD_PARTY_LICENSES.json",
            "third_party_notices": "resources/luaskills-packages/THIRD_PARTY_NOTICES.md",
            "help_index": "resources/luaskills-packages/help/index.json",
            "package_help_root": "resources/luaskills-packages/help/packages",
            "module_help_root": "resources/luaskills-packages/help/modules",
            "license_index": "licenses/luaskills-packages/index.json",
        },
    }
    write_json(resources_root / "luaskills-packages-manifest.json", runtime_packages_manifest)


def parse_args() -> argparse.Namespace:
    """Parse command line arguments for the metadata generator.
    解析元数据生成器的命令行参数。
    """

    parser = argparse.ArgumentParser()
    parser.add_argument("--project-root", required=True)
    parser.add_argument("--runtime-root", required=True)
    parser.add_argument("--platform", required=True)
    return parser.parse_args()


def main() -> int:
    """Run metadata generation and exit with one process status code.
    执行元数据生成，并返回进程状态码。
    """

    args = parse_args()
    generate_runtime_packages_metadata(
        project_root=Path(args.project_root).resolve(),
        runtime_root=Path(args.runtime_root).resolve(),
        platform=args.platform,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

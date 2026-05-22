#!/usr/bin/env python3
"""Fetch and stage one luaskills-packages release bundle for runtime assembly.
获取并暂存一个用于 runtime 组装的 luaskills-packages release bundle。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import sys
import urllib.error
import urllib.request
import zipfile
from pathlib import Path
from typing import Any


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


def read_json(path: Path) -> dict[str, Any]:
    """Read one JSON object file.
    读取一个 JSON 对象文件。
    """

    return json.loads(path.read_text(encoding="utf-8"))


def normalize_tag_to_version(tag: str) -> str:
    """Convert one Git tag like v0.1.3 into one semantic version string.
    将形如 v0.1.3 的 Git 标签转换为语义化版本字符串。
    """

    return tag[1:] if tag.startswith("v") else tag


def parse_semver(version_text: str) -> tuple[int, int, int]:
    """Parse one simple semantic version string into numeric parts.
    将一个简单语义化版本字符串解析为数字元组。
    """

    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", version_text)
    if not match:
        raise ValueError(f"unsupported semantic version: {version_text}")
    return tuple(int(group) for group in match.groups())


def request_json(url: str) -> Any:
    """Request one JSON payload from GitHub with optional token support.
    从 GitHub 请求一个 JSON 载荷，并可选携带令牌。
    """

    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "luaskills-runtime-packages-fetcher",
        },
    )
    token = os.environ.get("GITHUB_TOKEN", "").strip()
    if token:
        request.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.loads(response.read().decode("utf-8"))


def request_json_or_none(url: str) -> Any | None:
    """Request one JSON payload and return None for HTTP 404 responses.
    请求一个 JSON 载荷，并在遇到 HTTP 404 响应时返回 None。
    """

    try:
        return request_json(url)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return None
        raise


def download_bytes(url: str) -> bytes:
    """Download one binary asset with optional token support.
    使用可选令牌下载一个二进制资产。
    """

    request = urllib.request.Request(
        url,
        headers={"User-Agent": "luaskills-runtime-packages-fetcher"},
    )
    token = os.environ.get("GITHUB_TOKEN", "").strip()
    if token:
        request.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(request, timeout=120) as response:
        return response.read()


def load_binding_config(project_root: Path) -> dict[str, Any]:
    """Load the committed runtime-packages binding configuration.
    加载已提交的 runtime-packages 绑定配置。
    """

    binding_path = project_root / "scripts" / "runtime_packages_binding.json"
    binding = read_json(binding_path)
    if binding.get("schema_version") != 1:
        raise RuntimeError(f"unsupported runtime packages binding schema: {binding}")
    return binding


def resolve_binding(binding: dict[str, Any]) -> dict[str, str]:
    """Resolve repository, series, and optional explicit tag from config or env.
    从配置或环境变量解析仓库、协议线与可选精确标签。
    """

    repository = os.environ.get("LUASKILLS_PACKAGES_REPOSITORY", "").strip() or binding["repository"]
    series = os.environ.get("LUASKILLS_PACKAGES_SERIES", "").strip() or binding["series"]
    explicit_tag = os.environ.get("LUASKILLS_PACKAGES_TAG", "").strip()
    return {
        "repository": repository,
        "series": series,
        "explicit_tag": explicit_tag,
        "bundle_asset_template": binding["bundle_asset_template"],
        "bundle_sha256_template": binding["bundle_sha256_template"],
    }


def find_release_for_series(
    repository: str,
    series: str,
    bundle_asset_template: str,
    bundle_sha256_template: str,
) -> dict[str, Any]:
    """Resolve the latest usable GitHub release inside one compatible major.minor series.
    解析一个兼容 major.minor 协议线中最新且可用的 GitHub release。
    """

    releases_url = f"https://api.github.com/repos/{repository}/releases?per_page=100"
    payload = request_json(releases_url)
    matches: list[tuple[tuple[int, int, int], dict[str, Any]]] = []
    for release in payload:
        if release.get("draft") or release.get("prerelease"):
            continue
        tag_name = release.get("tag_name", "")
        version_text = normalize_tag_to_version(tag_name)
        try:
            version = parse_semver(version_text)
        except ValueError:
            continue
        if f"{version[0]}.{version[1]}" != series:
            continue
        matches.append((version, release))
    matches.sort(key=lambda item: item[0], reverse=True)
    if not matches:
        raise RuntimeError(
            f"no luaskills-packages release found for series {series} in {repository}"
        )

    for _, release in matches:
        tag_name = str(release.get("tag_name", ""))
        required_asset_names = (
            bundle_asset_template.format(tag=tag_name),
            bundle_sha256_template.format(tag=tag_name),
        )
        release_asset_names = {str(asset.get("name", "")) for asset in release.get("assets", [])}
        if all(asset_name in release_asset_names for asset_name in required_asset_names):
            return release

    raise RuntimeError(
        "no luaskills-packages release in series "
        f"{series} provides both {bundle_asset_template} and {bundle_sha256_template}"
    )


def find_release_by_tag(repository: str, tag: str) -> dict[str, Any]:
    """Resolve one exact GitHub release by tag.
    通过标签解析一个精确的 GitHub release。
    """

    release_url = f"https://api.github.com/repos/{repository}/releases/tags/{tag}"
    try:
        return request_json(release_url)
    except urllib.error.HTTPError as error:
        if error.code == 404 and tag_exists(repository, tag):
            raise RuntimeError(
                f"luaskills-packages tag {tag} exists but published release assets are not available yet"
            ) from error
        raise RuntimeError(
            f"failed to resolve luaskills-packages release tag {tag}: {error}"
        ) from error


def tag_exists(repository: str, tag: str) -> bool:
    """Return whether one Git tag exists in the target repository.
    返回目标仓库中是否存在某个 Git 标签。
    """

    ref_url = f"https://api.github.com/repos/{repository}/git/ref/tags/{tag}"
    return request_json_or_none(ref_url) is not None


def asset_download_url(release: dict[str, Any], asset_name: str) -> str:
    """Find one release asset download URL by exact asset name.
    按精确资产名查找一个 release 资产下载地址。
    """

    for asset in release.get("assets", []):
        if asset.get("name") == asset_name:
            return asset["browser_download_url"]
    raise RuntimeError(
        "luaskills-packages release "
        f"{release.get('tag_name')} is published but required asset {asset_name} is not available yet"
    )


def sha256_text_for_asset(url: str) -> str:
    """Download one sha256 sidecar file and extract the expected hash value.
    下载一个 sha256 辅助文件并提取期望哈希值。
    """

    text = download_bytes(url).decode("utf-8").strip()
    if not text:
        raise RuntimeError("empty sha256 asset payload")
    return text.split()[0].strip().lower()


def verify_sha256(data: bytes, expected_hash: str, asset_name: str) -> None:
    """Verify the sha256 digest for one downloaded bundle asset.
    校验一个已下载 bundle 资产的 sha256 摘要。
    """

    actual_hash = hashlib.sha256(data).hexdigest().lower()
    if actual_hash != expected_hash:
        raise RuntimeError(
            f"sha256 mismatch for {asset_name}: expected {expected_hash}, got {actual_hash}"
        )


def prepare_override_bundle_root(external_root: Path, project_root: Path) -> dict[str, Any]:
    """Prepare one externally provided bundle directory as the active source.
    将一个外部提供的 bundle 目录准备成活动来源。
    """

    dist_root = external_root / "dist" if (external_root / "dist" / "lua_packages.txt").exists() else external_root
    compat_path = dist_root / "lua_packages.txt"
    if not compat_path.exists():
        raise RuntimeError(f"bundle directory does not contain lua_packages.txt: {external_root}")
    bundle_version = os.environ.get("LUASKILLS_PACKAGES_TAG", "").strip() or "external"
    return {
        "schema_version": 1,
        "repository": os.environ.get("LUASKILLS_PACKAGES_REPOSITORY", "").strip() or "external",
        "series": os.environ.get("LUASKILLS_PACKAGES_SERIES", "").strip() or "external",
        "resolved_tag": bundle_version,
        "bundle_id": "official",
        "bundle_version": normalize_tag_to_version(bundle_version),
        "generation_mode": "external-bundle",
        "bundle_root": str(external_root.resolve()),
        "dist_root": str(dist_root.resolve()),
        "compat_lua_packages": str(compat_path.resolve()),
    }


def write_active_bundle_state(project_root: Path, active_state: dict[str, Any]) -> None:
    """Write the active bundle state and remove obsolete local compatibility metadata.
    写入活动 bundle 状态，并移除已废弃的本地兼容元数据。
    """

    active_path = project_root / "target" / "runtime-packages" / "active.json"
    write_json(active_path, active_state)
    legacy_generated_from_path = project_root / "scripts" / "lua_packages.generated-from.json"
    if legacy_generated_from_path.exists():
        legacy_generated_from_path.unlink()


def stage_release_bundle(project_root: Path, binding: dict[str, str], refresh: bool) -> dict[str, Any]:
    """Download or reuse one release bundle and return the staged active-state payload.
    下载或复用一个 release bundle，并返回暂存后的活动状态载荷。
    """

    explicit_tag = binding["explicit_tag"]
    if explicit_tag:
        release = find_release_by_tag(binding["repository"], explicit_tag)
    else:
        release = find_release_for_series(
            binding["repository"],
            binding["series"],
            binding["bundle_asset_template"],
            binding["bundle_sha256_template"],
        )

    resolved_tag = release["tag_name"]
    bundle_root = project_root / "target" / "runtime-packages" / "bundles" / resolved_tag
    dist_root = bundle_root / "dist"
    compat_path = dist_root / "lua_packages.txt"

    if not refresh and compat_path.exists():
        return {
            "schema_version": 1,
            "repository": binding["repository"],
            "series": binding["series"],
            "resolved_tag": resolved_tag,
            "bundle_id": "official",
            "bundle_version": normalize_tag_to_version(resolved_tag),
            "generation_mode": "release-bundle",
            "bundle_root": str(bundle_root.resolve()),
            "dist_root": str(dist_root.resolve()),
            "compat_lua_packages": str(compat_path.resolve()),
        }

    bundle_asset_name = binding["bundle_asset_template"].format(tag=resolved_tag)
    sha256_asset_name = binding["bundle_sha256_template"].format(tag=resolved_tag)
    bundle_url = asset_download_url(release, bundle_asset_name)
    sha256_url = asset_download_url(release, sha256_asset_name)

    bundle_bytes = download_bytes(bundle_url)
    expected_hash = sha256_text_for_asset(sha256_url)
    verify_sha256(bundle_bytes, expected_hash, bundle_asset_name)

    if bundle_root.exists():
        shutil.rmtree(bundle_root)
    dist_root.mkdir(parents=True, exist_ok=True)
    archive_path = bundle_root / bundle_asset_name
    archive_path.write_bytes(bundle_bytes)
    with zipfile.ZipFile(archive_path) as archive:
        archive.extractall(dist_root)
    archive_path.unlink(missing_ok=True)

    if not compat_path.exists():
        raise RuntimeError(f"downloaded bundle does not contain lua_packages.txt: {dist_root}")

    return {
        "schema_version": 1,
        "repository": binding["repository"],
        "series": binding["series"],
        "resolved_tag": resolved_tag,
        "bundle_id": "official",
        "bundle_version": normalize_tag_to_version(resolved_tag),
        "generation_mode": "release-bundle",
        "bundle_root": str(bundle_root.resolve()),
        "dist_root": str(dist_root.resolve()),
        "compat_lua_packages": str(compat_path.resolve()),
    }


def parse_args() -> argparse.Namespace:
    """Parse command-line arguments for bundle staging.
    解析 bundle 暂存流程的命令行参数。
    """

    parser = argparse.ArgumentParser()
    parser.add_argument("--project-root", required=True)
    parser.add_argument("--refresh", action="store_true")
    return parser.parse_args()


def main() -> int:
    """Resolve one runtime bundle source, stage it, and persist the active state.
    解析一个 runtime bundle 来源、完成暂存并持久化活动状态。
    """

    args = parse_args()
    project_root = Path(args.project_root).resolve()
    binding = resolve_binding(load_binding_config(project_root))

    external_bundle_dir = os.environ.get("LUASKILLS_PACKAGES_BUNDLE_DIR", "").strip()
    if external_bundle_dir:
        active_state = prepare_override_bundle_root(Path(external_bundle_dir), project_root)
    else:
        active_state = stage_release_bundle(project_root, binding, refresh=args.refresh)

    write_active_bundle_state(project_root, active_state)
    print(
        json.dumps(
            {
                "repository": active_state["repository"],
                "series": active_state["series"],
                "resolved_tag": active_state["resolved_tag"],
                "generation_mode": active_state["generation_mode"],
                "dist_root": active_state["dist_root"],
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

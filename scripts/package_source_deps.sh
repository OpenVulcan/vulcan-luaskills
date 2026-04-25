#!/usr/bin/env bash
set -euo pipefail

# ProjectRoot points at the repository root regardless of the caller location.
# ProjectRoot 指向仓库根目录，避免调用方当前位置影响路径解析。
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Version stores the source dependency package version.
# Version 保存源码依赖包版本。
VERSION="${1:-v0.1.0}"

# StagingDir stores package content before compression.
# StagingDir 保存压缩前的包内容。
STAGING_DIR="${STAGING_DIR:-target/source-deps-package}"

# OutputDir stores final release archives.
# OutputDir 保存最终发布压缩包。
OUTPUT_DIR="${OUTPUT_DIR:-target/release-packages}"

ensure_dir() {
  # Create one directory when it does not exist.
  # 在目录不存在时创建该目录。
  mkdir -p "$1"
}

create_tar_from_dir() {
  # Archive top-level children without adding a leading ./ entry.
  # 按一级子项打包，避免归档内出现 ./ 前缀。
  local source_dir="$1"
  local archive_path="$2"
  local members=()
  while IFS= read -r entry; do
    members+=("$(basename "$entry")")
  done < <(find "$source_dir" -mindepth 1 -maxdepth 1)
  if [ "${#members[@]}" -eq 0 ]; then
    echo "Cannot create archive from empty directory: $source_dir" >&2
    return 1
  fi
  tar -czf "$archive_path" -C "$source_dir" "${members[@]}"
}

copy_path_if_exists() {
  # Copy one path when it exists.
  # 在路径存在时复制该路径。
  local source="$1"
  local destination="$2"
  if [ -e "$source" ]; then
    cp -a "$source" "$destination"
  fi
}

cd "$PROJECT_ROOT"

PACKAGE_ROOT="$STAGING_DIR/luaskills-source-deps"
rm -rf "$PACKAGE_ROOT"
ensure_dir "$PACKAGE_ROOT/scripts"
ensure_dir "$PACKAGE_ROOT/licenses"
ensure_dir "$OUTPUT_DIR"

copy_path_if_exists "$PROJECT_ROOT/scripts/lua_packages.txt" "$PACKAGE_ROOT/lua_packages.txt"
copy_path_if_exists "$PROJECT_ROOT/scripts/luarocks_overrides" "$PACKAGE_ROOT/luarocks_overrides"
copy_path_if_exists "$PROJECT_ROOT/scripts/fetch_runtime_deps.ps1" "$PACKAGE_ROOT/scripts/fetch_runtime_deps.ps1"
copy_path_if_exists "$PROJECT_ROOT/scripts/fetch_runtime_deps.sh" "$PACKAGE_ROOT/scripts/fetch_runtime_deps.sh"
copy_path_if_exists "$PROJECT_ROOT/scripts/install_lua_deps.ps1" "$PACKAGE_ROOT/scripts/install_lua_deps.ps1"
copy_path_if_exists "$PROJECT_ROOT/scripts/install_lua_deps.sh" "$PACKAGE_ROOT/scripts/install_lua_deps.sh"
copy_path_if_exists "$PROJECT_ROOT/scripts/package_lua_runtime.ps1" "$PACKAGE_ROOT/scripts/package_lua_runtime.ps1"
copy_path_if_exists "$PROJECT_ROOT/scripts/package_lua_runtime.sh" "$PACKAGE_ROOT/scripts/package_lua_runtime.sh"
copy_path_if_exists "$PROJECT_ROOT/scripts/package_ffi_sdk.ps1" "$PACKAGE_ROOT/scripts/package_ffi_sdk.ps1"
copy_path_if_exists "$PROJECT_ROOT/scripts/package_ffi_sdk.sh" "$PACKAGE_ROOT/scripts/package_ffi_sdk.sh"
copy_path_if_exists "$PROJECT_ROOT/scripts/package_demo.ps1" "$PACKAGE_ROOT/scripts/package_demo.ps1"
copy_path_if_exists "$PROJECT_ROOT/scripts/package_demo.sh" "$PACKAGE_ROOT/scripts/package_demo.sh"
copy_path_if_exists "$PROJECT_ROOT/LICENSE" "$PACKAGE_ROOT/licenses/LICENSE"

cat > "$PACKAGE_ROOT/source-deps-manifest.json" <<JSON
{
  "schema_version": 1,
  "version": "${VERSION}",
  "package_name": "luaskills-source-deps",
  "lua_packages_manifest": "lua_packages.txt",
  "native_dependencies": {
    "openssl": "3.4.1",
    "curl": "8.13.0",
    "zlib": "1.3.1",
    "pcre2": "10.45",
    "libyaml": "0.2.5"
  },
  "host_dependencies": {
    "vldb-controller": "v0.2.1"
  },
  "runtime_assets": {
    "repo": "OpenVulcan/vulcan-luaskills",
    "tag": "${VERSION}"
  },
  "targets": ["all", "lua", "vldb"]
}
JSON

ARCHIVE_NAME="luaskills-source-deps-${VERSION}.tar.gz"
create_tar_from_dir "$PACKAGE_ROOT" "$OUTPUT_DIR/$ARCHIVE_NAME"
echo "Source dependency package created: $OUTPUT_DIR/$ARCHIVE_NAME"

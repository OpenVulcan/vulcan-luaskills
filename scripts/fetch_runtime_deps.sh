#!/usr/bin/env bash
set -euo pipefail

# ProjectRoot points at the repository root regardless of the caller location.
# ProjectRoot 指向仓库根目录，避免调用方当前位置影响路径解析。
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Target selects which dependency group to install.
# Target 选择需要安装的依赖分组。
TARGET="${1:-all}"

# RuntimeRoot receives installed runtime files.
# RuntimeRoot 接收安装后的运行期文件。
RUNTIME_ROOT="${RUNTIME_ROOT:-output}"

# LuaRuntimeRepo stores the GitHub repository for Lua runtime assets.
# LuaRuntimeRepo 保存 Lua runtime 资产所在的 GitHub 仓库。
LUA_RUNTIME_REPO="${LUA_RUNTIME_REPO:-OpenVulcan/vulcan-luaskills}"

# LuaRuntimeVersion stores the GitHub Release tag for Lua runtime assets.
# LuaRuntimeVersion 保存 Lua runtime 资产的 GitHub Release 标签。
LUA_RUNTIME_VERSION="${LUA_RUNTIME_VERSION:-v0.1.0}"

# VldbControllerRepo stores the GitHub repository for vldb-controller assets.
# VldbControllerRepo 保存 vldb-controller 资产所在的 GitHub 仓库。
VLDB_CONTROLLER_REPO="${VLDB_CONTROLLER_REPO:-OpenVulcan/vldb-controller}"

# VldbControllerVersion stores the GitHub Release tag for vldb-controller assets.
# VldbControllerVersion 保存 vldb-controller 资产的 GitHub Release 标签。
VLDB_CONTROLLER_VERSION="${VLDB_CONTROLLER_VERSION:-v0.2.1}"

ensure_dir() {
  # Create one directory when it does not exist.
  # 在目录不存在时创建该目录。
  mkdir -p "$1"
}

platform_key() {
  # Resolve the current luaskills runtime asset platform key.
  # 解析当前 luaskills runtime 资产平台标识。
  case "$(uname -s)" in
    Linux) os_key="linux" ;;
    Darwin) os_key="macos" ;;
    *) echo "Unsupported operating system: $(uname -s)" >&2; return 1 ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64) arch_key="x64" ;;
    aarch64|arm64) arch_key="arm64" ;;
    *) echo "Unsupported architecture: $(uname -m)" >&2; return 1 ;;
  esac
  printf '%s-%s\n' "$os_key" "$arch_key"
}

vldb_asset_info() {
  # Resolve vldb-controller target, extension, and binary name.
  # 解析 vldb-controller 的目标三元组、扩展名与二进制文件名。
  case "$(uname -m)" in
    x86_64|amd64) arch_key="x86_64" ;;
    aarch64|arm64) arch_key="aarch64" ;;
    *) echo "Unsupported architecture for vldb-controller: $(uname -m)" >&2; return 1 ;;
  esac
  case "$(uname -s)" in
    Linux) printf '%s-unknown-linux-gnu|.tar.gz|vldb-controller\n' "$arch_key" ;;
    Darwin) printf '%s-apple-darwin|.tar.gz|vldb-controller\n' "$arch_key" ;;
    *) echo "Unsupported operating system for vldb-controller: $(uname -s)" >&2; return 1 ;;
  esac
}

release_asset_url() {
  # Find one exact GitHub Release asset download URL.
  # 查找一个精确 GitHub Release 资产下载地址。
  local repo="$1"
  local tag="$2"
  local asset_name="$3"
  local api_url="https://api.github.com/repos/${repo}/releases/tags/${tag}"
  curl -fsSL "$api_url" | python3 -c '
import json
import sys
asset_name = sys.argv[1]
data = json.load(sys.stdin)
for asset in data.get("assets", []):
    if asset.get("name") == asset_name:
        print(asset.get("browser_download_url", ""))
        raise SystemExit(0)
raise SystemExit(f"asset not found: {asset_name}")
' "$asset_name"
}

install_lua_runtime() {
  # Download and install one luaskills Lua runtime package.
  # 下载并安装一个 luaskills Lua runtime 包。
  local platform
  platform="$(platform_key)"
  local asset_name="lua-runtime-${platform}.tar.gz"
  local temp_dir
  temp_dir="$(mktemp -d)"
  trap 'rm -rf "$temp_dir"' RETURN
  local archive="$temp_dir/$asset_name"
  local extract_dir="$temp_dir/extract"
  ensure_dir "$extract_dir"
  curl -fSL "$(release_asset_url "$LUA_RUNTIME_REPO" "$LUA_RUNTIME_VERSION" "$asset_name")" -o "$archive"
  tar -xzf "$archive" -C "$extract_dir"
  ensure_dir "$RUNTIME_ROOT"
  for dir_name in lua_packages libs resources; do
    if [ -d "$extract_dir/$dir_name" ]; then
      ensure_dir "$RUNTIME_ROOT/$dir_name"
      cp -a "$extract_dir/$dir_name"/. "$RUNTIME_ROOT/$dir_name"/
    fi
  done
  if [ -d "$extract_dir/licenses" ]; then
    ensure_dir "$RUNTIME_ROOT/licenses/lua-runtime"
    cp -a "$extract_dir/licenses"/. "$RUNTIME_ROOT/licenses/lua-runtime"/
  fi
}

install_vldb_controller() {
  # Download and install vldb-controller into runtime bin.
  # 下载 vldb-controller 并安装到运行期 bin 目录。
  local info target archive_ext binary_name
  info="$(vldb_asset_info)"
  IFS='|' read -r target archive_ext binary_name <<< "$info"
  local asset_name="vldb-controller-${VLDB_CONTROLLER_VERSION}-${target}${archive_ext}"
  local temp_dir
  temp_dir="$(mktemp -d)"
  trap 'rm -rf "$temp_dir"' RETURN
  local archive="$temp_dir/$asset_name"
  local extract_dir="$temp_dir/extract"
  ensure_dir "$extract_dir"
  curl -fSL "$(release_asset_url "$VLDB_CONTROLLER_REPO" "$VLDB_CONTROLLER_VERSION" "$asset_name")" -o "$archive"
  tar -xzf "$archive" -C "$extract_dir"
  local binary
  binary="$(find "$extract_dir" -type f -name "$binary_name" | head -1)"
  [ -n "$binary" ] || { echo "vldb-controller binary not found in $asset_name" >&2; return 1; }
  ensure_dir "$RUNTIME_ROOT/bin"
  cp -f "$binary" "$RUNTIME_ROOT/bin/$binary_name"
  chmod +x "$RUNTIME_ROOT/bin/$binary_name"
  ensure_dir "$RUNTIME_ROOT/resources"
  cat > "$RUNTIME_ROOT/resources/vldb-controller-manifest.json" <<JSON
{
  "schema_version": 1,
  "name": "vldb-controller",
  "version": "${VLDB_CONTROLLER_VERSION}",
  "asset": "${asset_name}",
  "installed_binary": "bin/${binary_name}"
}
JSON
}

cd "$PROJECT_ROOT"
ensure_dir "$RUNTIME_ROOT/resources"

case "$TARGET" in
  all)
    install_lua_runtime
    install_vldb_controller
    ;;
  lua)
    install_lua_runtime
    ;;
  vldb)
    install_vldb_controller
    ;;
  *)
    echo "Usage: $0 [all|lua|vldb]" >&2
    exit 2
    ;;
esac

echo "Runtime dependency target '$TARGET' installed into $RUNTIME_ROOT"

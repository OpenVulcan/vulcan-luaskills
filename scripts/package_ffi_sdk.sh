#!/usr/bin/env bash
set -euo pipefail

# ProjectRoot points at the repository root regardless of the caller location.
# ProjectRoot 指向仓库根目录，避免调用方当前位置影响路径解析。
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Platform stores the release asset platform key.
# Platform 保存发布资产使用的平台标识。
PLATFORM="${1:-}"

# OutputDir stores final release archives.
# OutputDir 保存最终发布压缩包。
OUTPUT_DIR="${OUTPUT_DIR:-target/release-packages}"

ensure_dir() {
  # Create one directory when it does not exist.
  # 在目录不存在时创建该目录。
  mkdir -p "$1"
}

if [ -z "$PLATFORM" ]; then
  case "$(uname -s)" in
    Linux) os_key="linux" ;;
    Darwin) os_key="macos" ;;
    *) os_key="unknown" ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64) arch_key="x64" ;;
    aarch64|arm64) arch_key="arm64" ;;
    *) arch_key="$(uname -m)" ;;
  esac
  PLATFORM="${os_key}-${arch_key}"
fi

cd "$PROJECT_ROOT"
PACKAGE_ROOT="target/ffi-sdk-package/luaskills-ffi-sdk"
rm -rf "$PACKAGE_ROOT"
ensure_dir "$PACKAGE_ROOT/include"
ensure_dir "$PACKAGE_ROOT/lib"
ensure_dir "$PACKAGE_ROOT/licenses"
ensure_dir "$OUTPUT_DIR"

cp -f include/*.h "$PACKAGE_ROOT/include/"
find target/release -maxdepth 1 -type f \( -name '*.dll' -o -name '*.lib' -o -name '*.so' -o -name '*.dylib' -o -name '*.a' \) -exec cp -f {} "$PACKAGE_ROOT/lib/" \; 2>/dev/null || true
cp -f LICENSE "$PACKAGE_ROOT/licenses/LICENSE"

cat > "$PACKAGE_ROOT/ffi-sdk-manifest.json" <<JSON
{
  "schema_version": 1,
  "package_name": "luaskills-ffi-sdk-${PLATFORM}",
  "platform": "${PLATFORM}",
  "headers": ["include/vulcan_luaskills_ffi.h", "include/vulcan_luaskills_json_ffi.h"],
  "library_dir": "lib"
}
JSON

ARCHIVE_NAME="luaskills-ffi-sdk-${PLATFORM}.tar.gz"
tar -czf "$OUTPUT_DIR/$ARCHIVE_NAME" -C "$PACKAGE_ROOT" .
echo "FFI SDK package created: $OUTPUT_DIR/$ARCHIVE_NAME"

#!/usr/bin/env bash
set -euo pipefail

# ProjectRoot points at the repository root regardless of the caller location.
# ProjectRoot 指向仓库根目录，避免调用方当前位置影响路径解析。
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Mode stores the demo integration mode.
# Mode 保存 demo 集成模式。
MODE="${1:?Usage: package_demo.sh ffi|rust [platform]}"

# Platform stores the release asset platform key.
# Platform 保存发布资产使用的平台标识。
PLATFORM="${2:-}"

# OutputDir stores final release archives.
# OutputDir 保存最终发布压缩包。
OUTPUT_DIR="${OUTPUT_DIR:-target/release-packages}"

# ReleaseTag stores the Git tag used by packaged Rust demos.
# ReleaseTag 保存发布 Rust demo 使用的 Git 标签。
RELEASE_TAG="${3:-${RELEASE_TAG:-v0.1.0}}"

ensure_dir() {
  # Create one directory when it does not exist.
  # 在目录不存在时创建该目录。
  mkdir -p "$1"
}

write_packaged_demo_scripts() {
  # Write run scripts that work from the packaged demo root.
  # 写入可从发布 demo 包根目录直接运行的脚本。
  local mode="$1"
  local package_root="$2"

  if [ "$mode" = "rust" ]; then
    cat > "$package_root/run.ps1" <<'PS1'
param(
    # Dependency target to fetch before running the packaged demo.
    # 运行发布 demo 前需要拉取的依赖目标。
    [ValidateSet("none", "all", "lua", "vldb")]
    [string]$Fetch = "none"
)

$ErrorActionPreference = "Stop"

# PackageRoot points at the extracted demo package root.
# PackageRoot 指向解压后的 demo 包根目录。
$PackageRoot = if ($PSScriptRoot) { $PSScriptRoot } else { (Get-Location).Path }

# RuntimeRoot is the packaged runtime root consumed by the demo.
# RuntimeRoot 是 demo 使用的包内运行根目录。
$RuntimeRoot = Join-Path $PackageRoot "runtime"

if ($Fetch -ne "none") {
    & (Join-Path $PackageRoot "scripts\fetch_runtime_deps.ps1") -Target $Fetch -RuntimeRoot $RuntimeRoot
}

if (Test-Path -LiteralPath (Join-Path $RuntimeRoot "resources\runtime-env.ps1")) {
    . (Join-Path $RuntimeRoot "resources\runtime-env.ps1")
}

$env:LUASKILLS_RUNTIME_ROOT = $RuntimeRoot
cargo run --manifest-path (Join-Path $PackageRoot "Cargo.toml")
PS1
    cat > "$package_root/run.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

# PackageRoot points at the extracted demo package root.
# PackageRoot 指向解压后的 demo 包根目录。
PACKAGE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Target selects which dependency group to fetch before running.
# Target 选择运行前需要拉取的依赖分组。
TARGET="${1:-none}"

# RuntimeRoot is the packaged runtime root consumed by the demo.
# RuntimeRoot 是 demo 使用的包内运行根目录。
RUNTIME_ROOT="$PACKAGE_ROOT/runtime"

if [ "$TARGET" != "none" ]; then
  RUNTIME_ROOT="$RUNTIME_ROOT" bash "$PACKAGE_ROOT/scripts/fetch_runtime_deps.sh" "$TARGET"
fi

if [ -f "$RUNTIME_ROOT/resources/runtime-env.sh" ]; then
  # shellcheck source=/dev/null
  RUNTIME_ROOT="$RUNTIME_ROOT" . "$RUNTIME_ROOT/resources/runtime-env.sh"
fi

LUASKILLS_RUNTIME_ROOT="$RUNTIME_ROOT" cargo run --manifest-path "$PACKAGE_ROOT/Cargo.toml"
SH
    chmod +x "$package_root/run.sh"
    return
  fi

  cat > "$package_root/run.ps1" <<'PS1'
param(
    # Dependency target to fetch before running the packaged demo.
    # 运行发布 demo 前需要拉取的依赖目标。
    [ValidateSet("none", "all", "lua", "vldb")]
    [string]$Fetch = "none"
)

$ErrorActionPreference = "Stop"

# PackageRoot points at the extracted demo package root.
# PackageRoot 指向解压后的 demo 包根目录。
$PackageRoot = if ($PSScriptRoot) { $PSScriptRoot } else { (Get-Location).Path }

# RuntimeRoot is the packaged runtime root consumed by the demo.
# RuntimeRoot 是 demo 使用的包内运行根目录。
$RuntimeRoot = Join-Path $PackageRoot "runtime"

if ($Fetch -ne "none") {
    & (Join-Path $PackageRoot "scripts\fetch_runtime_deps.ps1") -Target $Fetch -RuntimeRoot $RuntimeRoot
}

if (Test-Path -LiteralPath (Join-Path $RuntimeRoot "resources\runtime-env.ps1")) {
    . (Join-Path $RuntimeRoot "resources\runtime-env.ps1")
}

$Library = Get-ChildItem -File -Path (Join-Path $PackageRoot "lib\*") -Include "*.dll","*.so","*.dylib" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $Library) {
    throw "No LuaSkills FFI library found under package lib directory."
}

$CompatRuntime = Join-Path $PackageRoot "standard_runtime\runtime_root"
New-Item -ItemType Directory -Force -Path $CompatRuntime | Out-Null
Copy-Item -Recurse -Force -Path (Join-Path $RuntimeRoot "*") -Destination $CompatRuntime -ErrorAction SilentlyContinue
$LibDir = Join-Path $PackageRoot "lib"
if ($IsWindows -or $env:OS -eq "Windows_NT") {
    $env:PATH = "$LibDir;$env:PATH"
} elseif ($IsMacOS) {
    $env:DYLD_LIBRARY_PATH = "$LibDir" + $(if ($env:DYLD_LIBRARY_PATH) { ":$env:DYLD_LIBRARY_PATH" } else { "" })
} else {
    $env:LD_LIBRARY_PATH = "$LibDir" + $(if ($env:LD_LIBRARY_PATH) { ":$env:LD_LIBRARY_PATH" } else { "" })
}
$env:VULCAN_LUASKILLS_LIB = $Library.FullName
python (Join-Path $PackageRoot "python\demo.py")
PS1
  cat > "$package_root/run.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

# PackageRoot points at the extracted demo package root.
# PackageRoot 指向解压后的 demo 包根目录。
PACKAGE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Target selects which dependency group to fetch before running.
# Target 选择运行前需要拉取的依赖分组。
TARGET="${1:-none}"

# RuntimeRoot is the packaged runtime root consumed by the demo.
# RuntimeRoot 是 demo 使用的包内运行根目录。
RUNTIME_ROOT="$PACKAGE_ROOT/runtime"

if [ "$TARGET" != "none" ]; then
  RUNTIME_ROOT="$RUNTIME_ROOT" bash "$PACKAGE_ROOT/scripts/fetch_runtime_deps.sh" "$TARGET"
fi

if [ -f "$RUNTIME_ROOT/resources/runtime-env.sh" ]; then
  # shellcheck source=/dev/null
  RUNTIME_ROOT="$RUNTIME_ROOT" . "$RUNTIME_ROOT/resources/runtime-env.sh"
fi

LIBRARY="$(find "$PACKAGE_ROOT/lib" -maxdepth 1 -type f \( -name '*.so' -o -name '*.dylib' -o -name '*.dll' \) | head -1)"
[ -n "$LIBRARY" ] || { echo "No LuaSkills FFI library found under package lib directory." >&2; exit 1; }
mkdir -p "$PACKAGE_ROOT/standard_runtime/runtime_root"
cp -a "$RUNTIME_ROOT"/. "$PACKAGE_ROOT/standard_runtime/runtime_root"/
case "$(uname -s)" in
  Darwin) export DYLD_LIBRARY_PATH="$PACKAGE_ROOT/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" ;;
  Linux) export LD_LIBRARY_PATH="$PACKAGE_ROOT/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" ;;
esac
VULCAN_LUASKILLS_LIB="$LIBRARY" python3 "$PACKAGE_ROOT/python/demo.py"
SH
  chmod +x "$package_root/run.sh"
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
PACKAGE_ROOT="target/demo-package/luaskills-demo-$MODE"
rm -rf "$PACKAGE_ROOT"
ensure_dir "$PACKAGE_ROOT/scripts"
ensure_dir "$PACKAGE_ROOT/runtime"
ensure_dir "$PACKAGE_ROOT/licenses"
ensure_dir "$OUTPUT_DIR"

cp -a "examples/demo-$MODE"/. "$PACKAGE_ROOT"/

# Build caches are useful locally but must not be shipped in the demo source package.
# 构建缓存仅对本地开发有用，不能进入 demo 源码包产物。
rm -rf "$PACKAGE_ROOT/target"

cp -a "examples/ffi/standard_runtime/runtime_root"/. "$PACKAGE_ROOT/runtime"/
cp -f scripts/fetch_runtime_deps.ps1 "$PACKAGE_ROOT/scripts/fetch_runtime_deps.ps1"
cp -f scripts/fetch_runtime_deps.sh "$PACKAGE_ROOT/scripts/fetch_runtime_deps.sh"
cp -f LICENSE "$PACKAGE_ROOT/licenses/LICENSE"

if [ "$MODE" = "ffi" ]; then
  ensure_dir "$PACKAGE_ROOT/include"
  ensure_dir "$PACKAGE_ROOT/lib"
  ensure_dir "$PACKAGE_ROOT/python"
  cp -f include/*.h "$PACKAGE_ROOT/include/"
  cp -a examples/ffi/python/. "$PACKAGE_ROOT/python"/
  find target/release -maxdepth 1 -type f \( -name '*.dll' -o -name '*.lib' -o -name '*.so' -o -name '*.dylib' -o -name '*.a' \) -exec cp -f {} "$PACKAGE_ROOT/lib/" \; 2>/dev/null || true
else
  if [ -f "$PACKAGE_ROOT/Cargo.toml" ]; then
    python3 - "$PACKAGE_ROOT/Cargo.toml" "$RELEASE_TAG" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
text = text.replace(
    'vulcan-luaskills = { path = "../.." }',
    f'vulcan-luaskills = {{ git = "https://github.com/OpenVulcan/vulcan-luaskills.git", tag = "{sys.argv[2]}" }}',
)
path.write_text(text, encoding="utf-8")
PY
  fi
fi

write_packaged_demo_scripts "$MODE" "$PACKAGE_ROOT"

cat > "$PACKAGE_ROOT/demo-manifest.json" <<JSON
{
  "schema_version": 1,
  "package_name": "luaskills-demo-${MODE}-${PLATFORM}",
  "platform": "${PLATFORM}",
  "mode": "${MODE}",
  "runtime_root": "runtime",
  "release_tag": "${RELEASE_TAG}",
  "fetch_targets": ["all", "lua", "vldb"]
}
JSON

ARCHIVE_NAME="luaskills-demo-${MODE}-${PLATFORM}.tar.gz"
tar -czf "$OUTPUT_DIR/$ARCHIVE_NAME" -C "$PACKAGE_ROOT" .
echo "Demo package created: $OUTPUT_DIR/$ARCHIVE_NAME"

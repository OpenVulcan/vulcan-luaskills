param(
    [string]$Mode = "ffi",
    [string]$Platform = "",
    [string]$OutputDir = "target\release-packages",
    [string]$ReleaseTag = "v0.1.0"
)

$ErrorActionPreference = "Stop"

function Resolve-ProjectRoot {
    <#
    .SYNOPSIS
    Resolve the repository root from script metadata or the caller location.
    从脚本元数据或调用方位置解析仓库根目录。

    .PARAMETER ScriptDirectory
    Directory that contains the current script when PowerShell exposes it.
    PowerShell 可用时当前脚本所在的目录。

    .OUTPUTS
    Repository root path that contains Cargo.toml and scripts.
    包含 Cargo.toml 与 scripts 目录的仓库根路径。
    #>
    param([string]$ScriptDirectory)

    $Candidates = @()
    if ($ScriptDirectory) {
        $Candidates += $ScriptDirectory
    }
    $Candidates += (Get-Location).Path

    foreach ($Candidate in $Candidates) {
        $Current = $Candidate
        while ($Current) {
            if ((Test-Path -LiteralPath (Join-Path $Current "Cargo.toml")) -and (Test-Path -LiteralPath (Join-Path $Current "scripts"))) {
                return $Current
            }
            $Parent = Split-Path -Parent $Current
            if (-not $Parent -or $Parent -eq $Current) {
                break
            }
            $Current = $Parent
        }
    }

    throw "Unable to resolve project root from script or current directory."
}

# ScriptDir points at the current script directory when PowerShell exposes it.
# ScriptDir 在 PowerShell 提供脚本路径时指向当前脚本目录。
$ScriptDir = if ($PSScriptRoot) { $PSScriptRoot } elseif ($PSCommandPath) { Split-Path -Parent $PSCommandPath } elseif ($MyInvocation.MyCommand.Path) { Split-Path -Parent $MyInvocation.MyCommand.Path } else { "" }

# ProjectRoot points at the repository root regardless of the caller location.
# ProjectRoot 指向仓库根目录，避免调用方当前位置影响路径解析。
$ProjectRoot = Resolve-ProjectRoot -ScriptDirectory $ScriptDir
Set-Location $ProjectRoot

if ($Mode -ne "ffi" -and $Mode -ne "rust") {
    throw "Mode must be 'ffi' or 'rust'."
}

function Ensure-Dir {
    <#
    .SYNOPSIS
    Create one directory when it does not exist.
    在目录不存在时创建该目录。

    .PARAMETER Path
    Directory path to create.
    需要创建的目录路径。
    #>
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -ItemType Directory -Path $Path -Force | Out-Null
    }
}

function Write-PackagedDemoScripts {
    <#
    .SYNOPSIS
    Write run scripts that work from the packaged demo root.
    写入可从发布 demo 包根目录直接运行的脚本。

    .PARAMETER Mode
    Demo mode to generate scripts for.
    需要生成脚本的 demo 模式。

    .PARAMETER PackageRoot
    Package root that receives run scripts.
    接收运行脚本的包根目录。
    #>
    param(
        [string]$Mode,
        [string]$PackageRoot
    )

    if ($Mode -eq "rust") {
        @'
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
'@ | Set-Content -Path (Join-Path $PackageRoot "run.ps1") -Encoding UTF8

        @'
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
'@ | Set-Content -Path (Join-Path $PackageRoot "run.sh") -Encoding UTF8
        return
    }

    @'
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
'@ | Set-Content -Path (Join-Path $PackageRoot "run.ps1") -Encoding UTF8

    @'
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
'@ | Set-Content -Path (Join-Path $PackageRoot "run.sh") -Encoding UTF8
}

if (-not $Platform) {
    $Arch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToLowerInvariant()
    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
        $Platform = "windows-$Arch"
    } elseif ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) {
        $Platform = "macos-$Arch"
    } else {
        $Platform = "linux-$Arch"
    }
}

$PackageRoot = "target\demo-package\luaskills-demo-$Mode"
if (Test-Path -LiteralPath $PackageRoot) {
    Remove-Item -LiteralPath $PackageRoot -Recurse -Force
}

Ensure-Dir $PackageRoot
Ensure-Dir (Join-Path $PackageRoot "scripts")
Ensure-Dir (Join-Path $PackageRoot "runtime")
Ensure-Dir (Join-Path $PackageRoot "licenses")
Ensure-Dir $OutputDir

Copy-Item -Recurse -Force -Path "examples\demo-$Mode\*" -Destination $PackageRoot -ErrorAction SilentlyContinue

# Build caches are useful locally but must not be shipped in the demo source package.
# 构建缓存仅对本地开发有用，不能进入 demo 源码包产物。
Remove-Item -Recurse -Force -LiteralPath (Join-Path $PackageRoot "target") -ErrorAction SilentlyContinue

Copy-Item -Recurse -Force -Path "examples\ffi\standard_runtime\runtime_root\*" -Destination (Join-Path $PackageRoot "runtime") -ErrorAction SilentlyContinue
Copy-Item -Force -LiteralPath "scripts\fetch_runtime_deps.ps1" -Destination (Join-Path $PackageRoot "scripts\fetch_runtime_deps.ps1")
Copy-Item -Force -LiteralPath "scripts\fetch_runtime_deps.sh" -Destination (Join-Path $PackageRoot "scripts\fetch_runtime_deps.sh")
Copy-Item -Force -LiteralPath "LICENSE" -Destination (Join-Path $PackageRoot "licenses\LICENSE")

if ($Mode -eq "ffi") {
    Ensure-Dir (Join-Path $PackageRoot "include")
    Ensure-Dir (Join-Path $PackageRoot "lib")
    Ensure-Dir (Join-Path $PackageRoot "python")
    Copy-Item -Force -Path "include\*.h" -Destination (Join-Path $PackageRoot "include")
    Copy-Item -Recurse -Force -Path "examples\ffi\python\*" -Destination (Join-Path $PackageRoot "python")
    Get-ChildItem -File -Path "target\release\*" -Include "*.dll","*.lib","*.so","*.dylib","*.a" -ErrorAction SilentlyContinue | ForEach-Object {
        Copy-Item -Force -LiteralPath $_.FullName -Destination (Join-Path $PackageRoot "lib")
    }
} else {
    $CargoTomlPath = Join-Path $PackageRoot "Cargo.toml"
    if (Test-Path -LiteralPath $CargoTomlPath) {
        (Get-Content -Raw -Path $CargoTomlPath).Replace('vulcan-luaskills = { path = "../.." }', "vulcan-luaskills = { git = `"https://github.com/OpenVulcan/vulcan-luaskills.git`", tag = `"$ReleaseTag`" }") |
            Set-Content -Path $CargoTomlPath -Encoding UTF8
    }
}

Write-PackagedDemoScripts -Mode $Mode -PackageRoot $PackageRoot

[ordered]@{
    schema_version = 1
    package_name = "luaskills-demo-$Mode-$Platform"
    platform = $Platform
    mode = $Mode
    runtime_root = "runtime"
    release_tag = $ReleaseTag
    fetch_targets = @("all", "lua", "vldb")
} | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $PackageRoot "demo-manifest.json") -Encoding UTF8

$ArchiveName = "luaskills-demo-$Mode-$Platform.tar.gz"
$ResolvedOutput = (Resolve-Path -LiteralPath $OutputDir).Path
Push-Location $PackageRoot
try {
    tar -czf (Join-Path $ResolvedOutput $ArchiveName) .
} finally {
    Pop-Location
}

Write-Host "Demo package created: $(Join-Path $OutputDir $ArchiveName)"

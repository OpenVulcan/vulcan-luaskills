param(
    # Target platform key used in archive and manifest names.
    # 用于归档文件与清单文件命名的目标平台标识。
    [string]$Platform = "",
    # Output directory that receives the final archive.
    # 接收最终压缩包的输出目录。
    [string]$OutputDir = "target\release-packages"
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

$PackageRoot = "target\ffi-sdk-package\luaskills-ffi-sdk"
if (Test-Path -LiteralPath $PackageRoot) {
    Remove-Item -LiteralPath $PackageRoot -Recurse -Force
}

Ensure-Dir (Join-Path $PackageRoot "include")
Ensure-Dir (Join-Path $PackageRoot "lib")
Ensure-Dir (Join-Path $PackageRoot "licenses")
Ensure-Dir $OutputDir

Copy-Item -Force -Path "include\*.h" -Destination (Join-Path $PackageRoot "include")
Get-ChildItem -File -Path "target\release\*" -Include "*.dll","*.lib","*.so","*.dylib","*.a" -ErrorAction SilentlyContinue | ForEach-Object {
    Copy-Item -Force -LiteralPath $_.FullName -Destination (Join-Path $PackageRoot "lib")
}
Copy-Item -Force -LiteralPath "LICENSE" -Destination (Join-Path $PackageRoot "licenses\LICENSE")

[ordered]@{
    schema_version = 1
    package_name = "luaskills-ffi-sdk-$Platform"
    platform = $Platform
    headers = @("include/vulcan_luaskills_ffi.h", "include/vulcan_luaskills_json_ffi.h")
    library_dir = "lib"
} | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $PackageRoot "ffi-sdk-manifest.json") -Encoding UTF8

$ArchiveName = "luaskills-ffi-sdk-$Platform.tar.gz"
$ResolvedOutput = (Resolve-Path -LiteralPath $OutputDir).Path
Push-Location $PackageRoot
try {
    tar -czf (Join-Path $ResolvedOutput $ArchiveName) .
} finally {
    Pop-Location
}

Write-Host "FFI SDK package created: $(Join-Path $OutputDir $ArchiveName)"

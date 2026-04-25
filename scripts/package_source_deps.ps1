param(
    # Source dependency package version.
    # 源码依赖包版本。
    [string]$Version = "v0.1.0",
    # Staging directory assembled before compression.
    # 压缩前用于组装源码依赖包的暂存目录。
    [string]$StagingDir = "target\source-deps-package",
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

function Copy-Path {
    <#
    .SYNOPSIS
    Copy one file or directory when it exists.
    在路径存在时复制一个文件或目录。

    .PARAMETER Source
    Source path.
    源路径。

    .PARAMETER Destination
    Destination path.
    目标路径。
    #>
    param(
        [string]$Source,
        [string]$Destination
    )

    if (Test-Path -LiteralPath $Source) {
        Copy-Item -Recurse -Force -LiteralPath $Source -Destination $Destination
    }
}

$PackageRoot = Join-Path $StagingDir "luaskills-source-deps"
if (Test-Path -LiteralPath $PackageRoot) {
    Remove-Item -LiteralPath $PackageRoot -Recurse -Force
}

Ensure-Dir $PackageRoot
Ensure-Dir (Join-Path $PackageRoot "scripts")
Ensure-Dir (Join-Path $PackageRoot "licenses")
Ensure-Dir $OutputDir

Copy-Path -Source (Join-Path $ProjectRoot "scripts\lua_packages.txt") -Destination (Join-Path $PackageRoot "lua_packages.txt")
Copy-Path -Source (Join-Path $ProjectRoot "scripts\luarocks_overrides") -Destination (Join-Path $PackageRoot "luarocks_overrides")
Copy-Path -Source (Join-Path $ProjectRoot "scripts\fetch_runtime_deps.ps1") -Destination (Join-Path $PackageRoot "scripts\fetch_runtime_deps.ps1")
Copy-Path -Source (Join-Path $ProjectRoot "scripts\fetch_runtime_deps.sh") -Destination (Join-Path $PackageRoot "scripts\fetch_runtime_deps.sh")
Copy-Path -Source (Join-Path $ProjectRoot "scripts\install_lua_deps.ps1") -Destination (Join-Path $PackageRoot "scripts\install_lua_deps.ps1")
Copy-Path -Source (Join-Path $ProjectRoot "scripts\install_lua_deps.sh") -Destination (Join-Path $PackageRoot "scripts\install_lua_deps.sh")
Copy-Path -Source (Join-Path $ProjectRoot "scripts\package_lua_runtime.ps1") -Destination (Join-Path $PackageRoot "scripts\package_lua_runtime.ps1")
Copy-Path -Source (Join-Path $ProjectRoot "scripts\package_lua_runtime.sh") -Destination (Join-Path $PackageRoot "scripts\package_lua_runtime.sh")
Copy-Path -Source (Join-Path $ProjectRoot "scripts\package_ffi_sdk.ps1") -Destination (Join-Path $PackageRoot "scripts\package_ffi_sdk.ps1")
Copy-Path -Source (Join-Path $ProjectRoot "scripts\package_ffi_sdk.sh") -Destination (Join-Path $PackageRoot "scripts\package_ffi_sdk.sh")
Copy-Path -Source (Join-Path $ProjectRoot "scripts\package_demo.ps1") -Destination (Join-Path $PackageRoot "scripts\package_demo.ps1")
Copy-Path -Source (Join-Path $ProjectRoot "scripts\package_demo.sh") -Destination (Join-Path $PackageRoot "scripts\package_demo.sh")
Copy-Path -Source (Join-Path $ProjectRoot "LICENSE") -Destination (Join-Path $PackageRoot "licenses\LICENSE")

$Manifest = [ordered]@{
    schema_version = 1
    version = $Version
    package_name = "luaskills-source-deps"
    lua_packages_manifest = "lua_packages.txt"
    native_dependencies = [ordered]@{
        openssl = "3.4.1"
        curl = "8.13.0"
        zlib = "1.3.1"
        pcre2 = "10.45"
        libyaml = "0.2.5"
    }
    host_dependencies = [ordered]@{
        "vldb-controller" = "v0.2.1"
    }
    runtime_assets = [ordered]@{
        repo = "OpenVulcan/vulcan-luaskills"
        tag = $Version
    }
    targets = @("all", "lua", "vldb")
}

$Manifest | ConvertTo-Json -Depth 12 | Set-Content -Path (Join-Path $PackageRoot "source-deps-manifest.json") -Encoding UTF8

$ArchiveName = "luaskills-source-deps-$Version.tar.gz"
$ArchivePath = Join-Path $OutputDir $ArchiveName
if (Test-Path -LiteralPath $ArchivePath) {
    Remove-Item -LiteralPath $ArchivePath -Force
}

$ResolvedOutput = (Resolve-Path -LiteralPath $OutputDir).Path
Push-Location $PackageRoot
try {
    tar -czf (Join-Path $ResolvedOutput $ArchiveName) .
} finally {
    Pop-Location
}

Write-Host "Source dependency package created: $ArchivePath"

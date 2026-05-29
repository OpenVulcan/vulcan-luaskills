param(
    # Dependency target to fetch for local debug runs.
    # 本地调试运行需要拉取的依赖目标。
    [ValidateSet("all", "lua", "vldb", "vldb-controller", "vldb-direct")]
    [string]$Target = "lua",
    # Optional database dependency preset used only by all/vldb targets.
    # 仅 all/vldb 目标使用的可选数据库依赖预设。
    [ValidateSet("none", "vldb-controller", "vldb-direct", "host-callback")]
    [string]$Database = "none",
    # Runtime root that receives debug dependencies.
    # 接收调试依赖的运行根目录。
    [string]$RuntimeRoot = "output"
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
            if ((Test-Path -LiteralPath (Join-Path $Current "Cargo.toml")) -and (Test-Path -LiteralPath (Join-Path $Current "scripts\deps\fetch_deps.ps1"))) {
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

$ScriptDir = if ($PSScriptRoot) { $PSScriptRoot } elseif ($PSCommandPath) { Split-Path -Parent $PSCommandPath } elseif ($MyInvocation.MyCommand.Path) { Split-Path -Parent $MyInvocation.MyCommand.Path } else { "" }
$ProjectRoot = Resolve-ProjectRoot -ScriptDirectory $ScriptDir
$RuntimeRootPath = if ([System.IO.Path]::IsPathRooted($RuntimeRoot)) { $RuntimeRoot } else { Join-Path $ProjectRoot $RuntimeRoot }
New-Item -ItemType Directory -Force -Path $RuntimeRootPath | Out-Null
powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $ProjectRoot "scripts\deps\fetch_deps.ps1") -Target $Target -Database $Database -RuntimeRoot $RuntimeRootPath
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
Write-Host "Debug runtime dependencies are ready under $RuntimeRootPath"

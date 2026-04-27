param(
    # Dependency target to install: all, lua, or vldb.
    # 需要安装的依赖目标：all、lua 或 vldb。
    [ValidateSet("all", "lua", "vldb")]
    [string]$Target = "all",
    # Runtime root that receives the installed files.
    # 接收安装文件的运行根目录。
    [string]$RuntimeRoot = "output",
    # Lua runtime release repository.
    # Lua runtime 发布仓库。
    [string]$LuaRuntimeRepo = "LuaSkills/luaskills",
    # Lua runtime release tag.
    # Lua runtime 发布标签。
    [string]$LuaRuntimeVersion = "v0.2.1",
    # vldb-controller release repository.
    # vldb-controller 发布仓库。
    [string]$VldbControllerRepo = "OpenVulcan/vldb-controller",
    # vldb-controller release tag.
    # vldb-controller 发布标签。
    [string]$VldbControllerVersion = "v0.2.1",
    # Also install vldb-controller into third_party for source builds.
    # 是否同时将 vldb-controller 安装到 third_party 供源码构建复用。
    [switch]$DevCache
)

$ErrorActionPreference = "Stop"

function Resolve-ProjectRoot {
    <#
    .SYNOPSIS
    Resolve the repository or packaged demo root from script metadata or the caller location.
    从脚本元数据或调用方位置解析仓库根目录或已发布 demo 包根目录。

    .PARAMETER ScriptDirectory
    Directory that contains the current script when PowerShell exposes it.
    PowerShell 可用时当前脚本所在的目录。

    .OUTPUTS
    Root path that contains either Cargo.toml plus scripts, or packaged demo scripts.
    包含 Cargo.toml 与 scripts 的仓库根路径，或包含发布 demo 脚本的包根路径。
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
            $PackagedFetchScript = Join-Path $Current "scripts\fetch_runtime_deps.ps1"
            $PackagedManifest = Join-Path $Current "demo-manifest.json"
            $PackagedRuntime = Join-Path $Current "runtime"
            if ((Test-Path -LiteralPath $PackagedFetchScript) -and ((Test-Path -LiteralPath $PackagedManifest) -or (Test-Path -LiteralPath $PackagedRuntime))) {
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

function Get-PlatformKey {
    <#
    .SYNOPSIS
    Resolve the current platform key used by luaskills runtime assets.
    解析当前平台对应的 luaskills runtime 资产标识。

    .OUTPUTS
    Platform key such as windows-x64 or linux-arm64.
    类似 windows-x64 或 linux-arm64 的平台标识。
    #>
    $Arch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToLowerInvariant()
    switch ($Arch) {
        "x64" { $ArchKey = "x64" }
        "arm64" { $ArchKey = "arm64" }
        default { throw "Unsupported architecture: $Arch" }
    }

    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
        if ($ArchKey -ne "x64") {
            throw "Windows runtime assets currently support x64 only."
        }
        return "windows-x64"
    }
    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) {
        return "macos-$ArchKey"
    }
    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Linux)) {
        return "linux-$ArchKey"
    }
    throw "Unsupported operating system."
}

function Get-VldbAssetInfo {
    <#
    .SYNOPSIS
    Resolve vldb-controller asset metadata for the current platform.
    解析当前平台的 vldb-controller 资产元数据。

    .OUTPUTS
    Hashtable containing target, archive_ext, and binary_name.
    包含 target、archive_ext 与 binary_name 的哈希表。
    #>
    $Arch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToLowerInvariant()
    switch ($Arch) {
        "x64" { $ArchKey = "x86_64" }
        "arm64" { $ArchKey = "aarch64" }
        default { throw "Unsupported architecture for vldb-controller: $Arch" }
    }

    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
        if ($ArchKey -ne "x86_64") {
            throw "vldb-controller currently supports Windows x86_64 only."
        }
        return @{ target = "x86_64-pc-windows-msvc"; archive_ext = ".zip"; binary_name = "vldb-controller.exe" }
    }
    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) {
        return @{ target = "$ArchKey-apple-darwin"; archive_ext = ".tar.gz"; binary_name = "vldb-controller" }
    }
    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Linux)) {
        return @{ target = "$ArchKey-unknown-linux-gnu"; archive_ext = ".tar.gz"; binary_name = "vldb-controller" }
    }
    throw "Unsupported operating system for vldb-controller."
}

function Get-ReleaseAssetUrl {
    <#
    .SYNOPSIS
    Find one exact GitHub Release asset download URL.
    查找一个精确 GitHub Release 资产下载地址。

    .PARAMETER Repo
    GitHub repository in owner/name form.
    owner/name 形式的 GitHub 仓库。

    .PARAMETER Tag
    Release tag name.
    Release 标签名。

    .PARAMETER AssetName
    Exact asset file name.
    精确资产文件名。
    #>
    param(
        [string]$Repo,
        [string]$Tag,
        [string]$AssetName
    )

    $ApiUrl = "https://api.github.com/repos/$Repo/releases/tags/$Tag"
    $Release = Invoke-RestMethod -Uri $ApiUrl -UseBasicParsing
    $Asset = $Release.assets | Where-Object { $_.name -eq $AssetName } | Select-Object -First 1
    if (-not $Asset) {
        $Available = ($Release.assets | ForEach-Object { $_.name }) -join ", "
        throw "Asset '$AssetName' not found in $Repo@$Tag. Available: $Available"
    }
    return $Asset.browser_download_url
}

function Expand-ArchiveSmart {
    <#
    .SYNOPSIS
    Extract a .zip or .tar.gz archive.
    解压 .zip 或 .tar.gz 压缩包。

    .PARAMETER ArchivePath
    Archive path to extract.
    需要解压的压缩包路径。

    .PARAMETER Destination
    Destination directory.
    目标目录。
    #>
    param(
        [string]$ArchivePath,
        [string]$Destination
    )

    Ensure-Dir $Destination
    if ($ArchivePath.EndsWith(".zip", [System.StringComparison]::OrdinalIgnoreCase)) {
        Expand-Archive -Path $ArchivePath -DestinationPath $Destination -Force
    } else {
        tar -xzf $ArchivePath -C $Destination
    }
}

function Test-ExistingRuntimeContent {
    <#
    .SYNOPSIS
    Check whether a packaged demo already contains runnable runtime content.
    检查已发布 demo 包是否已经包含可运行的 runtime 内容。

    .PARAMETER RuntimeRootPath
    Runtime root to inspect.
    需要检查的运行根目录。

    .OUTPUTS
    Boolean value indicating whether local runtime content already exists.
    表示本地 runtime 内容是否已经存在的布尔值。
    #>
    param([string]$RuntimeRootPath)

    $SkillsDir = Join-Path $RuntimeRootPath "skills"
    $LuaPackagesDir = Join-Path $RuntimeRootPath "lua_packages"
    return (Test-Path -LiteralPath $SkillsDir) -or (Test-Path -LiteralPath $LuaPackagesDir)
}

function Install-LuaRuntime {
    <#
    .SYNOPSIS
    Download and install one luaskills Lua runtime package.
    下载并安装一个 luaskills Lua runtime 包。

    .PARAMETER RuntimeRootPath
    Runtime root that receives lua_packages, libs, resources, and licenses.
    接收 lua_packages、libs、resources 与 licenses 的运行根目录。
    #>
    param([string]$RuntimeRootPath)

    $Platform = Get-PlatformKey
    $AssetName = "lua-runtime-$Platform.tar.gz"
    $TempDir = Join-Path $env:TEMP "luaskills_lua_runtime_$PID"
    $ArchivePath = Join-Path $TempDir $AssetName
    $ExtractDir = Join-Path $TempDir "extract"

    if (Test-Path -LiteralPath $TempDir) {
        Remove-Item -LiteralPath $TempDir -Recurse -Force
    }
    Ensure-Dir $TempDir

    try {
        try {
            $Url = Get-ReleaseAssetUrl -Repo $LuaRuntimeRepo -Tag $LuaRuntimeVersion -AssetName $AssetName
        } catch {
            if (Test-ExistingRuntimeContent -RuntimeRootPath $RuntimeRootPath) {
                Write-Warning "Lua runtime asset '$AssetName' was not found in $LuaRuntimeRepo@$LuaRuntimeVersion. Existing packaged runtime content will be used."
                return
            }
            throw
        }
        Invoke-WebRequest -Uri $Url -OutFile $ArchivePath -UseBasicParsing
        Expand-ArchiveSmart -ArchivePath $ArchivePath -Destination $ExtractDir

        foreach ($DirName in @("lua_packages", "libs", "resources")) {
            $Source = Join-Path $ExtractDir $DirName
            if (Test-Path -LiteralPath $Source) {
                Ensure-Dir (Join-Path $RuntimeRootPath $DirName)
                Copy-Item -Recurse -Force -Path (Join-Path $Source "*") -Destination (Join-Path $RuntimeRootPath $DirName) -ErrorAction SilentlyContinue
            }
        }

        $LicenseSource = Join-Path $ExtractDir "licenses"
        if (Test-Path -LiteralPath $LicenseSource) {
            $LicenseDest = Join-Path $RuntimeRootPath "licenses\lua-runtime"
            Ensure-Dir $LicenseDest
            Copy-Item -Recurse -Force -Path (Join-Path $LicenseSource "*") -Destination $LicenseDest -ErrorAction SilentlyContinue
        }
    } finally {
        Remove-Item -LiteralPath $TempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Install-VldbController {
    <#
    .SYNOPSIS
    Download and install vldb-controller into the demo runtime bin directory.
    下载 vldb-controller 并安装到 demo 运行期 bin 目录。

    .PARAMETER RuntimeRootPath
    Runtime root that receives bin/vldb-controller.
    接收 bin/vldb-controller 的运行根目录。
    #>
    param([string]$RuntimeRootPath)

    $AssetInfo = Get-VldbAssetInfo
    $AssetName = "vldb-controller-$VldbControllerVersion-$($AssetInfo.target)$($AssetInfo.archive_ext)"
    $TempDir = Join-Path $env:TEMP "luaskills_vldb_$PID"
    $ArchivePath = Join-Path $TempDir $AssetName
    $ExtractDir = Join-Path $TempDir "extract"

    if (Test-Path -LiteralPath $TempDir) {
        Remove-Item -LiteralPath $TempDir -Recurse -Force
    }
    Ensure-Dir $TempDir

    try {
        $Url = Get-ReleaseAssetUrl -Repo $VldbControllerRepo -Tag $VldbControllerVersion -AssetName $AssetName
        Invoke-WebRequest -Uri $Url -OutFile $ArchivePath -UseBasicParsing
        Expand-ArchiveSmart -ArchivePath $ArchivePath -Destination $ExtractDir

        $Binary = Get-ChildItem -Recurse -File -Path $ExtractDir -Filter $AssetInfo.binary_name | Select-Object -First 1
        if (-not $Binary) {
            throw "vldb-controller binary not found in $AssetName"
        }

        $RuntimeBin = Join-Path $RuntimeRootPath "bin"
        Ensure-Dir $RuntimeBin
        Copy-Item -Force -LiteralPath $Binary.FullName -Destination (Join-Path $RuntimeBin $AssetInfo.binary_name)

        if ($DevCache) {
            $DevBin = Join-Path $ProjectRoot "third_party\vldb_controller\bin"
            Ensure-Dir $DevBin
            Copy-Item -Force -LiteralPath $Binary.FullName -Destination (Join-Path $DevBin $AssetInfo.binary_name)
        }

        $LicenseDest = Join-Path $RuntimeRootPath "licenses\vldb-controller"
        Ensure-Dir $LicenseDest
        [ordered]@{
            schema_version = 1
            name = "vldb-controller"
            version = $VldbControllerVersion
            asset = $AssetName
            installed_binary = "bin/$($AssetInfo.binary_name)"
        } | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $RuntimeRootPath "resources\vldb-controller-manifest.json") -Encoding UTF8
    } finally {
        Remove-Item -LiteralPath $TempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Ensure-Dir $RuntimeRoot
Ensure-Dir (Join-Path $RuntimeRoot "resources")

if ($Target -eq "all" -or $Target -eq "lua") {
    Install-LuaRuntime -RuntimeRootPath $RuntimeRoot
}

if ($Target -eq "all" -or $Target -eq "vldb") {
    Install-VldbController -RuntimeRootPath $RuntimeRoot
}

Write-Host "Runtime dependency target '$Target' installed into $RuntimeRoot"

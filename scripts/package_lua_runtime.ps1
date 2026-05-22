param(
    # Target platform key used in archive and manifest names.
    # 用于归档文件与清单文件命名的目标平台标识。
    [string]$Platform = "",
    # Source third_party directory produced by the build pipeline.
    # 构建流水线生成的 third_party 源目录。
    [string]$ThirdPartyDir = "third_party",
    # Runtime staging directory assembled before compression.
    # 压缩前用于组装运行期目录的暂存目录。
    [string]$StagingDir = "target\lua-runtime-package",
    # Output directory that receives the final runtime archive.
    # 接收最终 runtime 压缩包的输出目录。
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

# NativeRuntimeExtensions lists only files that are meaningful at runtime.
# NativeRuntimeExtensions 只包含运行期真正需要的原生库扩展名。
$NativeRuntimeExtensions = @("*.dll", "*.so", "*.so.*", "*.dylib")

# ExcludedRuntimeLibraryNames prevents build-only LuaJIT shims from leaking into runtime packages.
# ExcludedRuntimeLibraryNames 防止仅用于构建的 LuaJIT 兼容库泄漏到运行期包中。
$ExcludedRuntimeLibraryNames = @("lua51.dll", "luajit.exe", "lua.exe")

# BundledNativeDependencyPatterns identifies system-linked runtime libraries that must travel with packages.
# BundledNativeDependencyPatterns 标识需要随包携带的系统链接运行库。
$BundledNativeDependencyPatterns = @(
    "libz.so*",
    "zlib*.dll",
    "libz*.dylib",
    "libcurl.so*",
    "libcurl*.dll",
    "libcurl*.dylib",
    "libssl.so*",
    "libssl*.dll",
    "libssl*.dylib",
    "libcrypto.so*",
    "libcrypto*.dll",
    "libcrypto*.dylib",
    "libpcre2-*.so*",
    "pcre2*.dll",
    "libpcre2-*.dylib",
    "libyaml*.so*",
    "yaml*.dll",
    "libyaml*.dylib"
)

$script:BundledLibraries = [System.Collections.Generic.List[object]]::new()

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

function Copy-DirectoryContent {
    <#
    .SYNOPSIS
    Copy all children from one directory to another directory.
    将一个目录下的全部子项复制到另一个目录。

    .PARAMETER Source
    Existing source directory.
    已存在的源目录。

    .PARAMETER Destination
    Destination directory to create and populate.
    需要创建并填充的目标目录。
    #>
    param(
        [string]$Source,
        [string]$Destination
    )

    if (-not (Test-Path -LiteralPath $Source)) {
        return
    }

    Ensure-Dir $Destination
    Copy-Item -Recurse -Force -Path (Join-Path $Source "*") -Destination $Destination -ErrorAction SilentlyContinue
}

function Copy-LuaPackagesRuntimeTree {
    <#
    .SYNOPSIS
    Copy only LuaRocks runtime directories into the package.
    仅将 LuaRocks 运行期目录复制到产物包。

    .PARAMETER LuaPackagesDir
    Source LuaRocks tree under third_party.
    third_party 下的 LuaRocks 源目录。

    .PARAMETER RuntimeRoot
    Runtime package root directory.
    runtime 包根目录。
    #>
    param(
        [string]$LuaPackagesDir,
        [string]$RuntimeRoot
    )

    $RuntimeLuaPackages = Join-Path $RuntimeRoot "lua_packages"
    Copy-LuaRocksRuntimeDirectory -Source (Join-Path $LuaPackagesDir "lib\lua") -Destination (Join-Path $RuntimeLuaPackages "lib\lua")
    Copy-LuaRocksRuntimeDirectory -Source (Join-Path $LuaPackagesDir "share\lua") -Destination (Join-Path $RuntimeLuaPackages "share\lua")
}

function Copy-LuaRocksRuntimeDirectory {
    <#
    .SYNOPSIS
    Flatten LuaRocks' Lua 5.1 ABI directory into the runtime default layout.
    将 LuaRocks 的 Lua 5.1 ABI 目录扁平化到 runtime 默认布局。
    #>
    param(
        [string]$Source,
        [string]$Destination
    )

    if (-not (Test-Path -LiteralPath $Source)) {
        return
    }

    Ensure-Dir $Destination
    $VersionedSource = Join-Path $Source "5.1"
    if (Test-Path -LiteralPath $VersionedSource) {
        Copy-Item -Recurse -Force -Path (Join-Path $VersionedSource "*") -Destination $Destination -ErrorAction SilentlyContinue
    }

    Get-ChildItem -Force -LiteralPath $Source -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -ne "5.1" } |
        ForEach-Object {
            Copy-Item -Recurse -Force -LiteralPath $_.FullName -Destination $Destination
        }
}

function Copy-NativeRuntimeLibraries {
    <#
    .SYNOPSIS
    Copy native runtime libraries and skip build-only LuaJIT compatibility files.
    复制原生运行库，并跳过仅用于构建的 LuaJIT 兼容文件。

    .PARAMETER DepsDir
    Source native dependency directory.
    原生依赖源目录。

    .PARAMETER RuntimeRoot
    Runtime package root directory.
    runtime 包根目录。
    #>
    param(
        [string]$DepsDir,
        [string]$RuntimeRoot
    )

    $LibsDir = Join-Path $RuntimeRoot "libs"
    Ensure-Dir $LibsDir

    if (-not (Test-Path -LiteralPath $DepsDir)) {
        return
    }

    foreach ($Extension in $NativeRuntimeExtensions) {
        Get-ChildItem -Recurse -File -Path $DepsDir -Filter $Extension -ErrorAction SilentlyContinue | ForEach-Object {
            $Name = $_.Name.ToLowerInvariant()
            if ($ExcludedRuntimeLibraryNames -contains $Name) {
                return
            }
            $Destination = Join-Path $LibsDir $_.Name
            Copy-Item -Force -LiteralPath $_.FullName -Destination $Destination
            Add-BundledLibraryRecord -SourcePath $_.FullName -DestinationPath $Destination
        }
    }
}

function Test-BundledNativeDependencyName {
    <#
    .SYNOPSIS
    Check whether a native dependency name should be bundled into runtime libs.
    检查原生依赖名称是否应该打入 runtime libs。

    .PARAMETER Name
    File name to test against the allowlist.
    需要匹配白名单的文件名。

    .OUTPUTS
    Boolean value indicating whether the file should be copied.
    表示该文件是否需要复制的布尔值。
    #>
    param([string]$Name)

    foreach ($Pattern in $BundledNativeDependencyPatterns) {
        if ($Name -like $Pattern) {
            return $true
        }
    }
    return $false
}

function Get-NativeDependencyComponent {
    <#
    .SYNOPSIS
    Map a native library filename to its component name.
    将原生库文件名映射到组件名称。
    #>
    param([string]$Name)

    $Lower = $Name.ToLowerInvariant()
    if ($Lower -like "libz.so*" -or $Lower -like "zlib*.dll" -or $Lower -like "libz*.dylib") { return "zlib" }
    if ($Lower -like "libcurl.so*" -or $Lower -like "libcurl*.dll" -or $Lower -like "libcurl*.dylib") { return "curl" }
    if ($Lower -like "libssl.so*" -or $Lower -like "libssl*.dll" -or $Lower -like "libssl*.dylib" -or $Lower -like "libcrypto.so*" -or $Lower -like "libcrypto*.dll" -or $Lower -like "libcrypto*.dylib") { return "openssl" }
    if ($Lower -like "libpcre2-*.so*" -or $Lower -like "pcre2*.dll" -or $Lower -like "libpcre2-*.dylib") { return "pcre2" }
    if ($Lower -like "libyaml*.so*" -or $Lower -like "yaml*.dll" -or $Lower -like "libyaml*.dylib") { return "libyaml" }
    return "unknown"
}

function Add-BundledLibraryRecord {
    <#
    .SYNOPSIS
    Record one copied runtime library source path for manifests and license references.
    记录一个已复制运行库的来源路径，用于清单与授权引用。
    #>
    param(
        [string]$SourcePath,
        [string]$DestinationPath
    )

    $Name = Split-Path -Leaf $DestinationPath
    $script:BundledLibraries.Add([ordered]@{
        name = $Name
        component = Get-NativeDependencyComponent -Name $Name
        source_path = $SourcePath
    }) | Out-Null
}

function Get-LinkedDependencyPaths {
    <#
    .SYNOPSIS
    Read linked native dependency paths from ldd or otool.
    通过 ldd 或 otool 读取已链接的原生依赖路径。

    .PARAMETER BinaryPath
    Native binary to inspect.
    需要检查的原生二进制文件。

    .OUTPUTS
    Absolute file paths reported by the platform dependency tool.
    平台依赖工具报告的绝对文件路径。
    #>
    param([string]$BinaryPath)

    $Ldd = Get-Command ldd -ErrorAction SilentlyContinue
    if ($Ldd) {
        & $Ldd.Source $BinaryPath 2>$null | ForEach-Object {
            $Line = $_.Trim()
            if ($Line -match '=>\s+(/\S+)') {
                $Matches[1]
            } elseif ($Line -match '^(/\S+)') {
                $Matches[1]
            }
        }
        return
    }

    $Otool = Get-Command otool -ErrorAction SilentlyContinue
    if ($Otool) {
        & $Otool.Source -L $BinaryPath 2>$null | Select-Object -Skip 1 | ForEach-Object {
            $Line = $_.Trim()
            if ($Line -match '^(/\S+)') {
                $Matches[1]
            }
        }
    }
}

function Copy-LinkedRuntimeDependencies {
    <#
    .SYNOPSIS
    Iteratively copy allowlisted linked system libraries into runtime libs.
    迭代复制白名单内的已链接系统库到 runtime libs。

    .PARAMETER ScanRoot
    Directory that contains native binaries to inspect.
    包含待检查原生二进制文件的目录。

    .PARAMETER LibsDir
    Destination libs directory.
    目标 libs 目录。
    #>
    param(
        [string]$ScanRoot,
        [string]$LibsDir
    )

    if (-not (Test-Path -LiteralPath $ScanRoot)) {
        return
    }

    Ensure-Dir $LibsDir
    $Queue = [System.Collections.Generic.Queue[string]]::new()
    $Seen = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)

    foreach ($Root in @($ScanRoot, $LibsDir)) {
        if (-not (Test-Path -LiteralPath $Root)) {
            continue
        }
        foreach ($Extension in $NativeRuntimeExtensions) {
            Get-ChildItem -Recurse -File -Path $Root -Filter $Extension -ErrorAction SilentlyContinue | ForEach-Object {
                $Queue.Enqueue($_.FullName)
            }
        }
    }

    while ($Queue.Count -gt 0) {
        $BinaryPath = $Queue.Dequeue()
        if (-not (Test-Path -LiteralPath $BinaryPath)) {
            continue
        }
        if (-not $Seen.Add($BinaryPath)) {
            continue
        }

        foreach ($DependencyPath in (Get-LinkedDependencyPaths -BinaryPath $BinaryPath)) {
            if (-not $DependencyPath -or -not (Test-Path -LiteralPath $DependencyPath)) {
                continue
            }
            $DependencyName = Split-Path -Leaf $DependencyPath
            if (Test-BundledNativeDependencyName -Name $DependencyName) {
                $Destination = Join-Path $LibsDir $DependencyName
                if (-not (Test-Path -LiteralPath $Destination)) {
                    Copy-Item -Force -LiteralPath $DependencyPath -Destination $Destination
                    Add-BundledLibraryRecord -SourcePath $DependencyPath -DestinationPath $Destination
                    $Queue.Enqueue($Destination)
                }
            }
        }
    }
}

function Copy-LicenseCandidates {
    <#
    .SYNOPSIS
    Copy available license-like files for one component into the package.
    将某个组件可发现的授权文件复制到产物包。

    .PARAMETER ComponentName
    Component directory name under licenses.
    licenses 下的组件目录名。

    .PARAMETER SearchRoots
    Directories to scan for license files.
    需要扫描授权文件的目录集合。

    .PARAMETER LicenseRoot
    Package license root directory.
    产物包授权根目录。
    #>
    param(
        [string]$ComponentName,
        [string[]]$SearchRoots,
        [string]$LicenseRoot
    )

    $ComponentDir = Join-Path $LicenseRoot $ComponentName
    Ensure-Dir $ComponentDir
    $ResolvedLicenseRoot = (Resolve-Path -LiteralPath $LicenseRoot).Path.TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
    $ResolvedComponentDir = (Resolve-Path -LiteralPath $ComponentDir).Path.TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar)
    $LicenseRootPrefix = $ResolvedLicenseRoot + [System.IO.Path]::DirectorySeparatorChar
    $ComponentDirPrefix = $ResolvedComponentDir + [System.IO.Path]::DirectorySeparatorChar

    foreach ($SearchRoot in $SearchRoots) {
        if (-not (Test-Path -LiteralPath $SearchRoot)) {
            continue
        }

        Get-ChildItem -Recurse -File -Path $SearchRoot -Depth 5 -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)(\.|$)' } |
            ForEach-Object {
                $SourceFullPath = [System.IO.Path]::GetFullPath($_.FullName)
                $DestinationPath = Join-Path $ComponentDir $_.Name
                $DestinationFullPath = [System.IO.Path]::GetFullPath($DestinationPath)
                $CopiesIntoItself = $SourceFullPath.Equals($DestinationFullPath, [System.StringComparison]::OrdinalIgnoreCase)
                $CopiesFromPackageLicenses = $SourceFullPath.StartsWith($ComponentDirPrefix, [System.StringComparison]::OrdinalIgnoreCase) -or $SourceFullPath.StartsWith($LicenseRootPrefix, [System.StringComparison]::OrdinalIgnoreCase)
                if (-not $CopiesIntoItself -and -not $CopiesFromPackageLicenses) {
                    Copy-Item -Force -LiteralPath $_.FullName -Destination $DestinationPath
                }
            }
    }
}

function Save-OfficialLicense {
    <#
    .SYNOPSIS
    Download one official license file for a fixed native dependency.
    为固定原生依赖下载一个官方授权文件。
    #>
    param(
        [string]$ComponentName,
        [string]$FileName,
        [string]$Url,
        [string]$LicenseRoot
    )

    $ComponentDir = Join-Path $LicenseRoot ("native\" + $ComponentName)
    Ensure-Dir $ComponentDir
    $Destination = Join-Path $ComponentDir $FileName
    Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing
    Set-Content -Path (Join-Path $ComponentDir "$FileName.url.txt") -Value $Url -Encoding UTF8
}

function Save-OfficialNativeLicenses {
    <#
    .SYNOPSIS
    Always include official licenses for the fixed native dependency set.
    始终为固定原生依赖集合带入官方授权文件。
    #>
    param([string]$LicenseRoot)

    Save-OfficialLicense -ComponentName "openssl" -FileName "LICENSE.official.txt" -Url "https://raw.githubusercontent.com/openssl/openssl/openssl-3.4.1/LICENSE.txt" -LicenseRoot $LicenseRoot
    Save-OfficialLicense -ComponentName "curl" -FileName "COPYING.official.txt" -Url "https://raw.githubusercontent.com/curl/curl/curl-8_13_0/COPYING" -LicenseRoot $LicenseRoot
    Save-OfficialLicense -ComponentName "zlib" -FileName "LICENSE.official.txt" -Url "https://raw.githubusercontent.com/madler/zlib/v1.3.1/LICENSE" -LicenseRoot $LicenseRoot
    Save-OfficialLicense -ComponentName "pcre2" -FileName "LICENCE.official.md" -Url "https://raw.githubusercontent.com/PCRE2Project/pcre2/pcre2-10.45/LICENCE.md" -LicenseRoot $LicenseRoot
    Save-OfficialLicense -ComponentName "libyaml" -FileName "License.official" -Url "https://raw.githubusercontent.com/yaml/libyaml/0.2.5/License" -LicenseRoot $LicenseRoot
}

function Get-RockspecField {
    <#
    .SYNOPSIS
    Extract one common string field from a LuaRocks rockspec.
    从 LuaRocks rockspec 中提取常见字符串字段。
    #>
    param(
        [string]$RockspecPath,
        [string]$FieldName
    )

    if (-not (Test-Path -LiteralPath $RockspecPath)) {
        return ""
    }

    $Text = Get-Content -Raw -Path $RockspecPath
    $Pattern = "\b$([regex]::Escape($FieldName))\s*=\s*['""]([^'""]+)['""]"
    $Match = [regex]::Match($Text, $Pattern)
    if ($Match.Success) {
        return $Match.Groups[1].Value
    }
    return ""
}

function Copy-LuaRocksLicenseMetadata {
    <#
    .SYNOPSIS
    Preserve license metadata for every installed LuaRocks package.
    为每个已安装 LuaRocks 包保留授权元数据。
    #>
    param(
        [string]$LuaPackagesDir,
        [string]$LicenseRoot
    )

    $RocksRoot = Join-Path $LuaPackagesDir "lib\luarocks\rocks-5.1"
    if (-not (Test-Path -LiteralPath $RocksRoot)) {
        return
    }

    $LuaRocksLicenseRoot = Join-Path $LicenseRoot "luarocks"
    Ensure-Dir $LuaRocksLicenseRoot
    $ManifestRows = [System.Collections.Generic.List[string]]::new()

    Get-ChildItem -Directory -LiteralPath $RocksRoot -ErrorAction SilentlyContinue | ForEach-Object {
        $RockName = $_.Name
        Get-ChildItem -Directory -LiteralPath $_.FullName -ErrorAction SilentlyContinue | ForEach-Object {
            $Version = $_.Name
            $VersionDir = $_.FullName
            $Destination = Join-Path $LuaRocksLicenseRoot $RockName
            Ensure-Dir $Destination
            Copy-LicenseCandidates -ComponentName ("luarocks\" + $RockName) -SearchRoots @($VersionDir) -LicenseRoot $LicenseRoot
            $Rockspec = Get-ChildItem -File -LiteralPath $VersionDir -Filter "*.rockspec" -ErrorAction SilentlyContinue | Select-Object -First 1
            $License = ""
            $SourceUrl = ""
            $Homepage = ""
            $RockspecName = ""
            if ($Rockspec) {
                Copy-Item -Force -LiteralPath $Rockspec.FullName -Destination (Join-Path $Destination $Rockspec.Name)
                $License = Get-RockspecField -RockspecPath $Rockspec.FullName -FieldName "license"
                $SourceUrl = Get-RockspecField -RockspecPath $Rockspec.FullName -FieldName "url"
                $Homepage = Get-RockspecField -RockspecPath $Rockspec.FullName -FieldName "homepage"
                $RockspecName = $Rockspec.Name
            }
            if (-not $License) {
                $License = "See rockspec or upstream package"
            }
            @"
Package: $RockName
Version: $Version
License: $License
Source: $SourceUrl
Homepage: $Homepage
Rockspec: $RockspecName
"@ | Set-Content -Path (Join-Path $Destination "LICENSE.metadata.txt") -Encoding UTF8
            $ManifestRows.Add("$RockName`t$Version`t$License`t$SourceUrl`t$Homepage") | Out-Null
        }
    }

    $ManifestRows | Set-Content -Path (Join-Path $LuaRocksLicenseRoot "manifest.tsv") -Encoding UTF8
}

function Write-LicenseReferenceIfMissing {
    <#
    .SYNOPSIS
    Write a license reference when the copied system library has no nearby license file.
    当复制的系统库没有随源目录提供授权文件时写入授权引用。
    #>
    param(
        [string]$ComponentName,
        [string]$SourcePath,
        [string]$LicenseRoot
    )

    if (-not $ComponentName -or $ComponentName -eq "unknown") {
        return
    }

    $ComponentDir = Join-Path $LicenseRoot ("native\" + $ComponentName)
    Ensure-Dir $ComponentDir
    $Existing = Get-ChildItem -File -Path $ComponentDir -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|README)(\.|$)' } |
        Select-Object -First 1
    if ($Existing) {
        return
    }

    $License = switch ($ComponentName) {
        "openssl" { "Apache-2.0" }
        "curl" { "curl" }
        "zlib" { "Zlib" }
        "pcre2" { "BSD-3-Clause" }
        "libyaml" { "MIT" }
        default { "See upstream project" }
    }

    @"
Component: $ComponentName
License: $License
Bundled library source path: $SourcePath

No license file was found next to the copied system library during packaging.
This package records the upstream license identifier and the source path used by the build runner.
"@ | Set-Content -Path (Join-Path $ComponentDir "LICENSE.reference.txt") -Encoding UTF8
}

function Test-WindowsPackagePlatform {
    <#
    .SYNOPSIS
    Check whether one package platform key targets Windows.
    检查一个包平台标识是否面向 Windows。

    .PARAMETER PlatformKey
    Platform key such as windows-x64, linux-x64, or macos-arm64.
    形如 windows-x64、linux-x64 或 macos-arm64 的平台标识。

    .OUTPUTS
    Boolean value indicating whether the platform is Windows.
    表示平台是否为 Windows 的布尔值。
    #>
    param([string]$PlatformKey)

    return $PlatformKey -like "windows-*"
}

function Write-RuntimeEnvScripts {
    <#
    .SYNOPSIS
    Write helper scripts that let hosts include runtime/libs in the native loader path.
    写入帮助宿主把 runtime/libs 加入原生加载路径的辅助脚本。

    .PARAMETER RuntimeRoot
    Runtime package root that receives the helper script.
    接收辅助脚本的 runtime 包根目录。

    .PARAMETER Platform
    Target package platform used to choose PowerShell or shell helpers.
    用于选择 PowerShell 或 shell 辅助脚本的目标包平台。
    #>
    param(
        [string]$RuntimeRoot,
        [string]$Platform
    )

    $ResourcesDir = Join-Path $RuntimeRoot "resources"
    Ensure-Dir $ResourcesDir

    if (Test-WindowsPackagePlatform -PlatformKey $Platform) {
        @'
$RuntimeRoot = if ($env:RUNTIME_ROOT) { $env:RUNTIME_ROOT } else { Split-Path -Parent $PSScriptRoot }
$Libs = Join-Path $RuntimeRoot "libs"
$env:PATH = "$Libs;$env:PATH"
'@ | Set-Content -Path (Join-Path $ResourcesDir "runtime-env.ps1") -Encoding UTF8
        return
    }

    @'
#!/usr/bin/env bash
RUNTIME_ROOT="${RUNTIME_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
case "$(uname -s)" in
  Darwin) export DYLD_LIBRARY_PATH="$RUNTIME_ROOT/libs${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" ;;
  Linux) export LD_LIBRARY_PATH="$RUNTIME_ROOT/libs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" ;;
esac
'@ | Set-Content -Path (Join-Path $ResourcesDir "runtime-env.sh") -Encoding UTF8
}

function Write-JsonFile {
    <#
    .SYNOPSIS
    Write one object as pretty JSON.
    将对象以格式化 JSON 写入文件。

    .PARAMETER Path
    Destination JSON file path.
    目标 JSON 文件路径。

    .PARAMETER Value
    Object to serialize.
    需要序列化的对象。
    #>
    param(
        [string]$Path,
        [object]$Value
    )

    ConvertTo-Json -InputObject $Value -Depth 12 | Set-Content -Path $Path -Encoding UTF8
}

function New-TarFromDirectory {
    <#
    .SYNOPSIS
    Archive top-level children without adding a leading ./ entry.
    按一级子项打包，避免归档内出现 ./ 前缀。
    #>
    param(
        [string]$SourceDir,
        [string]$ArchivePath
    )

    $Members = @(Get-ChildItem -Force -LiteralPath $SourceDir | ForEach-Object { $_.Name })
    if (-not $Members -or $Members.Count -eq 0) {
        throw "Cannot create archive from empty directory: $SourceDir"
    }

    Push-Location $SourceDir
    try {
        tar -czf $ArchivePath @Members
    } finally {
        Pop-Location
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

$ThirdPartyPath = Resolve-Path -LiteralPath $ThirdPartyDir -ErrorAction SilentlyContinue
if (-not $ThirdPartyPath) {
    throw "Third-party directory not found: $ThirdPartyDir"
}

$RuntimeRoot = Join-Path $StagingDir "lua-runtime"
if (Test-Path -LiteralPath $RuntimeRoot) {
    Remove-Item -LiteralPath $RuntimeRoot -Recurse -Force
}

Ensure-Dir $RuntimeRoot
Ensure-Dir (Join-Path $RuntimeRoot "resources")
Ensure-Dir (Join-Path $RuntimeRoot "licenses")
Ensure-Dir $OutputDir

Copy-LuaPackagesRuntimeTree -LuaPackagesDir (Join-Path $ThirdPartyPath "lua_packages") -RuntimeRoot $RuntimeRoot
Copy-NativeRuntimeLibraries -DepsDir (Join-Path $ThirdPartyPath "deps") -RuntimeRoot $RuntimeRoot
Copy-LinkedRuntimeDependencies -ScanRoot $RuntimeRoot -LibsDir (Join-Path $RuntimeRoot "libs")
Copy-LinkedRuntimeDependencies -ScanRoot (Join-Path $ProjectRoot "target\release") -LibsDir (Join-Path $RuntimeRoot "libs")

Write-RuntimeEnvScripts -RuntimeRoot $RuntimeRoot -Platform $Platform
Copy-LicenseCandidates -ComponentName "luaskills" -SearchRoots @($ProjectRoot) -LicenseRoot (Join-Path $RuntimeRoot "licenses")

$NativeLicenseRoots = @(
    @{ name = "openssl"; roots = @("openssl-*", "deps\openssl", "target\lua_deps_build\openssl") },
    @{ name = "curl"; roots = @("curl-*", "deps\curl", "target\lua_deps_build\curl") },
    @{ name = "zlib"; roots = @("zlib-*", "deps\zlib", "target\lua_deps_build\zlib") },
    @{ name = "pcre2"; roots = @("pcre2-*", "deps\pcre2", "target\lua_deps_build\pcre2") },
    @{ name = "libyaml"; roots = @("yaml-*", "libyaml-*", "deps\libyaml", "target\lua_deps_build\libyaml") }
)

foreach ($Component in $NativeLicenseRoots) {
    $Roots = @()
    foreach ($RootPattern in $Component.roots) {
        if ($RootPattern -like "*\*" -or $RootPattern -like "*/*") {
            $ProjectCandidate = Join-Path $ProjectRoot $RootPattern
            if (Test-Path -LiteralPath $ProjectCandidate) {
                $Roots += $ProjectCandidate
            }
            $ThirdPartyCandidate = Join-Path $ThirdPartyPath $RootPattern
            if (Test-Path -LiteralPath $ThirdPartyCandidate) {
                $Roots += $ThirdPartyCandidate
            }
        } else {
            $Roots += Get-ChildItem -Path $ProjectRoot -Directory -Filter $RootPattern -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName }
            $Candidate = Join-Path $ThirdPartyPath $RootPattern
            if (Test-Path -LiteralPath $Candidate) {
                $Roots += $Candidate
            }
        }
    }
    Copy-LicenseCandidates -ComponentName ("native\" + $Component.name) -SearchRoots $Roots -LicenseRoot (Join-Path $RuntimeRoot "licenses")
}

Save-OfficialNativeLicenses -LicenseRoot (Join-Path $RuntimeRoot "licenses")
Copy-LuaRocksLicenseMetadata -LuaPackagesDir (Join-Path $ThirdPartyPath "lua_packages") -LicenseRoot (Join-Path $RuntimeRoot "licenses")

foreach ($Library in ($script:BundledLibraries | Sort-Object name, component, source_path -Unique)) {
    Write-LicenseReferenceIfMissing -ComponentName $Library.component -SourcePath $Library.source_path -LicenseRoot (Join-Path $RuntimeRoot "licenses")
}

$RuntimeManifest = [ordered]@{
    schema_version = 1
    package_name = "lua-runtime-$Platform"
    platform = $Platform
    layout = "luaskills-runtime-v1"
    exports = @("lua_packages/lib/lua", "lua_packages/share/lua", "libs", "resources", "licenses")
    packages_manifest = "resources/luaskills-packages-manifest.json"
    loader_env = [ordered]@{
        linux = "LD_LIBRARY_PATH=<runtime>/libs"
        macos = "DYLD_LIBRARY_PATH=<runtime>/libs"
        windows = "PATH=<runtime>\libs;%PATH%"
    }
    excludes = @("third_party/tools", "third_party/luarocks", "third_party/luajit", "lua51.dll", "luajit.exe", "build directories")
}

$LicenseManifest = [ordered]@{
    schema_version = 1
    package_name = "lua-runtime-$Platform"
    components = @(
        @{ name = "luaskills"; type = "runtime"; license = "MIT"; license_files = @("licenses/luaskills/LICENSE") },
        @{ name = "openssl"; type = "native-lib"; license = "Apache-2.0"; license_files = @("licenses/native/openssl") },
        @{ name = "curl"; type = "native-lib"; license = "curl"; license_files = @("licenses/native/curl") },
        @{ name = "zlib"; type = "native-lib"; license = "Zlib"; license_files = @("licenses/native/zlib") },
        @{ name = "pcre2"; type = "native-lib"; license = "BSD-3-Clause"; license_files = @("licenses/native/pcre2") },
        @{ name = "libyaml"; type = "native-lib"; license = "MIT"; license_files = @("licenses/native/libyaml") },
        @{ name = "luarocks-packages"; type = "lua-rocks"; license = "per-rockspec"; license_files = @("licenses/luarocks") }
    )
}

Write-JsonFile -Path (Join-Path $RuntimeRoot "resources\lua-runtime-manifest.json") -Value $RuntimeManifest
Write-JsonFile -Path (Join-Path $RuntimeRoot "resources\bundled-libs.json") -Value @($script:BundledLibraries | Sort-Object name, component, source_path -Unique)
Write-JsonFile -Path (Join-Path $RuntimeRoot "licenses\manifest.json") -Value $LicenseManifest

# Generate the runtime-facing luaskills-packages metadata tree after license manifests exist.
# 在授权清单就绪后生成面向运行时的 luaskills-packages 元数据目录树。
python (Join-Path $ProjectRoot "scripts\generate_runtime_packages_metadata.py") `
    --project-root $ProjectRoot `
    --runtime-root $RuntimeRoot `
    --platform $Platform

$ArchiveName = "lua-runtime-$Platform.tar.gz"
$ArchivePath = Join-Path $OutputDir $ArchiveName
if (Test-Path -LiteralPath $ArchivePath) {
    Remove-Item -LiteralPath $ArchivePath -Force
}

$ResolvedOutput = (Resolve-Path -LiteralPath $OutputDir).Path
New-TarFromDirectory -SourceDir $RuntimeRoot -ArchivePath (Join-Path $ResolvedOutput $ArchiveName)

Write-Host "Lua runtime package created: $ArchivePath"

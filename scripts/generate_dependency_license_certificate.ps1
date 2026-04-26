param(
    # Output directory for generated dependency license certificate files.
    # 生成依赖许可证证明文件的输出目录。
    [string]$OutputDir = "target/license-certificate",

    # Skip cargo-deny license validation before certificate generation.
    # 生成证明前跳过 cargo-deny 许可证校验。
    [switch]$SkipCheck
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Invokes a native command and fails when the process exits unsuccessfully.
# 调用原生命令，并在进程失败退出时终止脚本。
function Invoke-CheckedCommand {
    param(
        # Native executable name or path.
        # 原生命令名称或路径。
        [Parameter(Mandatory = $true)]
        [string]$FilePath,

        # Argument list passed to the executable.
        # 传递给可执行文件的参数列表。
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    # Keep stdout as the return value while preserving native exit-code checks.
    # 将标准输出作为返回值，同时保留原生命令退出码校验。
    $output = & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $FilePath $($Arguments -join ' ')"
    }

    return $output
}

# Parses one cargo-deny dependency entry into structured package data.
# 将一条 cargo-deny 依赖记录解析为结构化包数据。
function Convert-DependencyEntry {
    param(
        # Raw dependency entry emitted by cargo-deny.
        # cargo-deny 输出的原始依赖记录。
        [Parameter(Mandatory = $true)]
        [string]$Entry
    )

    # cargo-deny emits entries as "name version source"; split only the stable prefix.
    # cargo-deny 以“名称 版本 来源”输出记录；这里只拆分稳定前缀。
    if ($Entry -match '^(\S+)\s+(\S+)\s+(.+)$') {
        return [PSCustomObject]@{
            name = $Matches[1]
            version = $Matches[2]
            source = $Matches[3]
        }
    }

    return [PSCustomObject]@{
        name = $Entry
        version = ""
        source = ""
    }
}

# Converts cargo-deny license rows into certificate-friendly objects.
# 将 cargo-deny 许可证行转换为证明文件友好的对象。
function Convert-LicenseRows {
    param(
        # Raw license rows from cargo-deny JSON output.
        # cargo-deny JSON 输出中的原始许可证行。
        [Parameter(Mandatory = $true)]
        [object[]]$Rows
    )

    # Normalize the compact array shape: [license, [dependency entries...]].
    # 规范化紧凑数组结构：[许可证，[依赖记录...]]。
    return @(
        foreach ($row in $Rows) {
            $licenseName = [string]$row[0]
            $dependencies = @(
                foreach ($entry in @($row[1])) {
                    Convert-DependencyEntry -Entry ([string]$entry)
                }
            )

            [PSCustomObject]@{
                license = $licenseName
                dependency_count = $dependencies.Count
                dependencies = $dependencies
            }
        }
    )
}

# Reads cargo-deny license clarification entries from deny.toml.
# 从 deny.toml 读取 cargo-deny 许可证澄清项。
function Get-LicenseClarifications {
    param(
        # Path to the cargo-deny configuration file.
        # cargo-deny 配置文件路径。
        [Parameter(Mandatory = $true)]
        [string]$ConfigPath
    )

    if (-not (Test-Path -Path $ConfigPath)) {
        return @()
    }

    # Parse only the small clarification shape this script needs.
    # 仅解析脚本所需的澄清配置小结构。
    $content = Get-Content -Path $ConfigPath -Raw
    $blocks = [regex]::Split($content, '(?m)^\s*\[\[licenses\.clarify\]\]\s*$') | Select-Object -Skip 1
    return @(
        foreach ($block in $blocks) {
            $crateName = $null
            $expression = $null

            if ($block -match '(?m)^\s*crate\s*=\s*"([^"]+)"\s*$') {
                $crateName = $Matches[1]
            }

            if ($block -match '(?m)^\s*expression\s*=\s*"([^"]+)"\s*$') {
                $expression = $Matches[1]
            }

            if ($crateName -and $expression) {
                [PSCustomObject]@{
                    crate = $crateName
                    expression = $expression
                }
            }
        }
    )
}

# Applies cargo-deny clarification entries to the compact license list output.
# 将 cargo-deny 澄清项应用到紧凑许可证清单输出。
function Resolve-ClarifiedUnlicensedEntries {
    param(
        # License groups converted from cargo-deny list output.
        # 从 cargo-deny list 输出转换得到的许可证分组。
        [Parameter(Mandatory = $true)]
        [object[]]$Licenses,

        # Raw unlicensed entries from cargo-deny list output.
        # cargo-deny list 输出中的原始未授权记录。
        [Parameter(Mandatory = $true)]
        [object[]]$Unlicensed,

        # Clarification entries parsed from deny.toml.
        # 从 deny.toml 解析出的澄清记录。
        [Parameter(Mandatory = $true)]
        [object[]]$Clarifications
    )

    # Clone license groups so normalization does not mutate caller-owned objects.
    # 克隆许可证分组，避免规范化过程修改调用方持有的对象。
    $normalizedLicenses = [System.Collections.Generic.List[object]]::new()
    foreach ($licenseGroup in $Licenses) {
        $normalizedLicenses.Add([PSCustomObject]@{
            license = $licenseGroup.license
            dependency_count = $licenseGroup.dependency_count
            dependencies = @($licenseGroup.dependencies)
        })
    }

    $remainingUnlicensed = [System.Collections.Generic.List[string]]::new()
    $clarifiedEntries = [System.Collections.Generic.List[object]]::new()

    foreach ($entry in $Unlicensed) {
        $entryText = [string]$entry
        $clarification = $Clarifications | Where-Object { $entryText -like "$($_.crate) *" } | Select-Object -First 1

        if ($null -eq $clarification) {
            $remainingUnlicensed.Add($entryText)
            continue
        }

        $dependency = Convert-DependencyEntry -Entry $entryText
        $targetGroup = $normalizedLicenses | Where-Object { $_.license -eq $clarification.expression } | Select-Object -First 1
        if ($null -eq $targetGroup) {
            $targetGroup = [PSCustomObject]@{
                license = $clarification.expression
                dependency_count = 0
                dependencies = @()
            }
            $normalizedLicenses.Add($targetGroup)
        }

        $targetGroup.dependencies = @($targetGroup.dependencies) + @($dependency)
        $targetGroup.dependency_count = @($targetGroup.dependencies).Count
        $clarifiedEntries.Add([PSCustomObject]@{
            entry = $entryText
            license = $clarification.expression
        })
    }

    return [PSCustomObject]@{
        licenses = @($normalizedLicenses)
        unlicensed = @($remainingUnlicensed)
        clarified = @($clarifiedEntries)
    }
}

# Writes the Markdown dependency license certificate.
# 写入 Markdown 格式的依赖许可证证明。
function Write-LicenseCertificateMarkdown {
    param(
        # Certificate object used as the Markdown source.
        # 作为 Markdown 来源的证明对象。
        [Parameter(Mandatory = $true)]
        [object]$Certificate,

        # Target Markdown output path.
        # Markdown 输出目标路径。
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    # Build the document with explicit lines to keep output deterministic.
    # 使用显式行构建文档，确保输出稳定可复现。
    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add("# Dependency License Certificate")
    $lines.Add("")
    $lines.Add("- Project: $($Certificate.project.name) $($Certificate.project.version)")
    $lines.Add("- Generated At UTC: $($Certificate.generated_at_utc)")
    $lines.Add("- Cargo Deny Version: $($Certificate.cargo_deny_version)")
    $lines.Add("- License Check Status: $($Certificate.license_check_status)")
    $lines.Add("- License Count: $($Certificate.license_count)")
    $lines.Add("- Dependency Count: $($Certificate.dependency_count)")
    $lines.Add("- Unlicensed Count: $($Certificate.unlicensed_count)")
    $lines.Add("- Clarified Count: $($Certificate.clarified_count)")
    $lines.Add("")
    $lines.Add("## Licenses")

    foreach ($licenseGroup in $Certificate.licenses) {
        $lines.Add("")
        $lines.Add("### $($licenseGroup.license)")
        $lines.Add("")
        foreach ($dependency in $licenseGroup.dependencies) {
            $lines.Add("- $($dependency.name) $($dependency.version) - $($dependency.source)")
        }
    }

    if ($Certificate.unlicensed_count -gt 0) {
        $lines.Add("")
        $lines.Add("## Unlicensed")
        $lines.Add("")
        foreach ($entry in $Certificate.unlicensed) {
            $lines.Add("- $entry")
        }
    }

    if ($Certificate.clarified_count -gt 0) {
        $lines.Add("")
        $lines.Add("## Clarified")
        $lines.Add("")
        foreach ($entry in $Certificate.clarified) {
            $lines.Add("- $($entry.entry) => $($entry.license)")
        }
    }

    Set-Content -Path $Path -Value $lines -Encoding UTF8
}

# Creates a dependency license certificate from cargo-deny output.
# 基于 cargo-deny 输出创建依赖许可证证明。
function New-DependencyLicenseCertificate {
    param(
        # Target output directory for all generated files.
        # 所有生成文件的目标输出目录。
        [Parameter(Mandatory = $true)]
        [string]$TargetOutputDir,

        # Whether to skip the cargo-deny check step.
        # 是否跳过 cargo-deny 校验步骤。
        [Parameter(Mandatory = $true)]
        [bool]$ShouldSkipCheck
    )

    # Ensure cargo-deny is available before doing any generation work.
    # 在执行生成工作前确认 cargo-deny 可用。
    $cargoDenyVersion = (Invoke-CheckedCommand -FilePath "cargo" -Arguments @("deny", "--version")) -join "`n"

    $licenseCheckStatus = "skipped"
    if (-not $ShouldSkipCheck) {
        Invoke-CheckedCommand -FilePath "cargo" -Arguments @("deny", "check", "licenses") | Out-Null
        $licenseCheckStatus = "passed"
    }

    New-Item -ItemType Directory -Path $TargetOutputDir -Force | Out-Null

    $rawJsonPath = Join-Path $TargetOutputDir "cargo-deny-list.json"
    $certificateJsonPath = Join-Path $TargetOutputDir "dependency-license-certificate.json"
    $certificateMarkdownPath = Join-Path $TargetOutputDir "dependency-license-certificate.md"

    # Keep the raw cargo-deny JSON beside normalized certificates for auditability.
    # 将 cargo-deny 原始 JSON 与规范化证明并排保存，便于审计。
    $rawJson = (Invoke-CheckedCommand -FilePath "cargo" -Arguments @("deny", "list", "--format", "json")) -join "`n"
    Set-Content -Path $rawJsonPath -Value $rawJson -Encoding UTF8

    $denyList = $rawJson | ConvertFrom-Json
    $licenses = Convert-LicenseRows -Rows @($denyList.licenses)
    $clarifications = Get-LicenseClarifications -ConfigPath "deny.toml"
    $resolvedLicenses = Resolve-ClarifiedUnlicensedEntries -Licenses $licenses -Unlicensed @($denyList.unlicensed) -Clarifications $clarifications
    $licenses = @($resolvedLicenses.licenses)
    $dependencyCount = ($licenses | ForEach-Object { $_.dependency_count } | Measure-Object -Sum).Sum
    if ($null -eq $dependencyCount) {
        $dependencyCount = 0
    }

    $metadata = Invoke-CheckedCommand -FilePath "cargo" -Arguments @("metadata", "--format-version", "1", "--no-deps") | ConvertFrom-Json
    $rootPackage = $metadata.packages | Select-Object -First 1

    $certificate = [PSCustomObject]@{
        schema_version = 1
        generated_at_utc = [DateTimeOffset]::UtcNow.ToString("o")
        project = [PSCustomObject]@{
            name = $rootPackage.name
            version = $rootPackage.version
        }
        cargo_deny_version = $cargoDenyVersion
        license_check_status = $licenseCheckStatus
        license_count = $licenses.Count
        dependency_count = [int]$dependencyCount
        unlicensed_count = @($resolvedLicenses.unlicensed).Count
        clarified_count = @($resolvedLicenses.clarified).Count
        licenses = $licenses
        unlicensed = @($resolvedLicenses.unlicensed)
        clarified = @($resolvedLicenses.clarified)
        raw_unlicensed = @($denyList.unlicensed)
        raw_cargo_deny_list = $rawJsonPath
    }

    $certificate | ConvertTo-Json -Depth 8 | Set-Content -Path $certificateJsonPath -Encoding UTF8
    Write-LicenseCertificateMarkdown -Certificate $certificate -Path $certificateMarkdownPath

    return [PSCustomObject]@{
        raw_json = $rawJsonPath
        certificate_json = $certificateJsonPath
        certificate_markdown = $certificateMarkdownPath
    }
}

$result = New-DependencyLicenseCertificate -TargetOutputDir $OutputDir -ShouldSkipCheck ([bool]$SkipCheck)
Write-Host "Dependency license certificate generated:"
Write-Host "  Raw JSON: $($result.raw_json)"
Write-Host "  Certificate JSON: $($result.certificate_json)"
Write-Host "  Certificate Markdown: $($result.certificate_markdown)"

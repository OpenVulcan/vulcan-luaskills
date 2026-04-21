$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$demoRoot = Split-Path -Parent $scriptRoot
$backendDir = Join-Path $demoRoot "backends"

New-Item -ItemType Directory -Force -Path $backendDir | Out-Null

$candidates = @(
    @{
        Name = "vldb_sqlite.dll"
        Paths = @(
            "D:\projects\VulcanLocalDataGateway\vldb-sqlite\target\debug\vldb_sqlite.dll",
            "D:\projects\VulcanLocalDataGateway\vldb-sqlite\target\release\vldb_sqlite.dll"
        )
    },
    @{
        Name = "vldb_lancedb.dll"
        Paths = @(
            "D:\projects\VulcanLocalDataGateway\vldb-lancedb\target\debug\vldb_lancedb.dll",
            "D:\projects\VulcanLocalDataGateway\vldb-lancedb\target\release\vldb_lancedb.dll"
        )
    }
)

foreach ($candidate in $candidates) {
    $source = $candidate.Paths | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $source) {
        Write-Host "Skip $($candidate.Name): no local build found."
        continue
    }

    $target = Join-Path $backendDir $candidate.Name
    Copy-Item -Force -Path $source -Destination $target
    Write-Host "Copied $($candidate.Name) -> $target"
}

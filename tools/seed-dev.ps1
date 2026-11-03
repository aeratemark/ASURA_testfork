<#
.SYNOPSIS
    Seed the development login identities the patched client presents.

.DESCRIPTION
    Development convenience, not a deployment tool.

    The rows are in tools/seed-dev.sql -- one data file, applied the same way on Windows and Linux.
    This script is the Windows wrapper for the older verify scripts; the cross-platform way
    is `asura-supervisor seed`.

    Idempotent: re-running reopens the same sessions and leaves the characters alone, which is also
    how a developer resets a pair that a test or a logout closed.

    ASCII only, like every script in tools/: PowerShell 5.1 reads a BOM-less file as ANSI, where a
    UTF-8 em dash breaks the parse far from the cause (docs/HANDOFF.md).

.EXAMPLE
    .\tools\seed-dev.ps1
#>

$ErrorActionPreference = "Stop"

function Find-Psql {
    $onPath = Get-Command psql -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }

    $root = "C:\Program Files\PostgreSQL"
    if (Test-Path $root) {
        $candidate = Get-ChildItem $root -Directory |
            Sort-Object Name -Descending |
            ForEach-Object { Join-Path $_.FullName "bin\psql.exe" } |
            Where-Object { Test-Path $_ } |
            Select-Object -First 1
        if ($candidate) { return $candidate }
    }
    throw "psql.exe not found. Install PostgreSQL, or add its bin directory to PATH."
}

function Get-DatabaseUrl {
    $candidates = @(
        (Join-Path (Split-Path -Parent $PSScriptRoot) ".env"),
        (Join-Path $PSScriptRoot ".env")
    )
    foreach ($path in $candidates) {
        if (-not (Test-Path $path)) { continue }
        foreach ($line in Get-Content $path) {
            $trimmed = $line.Trim()
            if ($trimmed -eq "" -or $trimmed.StartsWith("#")) { continue }
            $parts = $trimmed.Split("=", 2)
            if ($parts.Count -eq 2 -and $parts[0].Trim() -eq "ASURA_DATABASE_URL") {
                $value = $parts[1].Trim().Trim("'").Trim('"')
                if ($value) { return $value }
            }
        }
    }
    throw "ASURA_DATABASE_URL not found. Copy .env.example to .env, or run tools/db-setup.ps1."
}

$script:Psql = Find-Psql
$url = Get-DatabaseUrl
Write-Host "psql: $script:Psql"

$sqlPath = Join-Path $PSScriptRoot "seed-dev.sql"
if (-not (Test-Path $sqlPath)) { throw "no $sqlPath" }

# Options before the database URI: psql reads anything after the dbname as an extra argument.
# psql's stderr is not fatal here: PowerShell turns native stderr into error records, and with
# $ErrorActionPreference = "Stop" a NOTICE would abort the seeding. Judge by the exit code instead.
$previous = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    $output = & $script:Psql -v ON_ERROR_STOP=1 -X -q -f $sqlPath $url 2>&1 | ForEach-Object { "$_" }
    $exitCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $previous
}
if ($exitCode -ne 0) {
    throw "psql failed (exit $exitCode) while seeding the dev identities:`n$($output -join "`n")"
}

Write-Host ""
Write-Host "dev identities seeded. The rig's portal DLL presents these pairs:"
Write-Host "  user1@example.com  portal fa520ee2-4419-4eb4-ae49-6e9abe6ef24f  session d8b9bf5d-90b1-4cbd-9b76-88da7be763b6"
Write-Host "  user2@example.com  portal 5fb222bc-9d19-4308-ac3e-3c62685bc6ae  session 224aee30-d8a5-4f18-ad8e-b5fcbf121c4e"
Write-Host "  user3@example.com  portal 9957c79c-b93c-4b60-9d84-9f38a1f62be3  session ccdb4b8d-1c88-48c0-bda0-0a5e8d87f4da"
Write-Host ""
Write-Host "verify with: cargo run -p asura-supervisor -- status (or the old tools/m3-verify.ps1)"

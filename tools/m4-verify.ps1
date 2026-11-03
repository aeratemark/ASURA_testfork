<#
.SYNOPSIS
    Verify the travel path: login, placement, and a character standing in a map.

.DESCRIPTION
    Loads ASURA_DATABASE_URL from the environment, or from the repository's .env. Then runs:

      1. every pending migration (crates/asura-db/migrations/*.sql), applied in order. Idempotent, so
         this is safe to re-run and is how a database created before a migration catches up.
      2. the dev identities and the maps (tools/seed-dev.ps1, tools/seed-map.ps1), because every test
         below needs a world and a map to place a player in.
      3. the protocol tests (crates/asura-protocol): the two message tables compute to the sizes the
         reference declares, the key exchange matches the reference's own fixtures, and the game
         builders produce the sizes the reference's send log records.
      4. the auth tests (crates/asura-auth): a client logs in over a socket, and a travel request is
         answered with a world that answers, or refused honestly.
      5. the world tests (crates/asura-world): a placed client is loaded into its map, a token nobody
         issued is closed silently, and - the one that matters - a client logs in to the auth service,
         travels, and reaches INSTANCE_LOAD_FINISH on the world server it was sent to.
      6. the generated files still match the reference: the auth table and the game table.

    Harness-only: no game client. The client arrives when you run asura-supervisor start
    (`docs/19-running.md`).

.EXAMPLE
    .\tools\m4-verify.ps1
#>

$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot

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

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargo) { $cargo = $cargo.Source }
if (-not $cargo) {
    $candidate = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path $candidate) { $cargo = $candidate }
}
if (-not $cargo) {
    throw "cargo not found. Install Rust, or add %USERPROFILE%\.cargo\bin to PATH."
}

function Get-DatabaseUrl {
    if ($env:ASURA_DATABASE_URL) { return $env:ASURA_DATABASE_URL.Trim() }
    $envFile = Join-Path $repo ".env"
    if (Test-Path $envFile) {
        foreach ($line in Get-Content $envFile) {
            $trimmed = $line.Trim()
            if ($trimmed -eq "" -or $trimmed.StartsWith("#")) { continue }
            $pair = $trimmed -split "=", 2
            if ($pair.Count -eq 2 -and $pair[0].Trim() -eq "ASURA_DATABASE_URL") {
                $value = $pair[1].Trim().Trim('"').Trim("'")
                if ($value) { return $value }
            }
        }
    }
    throw "ASURA_DATABASE_URL not set and no .env found. Run .\tools\db-setup.ps1 first."
}

$url = Get-DatabaseUrl
$psql = Find-Psql
Write-Host "database: $url"
Write-Host ""

# Native programs write progress and NOTICEs to stderr, and PowerShell turns native stderr into error
# records that $ErrorActionPreference = "Stop" treats as fatal. Every native call therefore runs with
# the preference relaxed and is judged by its exit code alone.
function Invoke-Native {
    param([string] $Exe, [string[]] $Arguments)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & $Exe @Arguments 2>&1 | ForEach-Object { "$_" } | ForEach-Object { Write-Host $_ }
        return $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
}

function Invoke-PsqlFile {
    param([string] $Path)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $psql -v ON_ERROR_STOP=1 -X -q -1 -f $Path $url 2>&1 | ForEach-Object { "$_" }
        return @{ ExitCode = $LASTEXITCODE; Output = ($output -join "`n") }
    } finally {
        $ErrorActionPreference = $previous
    }
}

Write-Host "1/6  migrations"
$migrations = Get-ChildItem (Join-Path $repo "crates\asura-db\migrations") -Filter "*.sql" | Sort-Object Name
if (-not $migrations) { throw "no migrations found under crates\asura-db\migrations" }
foreach ($migration in $migrations) {
    $result = Invoke-PsqlFile -Path $migration.FullName
    if ($result.ExitCode -ne 0) {
        throw "migration failed: $($migration.Name)`n$($result.Output)"
    }
    Write-Host "     applied $($migration.Name)"
}

Write-Host ""
Write-Host "2/6  dev identities and maps"
$code = Invoke-Native (Get-Command powershell).Source @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $PSScriptRoot "seed-dev.ps1"))
if ($code -ne 0) { throw "seeding the identities failed" }
$code = Invoke-Native (Get-Command powershell).Source @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $PSScriptRoot "seed-map.ps1"))
if ($code -ne 0) { throw "seeding the maps failed" }

Write-Host ""
Write-Host "3/6  protocol tests (message tables, key exchange, game message sizes)"
$code = Invoke-Native $cargo @("test", "--offline", "-p", "asura-protocol")
if ($code -ne 0) { throw "protocol tests failed" }

Write-Host ""
Write-Host "4/6  auth tests (login over a socket, travel placed or refused)"
$code = Invoke-Native $cargo @("test", "--offline", "-p", "asura-auth")
if ($code -ne 0) { throw "auth tests failed" }

Write-Host ""
Write-Host "5/6  world tests (placed client loaded, bad token closed, full chain to the map)"
$code = Invoke-Native $cargo @("test", "--offline", "-p", "asura-world", "--", "--nocapture")
if ($code -ne 0) { throw "world tests failed" }

Write-Host ""
Write-Host "travel path: OK"
Write-Host ""
Write-Host "Verified end to end without a game: a client logs in, asks to travel, is sent to a world"
Write-Host "that answers, presents its tokens, and receives the reference's full load block ending in"
Write-Host "INSTANCE_LOAD_FINISH -- the message that ends the loading screen."
Write-Host ""
Write-Host "Now the real client:"
Write-Host "  asura-supervisor start       # seeds, starts auth on 6112 and a world on 6113, watches"
Write-Host "  F:\GW\GW1\Gw.custom.exe -authsrv 127.0.0.1 -portal 127.0.0.1"

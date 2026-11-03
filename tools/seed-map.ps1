<#
.SYNOPSIS
    Seed the maps a world server can host, with the data a client needs to load them.

.DESCRIPTION
    Development convenience, not a deployment tool.

    A client entering a map is told three things that must all be right, or it never leaves the loading
    screen: which blob of Gw.dat to stream (`map_file_id`), and where inside it to put the character
    (`spawn_x`, `spawn_y`, `spawn_plane`). None of the three can be guessed -- (0,0) is outside the
    walkable geometry on every pre-Searing map measured so far.

    The values below are not invented. They come from the reference rig, where each map was verified
    against a live client on 2026-10-25: the tester stood a character in the map, read its position back
    (PreSearing/code/OpenTyria/GmMapsConfig.c), and confirmed it landed where it should. The blob id was
    confirmed separately, by matching the client's own terrain fingerprint against the archive entry.

    Idempotent: re-running updates the same rows.

    ASCII only, like every script in tools/: PowerShell 5.1 reads a BOM-less file as ANSI, where a UTF-8
    em dash breaks the parse far from the cause (docs/HANDOFF.md).

.EXAMPLE
    .\tools\seed-map.ps1

.EXAMPLE
    # after a world server refuses to start because a map is missing
    .\tools\seed-map.ps1
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

# map_file_id 0x1B97D (: 113021) is the pre-Searing region blob shared by Lakeside County, Ascalon City
# (pre-Searing) and Ashford Abbey. Sharing a blob is the normal case, not a bug: the map id plus the
# spawn point choose the place inside the region.
# The rows came from the reference's own catalogue (its generator was retired when parity closed, docs/16): every map it has a
# config for, with the aliases and MapType from its teleport table (`GmCommands.c`), the numbers from
# `GmMaps.h`, and the file id and spawn point from `GmMapsConfig.c`. Hand-typing them reached two maps; the
# catalogue has twelve that can be spawned into, and one of them is an explorable this server still refuses
# to host (`main.rs`) -- which the file says at the bottom.
$sqlPath = Join-Path $PSScriptRoot "map-config.sql"
if (-not (Test-Path $sqlPath)) {
    throw "no $sqlPath; the map catalogue SQL is committed as tools/map-config.sql"
}
$sql = Get-Content $sqlPath -Raw

# psql's stderr is not fatal here: PowerShell turns native stderr into error records, and with
# $ErrorActionPreference = "Stop" a NOTICE would abort the seeding. Judge by the exit code instead.
$previous = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    $output = & $script:Psql -v ON_ERROR_STOP=1 -X -q -c $sql $url 2>&1 | ForEach-Object { "$_" }
    $exitCode = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $previous
}
if ($exitCode -ne 0) {
    throw "psql failed (exit $exitCode) while seeding the maps:`n$($output -join "`n")"
}

Write-Host ""
Write-Host "maps seeded from tools/map-config.sql (generated):"
$summary = & $script:Psql -v ON_ERROR_STOP=1 -X -q -t -A -F '|' -c "SELECT map_id, map_type, name FROM map_config ORDER BY map_id" $url 2>&1
foreach ($line in $summary) {
    $parts = "$line" -split '\|'
    if ($parts.Count -eq 3) {
        $kind = if ($parts[1] -eq '0') { 'outpost   ' } else { "type $($parts[1])   " }
        Write-Host ("  {0}  {1} {2}" -f $parts[0], $kind, $parts[2])
    }
}
Write-Host ""
Write-Host "verify with: cargo run -p asura-world -- --maps 146 --world 8001"

<#
.SYNOPSIS
    The test battery: every check this project can make without a game client, in the order to run them.

.DESCRIPTION
    One command, a numbered checklist, and a PASS/FAIL verdict per step. It is meant to be followed:
    if a step fails, the steps after it that depend on it are marked SKIP rather than pretending.

    The steps, and why they are in this order:

      1. the environment (cargo, psql, python, a database URL) -- everything else needs these
      2. the database schema, migrated to this build
      3. the development data the other steps assume (the rig's identities, the maps)
      4. the protocol crate: the two generated message tables, the key exchange, the frame walker
      5. asura-db: the schema invariants, placement, login, migrations
      6. asura-auth: login over a socket, travel placed or refused, the game front door
      7. asura-world: the load, the movement rules, the full chain to INSTANCE_LOAD_FINISH
      8. the real client (by hand -- printed, never automated)

    ASCII only, like every script in tools/ (docs/HANDOFF.md).

.EXAMPLE
    .\tools\test-battery.ps1

.EXAMPLE
    # the same, but stop at the first failure with the full output of the failing step
    .\tools\test-battery.ps1 -StopOnFirstFailure
#>

param(
    [switch] $StopOnFirstFailure
)

$ErrorActionPreference = "Continue"

$repo = Split-Path -Parent $PSScriptRoot
$script:Failures = New-Object System.Collections.ArrayList

function Write-Step {
    param([int] $Number, [string] $Name)
    Write-Host ""
    Write-Host ("{0,2}. {1}" -f $Number, $Name) -ForegroundColor Cyan
}

function Write-Verdict {
    param([bool] $Passed, [string] $Name, [string] $Detail = "")
    if ($Passed) {
        Write-Host ("    PASS  {0}" -f $Name) -ForegroundColor Green
    } else {
        Write-Host ("    FAIL  {0}" -f $Name) -ForegroundColor Red
        if ($Detail) { Write-Host ("          {0}" -f $Detail) -ForegroundColor DarkGray }
        [void] $script:Failures.Add($Name)
        if ($StopOnFirstFailure) {
            Write-Host ""
            Write-Host "stopping at the first failure (-StopOnFirstFailure)" -ForegroundColor Yellow
            exit 1
        }
    }
}

function Write-Skip {
    param([string] $Reason)
    Write-Host ("    SKIP  {0}" -f $Reason) -ForegroundColor Yellow
}

# Native programs write progress to stderr, and PowerShell turns native stderr into error records; each
# call is judged by its exit code alone.
function Invoke-Native {
    param([string] $Exe, [string[]] $Arguments, [string] $WorkDir = $repo)
    $output = & $Exe @Arguments 2>&1 | ForEach-Object { "$_" }
    return @{ ExitCode = $LASTEXITCODE; Output = $output }
}

function Find-Cargo {
    $onPath = Get-Command cargo -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $candidate = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path $candidate) { return $candidate }
    return $null
}

function Find-Psql {
    $onPath = Get-Command psql -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $candidate = Get-ChildItem "C:\Program Files\PostgreSQL" -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending |
        ForEach-Object { Join-Path $_.FullName "bin\psql.exe" } |
        Where-Object { Test-Path $_ } |
        Select-Object -First 1
    return $candidate
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
    return $null
}

Write-Host "ASURA test battery"
Write-Host ("repository: {0}" -f $repo)

# --- 1. environment -------------------------------------------------------------------------------
Write-Step 1 "environment"
$cargo = Find-Cargo
$psql = Find-Psql
$python = (Get-Command python -ErrorAction SilentlyContinue).Source
$url = Get-DatabaseUrl

Write-Verdict ([bool]$cargo) "cargo" $(if ($cargo) { "" } else { "install Rust, or add %USERPROFILE%\.cargo\bin to PATH" })
Write-Verdict ([bool]$psql) "psql" $(if ($psql) { "" } else { "install PostgreSQL, or add its bin directory to PATH" })
Write-Verdict ([bool]$python) "python" $(if ($python) { "" } else { "install Python 3 (the generators need it)" })
Write-Verdict ([bool]$url) "ASURA_DATABASE_URL" $(if ($url) { "" } else { "copy .env.example to .env, or run tools/db-setup.ps1" })

# Every script in tools/ must at least parse. A missing comma in a param block broke `play.ps1` and the
# only symptom was "cannot start the server", a long way from the cause -- so the battery reads each one
# with PowerShell's own parser instead of trusting that an edit left it valid.
$brokenScripts = New-Object System.Collections.ArrayList
foreach ($script in Get-ChildItem (Join-Path $PSScriptRoot "*.ps1")) {
    $parseErrors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($script.FullName, [ref]$null, [ref]$parseErrors)
    if ($parseErrors -and $parseErrors.Count -gt 0) {
        $first = $parseErrors[0]
        [void]$brokenScripts.Add("$($script.Name) line $($first.Extent.StartLineNumber): $($first.Message)")
    }
}
Write-Verdict ($brokenScripts.Count -eq 0) "every tools/*.ps1 parses" ($brokenScripts -join " | ")

if (-not $cargo -or -not $url) {
    Write-Host ""
    Write-Host "the first two are required before anything else can run" -ForegroundColor Yellow
    exit 1
}

# --- 2. the schema --------------------------------------------------------------------------------
Write-Step 2 "database schema (every migration, in order)"
$migrations = Get-ChildItem (Join-Path $repo "crates\asura-db\migrations") -Filter "*.sql" | Sort-Object Name
$schemaOk = $true
foreach ($migration in $migrations) {
    $result = Invoke-Native $psql @("-v", "ON_ERROR_STOP=1", "-X", "-q", "-1", "-f", $migration.FullName, $url)
    if ($result.ExitCode -ne 0) {
        $schemaOk = $false
        Write-Verdict $false ("applying {0}" -f $migration.Name) ($result.Output -join " | ")
        break
    }
}
if ($schemaOk) { Write-Verdict $true ("{0} migration(s) applied, idempotently" -f $migrations.Count) }

# --- 3. development data --------------------------------------------------------------------------
Write-Step 3 "development data (the identities the client presents, the maps a world hosts)"
if (Test-Path (Join-Path $PSScriptRoot "seed-dev.ps1")) {
    $seed = & (Get-Command powershell).Source -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "seed-dev.ps1") 2>&1
    Write-Verdict ($LASTEXITCODE -eq 0) "identities seeded" ($seed -join " | ")
}
if (Test-Path (Join-Path $PSScriptRoot "seed-map.ps1")) {
    $seed = & (Get-Command powershell).Source -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "seed-map.ps1") 2>&1
    Write-Verdict ($LASTEXITCODE -eq 0) "maps seeded" ($seed -join " | ")
}

# --- 4..7 the Rust tests ---------------------------------------------------------------------------
$crates = @(
    @(4, "asura-protocol", "the message tables, the key exchange, the frame walker"),
    @(5, "asura-db", "schema invariants, placement, login, migrations"),
    @(6, "asura-auth", "login over a socket, travel, the game front door"),
    @(7, "asura-world", "the load, the movement rules, the chain to INSTANCE_LOAD_FINISH")
)
foreach ($entry in $crates) {
    Write-Step $entry[0] ("{0}: {1}" -f $entry[1], $entry[2])
    $result = Invoke-Native $cargo @("test", "--offline", "-p", $entry[1])
    $summary = ($result.Output | Select-String -Pattern '^test result:' | ForEach-Object { $_.Line }) -join "; "
    Write-Verdict ($result.ExitCode -eq 0) $entry[1] $summary
    if ($result.ExitCode -ne 0) {
        ($result.Output | Select-String -Pattern 'panicked at|FAILED|^error' -Context 0,3) | ForEach-Object { Write-Host $_ }
    }
}

# --- 8. the real client ---------------------------------------------------------------------------
Write-Step 8 "the real client (by hand)"
Write-Host "    Nothing here can launch it or read its window; these are the steps to follow:"
Write-Host "      1. the client is patched first: client-patcher/README.md (Gw.exe -> Gw.custom.exe)"
Write-Host "      2. asura-supervisor start      # the stack, watched; the log is the verdict"
Write-Host "      3. F:\GW\GW1\Gw.custom.exe -authsrv 127.0.0.1 -portal 127.0.0.1"
Write-Host "    the trace in that console is the verdict:"
Write-Host "      'travel to map 146 placed on world 8001'   the auth service placed the travel"
Write-Host "      'joined map 146'                            the client presented its tokens"
Write-Host "      'world -> 0x018E'                           it left the loading screen"

Write-Host ""
if ($script:Failures.Count -eq 0) {
    Write-Host "the battery passed: everything above says PASS" -ForegroundColor Green
    exit 0
}

Write-Host ("{0} step(s) failed:" -f $script:Failures.Count) -ForegroundColor Red
foreach ($failure in $script:Failures) { Write-Host ("  - {0}" -f $failure) -ForegroundColor Red }
exit 1

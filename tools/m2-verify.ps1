<#
.SYNOPSIS
    Run the M2 acceptance suite: the concurrency test, the reaper test, and the fake-world scenario.

.DESCRIPTION
    Loads ASURA_DATABASE_URL from the environment, or from the repository's .env. Then runs:

      1. the asura-db integration tests (crates/asura-db/tests/placement.rs):
         - capacity N-1 with N concurrent reservations: exactly N-1 win, the rest are refused,
           leased ends at capacity, the ledger matches, and the pool empties on release;
         - a full pool refuses a travel without disturbing the holder's lease;
         - the reaper frees a silent world's leases and leaves a fresh world alone;
         - reconcile rebuilds the counter from the ledger.
      2. the fake-world harness scenario (crates/asura-fakeworld), which runs the same sequence end to
         end and prints every check.

    Harness-only: no game client, no protocol, no simulation. The point of M2 is that placement can
    be trusted before anything client-facing exists (docs/roadmap.md).

.EXAMPLE
    .\tools\m2-verify.ps1
#>

$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargo) { $cargo = $cargo.Source }
if (-not $cargo) {
    $candidate = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path $candidate) { $cargo = $candidate }
}
if (-not $cargo) {
    throw "cargo not found. Install Rust, or add %USERPROFILE%\.cargo\bin to PATH."
}

# Load ASURA_DATABASE_URL from the environment, else from .env (the file db-setup.ps1 writes).
if (-not $env:ASURA_DATABASE_URL) {
    $envFile = Join-Path $repo ".env"
    if (Test-Path $envFile) {
        foreach ($line in Get-Content $envFile) {
            $trimmed = $line.Trim()
            if ($trimmed -eq "" -or $trimmed.StartsWith("#")) { continue }
            $pair = $trimmed -split "=", 2
            if ($pair.Count -eq 2 -and $pair[0].Trim() -eq "ASURA_DATABASE_URL") {
                $env:ASURA_DATABASE_URL = $pair[1].Trim().Trim('"').Trim("'")
            }
        }
    }
}
if (-not $env:ASURA_DATABASE_URL) {
    throw "ASURA_DATABASE_URL not set and no .env found. Run .\tools\db-setup.ps1 first."
}

Write-Host "database: $env:ASURA_DATABASE_URL"
Write-Host ""

# cargo writes progress to stderr, and PowerShell turns native stderr into error records that
# $ErrorActionPreference = "Stop" treats as fatal -- so a clean build (or a first build after a
# change) would abort this script before the tests ran. Run native programs with the preference
# relaxed and judge them by exit code alone.
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

Write-Host "1/2  integration tests (concurrency, refusal, reaper, reconcile)"
$code = Invoke-Native $cargo @("test", "--offline", "-p", "asura-db", "--test", "placement", "--", "--nocapture")
if ($code -ne 0) { throw "placement integration tests failed" }

Write-Host ""
Write-Host "2/2  fake-world scenario"
$code = Invoke-Native $cargo @("run", "--offline", "-q", "-p", "asura-fakeworld", "--", "scenario")
if ($code -ne 0) { throw "fake-world scenario failed" }

Write-Host ""
Write-Host "M2 acceptance: OK"

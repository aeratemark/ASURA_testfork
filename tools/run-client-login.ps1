<#
.SYNOPSIS
    Start everything the patched client needs, launch it, and report what the client actually did.

.DESCRIPTION
    One command, so that a real-client attempt does not depend on reading a game window and relaying
    what it said. It:

      1. seeds the development identities;
      2. starts the web gate the client logs in through (it asks for its user_id/session_id there);
      3. starts this project's auth service with the per-message trace on;
      4. launches the patched client;
      5. watches the trace and prints each event as it happens, with elapsed time;
      6. ends with a verdict: LOGIN ACCEPTED, REFUSED, or NO LOGIN ARRIVED (and why).

    Nothing here is a test that can pass without a human: the client waits at its login screen until
    somebody types the account name and presses Log In. What this script removes is the guessing
    afterwards.

.PARAMETER Seconds
    How long to wait for the login before giving up. Default 240.

.PARAMETER LoginName
    The account to type at the login screen. The seeded ones are user1/user2/user3@example.com.
    The password is not checked by the web gate: anything works.

.PARAMETER ClientPath
    The patched client. Default is the rig's install.

.PARAMETER Keep
    Leave the web gate and the auth service running afterwards, for a second attempt.

.EXAMPLE
    .\tools\run-client-login.ps1

.EXAMPLE
    .\tools\run-client-login.ps1 -LoginName user2@example.com -Seconds 120
#>
param(
    [int] $Seconds = 240,
    [string] $LoginName = "user1@example.com",
    [string] $ClientPath = "F:\GW\GW1\Gw.custom.exe",
    [switch] $Keep
)

$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
$tracePath = Join-Path $repo ".client-login.trace.log"
$outPath = Join-Path $repo ".client-login.out.log"
$webgateLog = Join-Path $repo ".client-login.webgate.log"

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

function Test-Port([int] $Port) {
    return [bool](Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
}

function Find-Psql {
    $onPath = Get-Command psql -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $root = "C:\Program Files\PostgreSQL"
    if (Test-Path $root) {
        $candidate = Get-ChildItem $root -Directory | Sort-Object Name -Descending |
            ForEach-Object { Join-Path $_.FullName "bin\psql.exe" } |
            Where-Object { Test-Path $_ } | Select-Object -First 1
        if ($candidate) { return $candidate }
    }
    return $null
}

function Get-DatabaseUrl {
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

# --- preconditions -----------------------------------------------------------------------------

if (-not (Test-Path $ClientPath)) {
    throw "the patched client is not at $ClientPath (pass -ClientPath)"
}
if (Test-Port 6112) {
    throw "port 6112 is busy: something else (a previous run, or GWPS) is already listening there. Stop it first."
}
if (-not (Get-DatabaseUrl)) {
    throw "no ASURA_DATABASE_URL in .env. Run .\tools\db-setup.ps1 first."
}

Write-Host "1/5  seeding the development identities"
$code = Invoke-Native (Get-Command powershell).Source @(
    "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", (Join-Path $PSScriptRoot "seed-dev.ps1"))
if ($code -ne 0) { throw "seeding failed" }

Write-Host ""
Write-Host "2/5  starting the web gate on 6601 (the client asks for its identity here)"
$webgatePath = Join-Path (Split-Path -Parent $repo) "PreSearing\tools\webgate.py"
$webgate = $null
if (Test-Port 6601) {
    Write-Host "     already listening; leaving it alone"
} elseif (Test-Path $webgatePath) {
    $webgate = Start-Process -FilePath "python" -ArgumentList $webgatePath `
        -WorkingDirectory (Split-Path -Parent $webgatePath) -PassThru -WindowStyle Hidden `
        -RedirectStandardOutput $webgateLog -RedirectStandardError "$webgateLog.err"
    Start-Sleep -Seconds 2
    if (-not (Test-Port 6601)) { throw "the web gate did not start (see $webgateLog)" }
    Write-Host "     started (pid $($webgate.Id)), log: $webgateLog"
} else {
    throw "no web gate found at $webgatePath, and nothing is listening on 6601. The client cannot log in without one."
}

Write-Host ""
Write-Host "3/5  starting the auth service on 6112, with the per-message trace"
Remove-Item $tracePath, $outPath -ErrorAction SilentlyContinue
if (-not (Test-Path (Join-Path $repo "target\debug\asura-auth.exe"))) {
    Write-Host "     building first"
    $code = Invoke-Native (Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe") @("build", "--offline", "-p", "asura-auth")
    if ($code -ne 0) { throw "the auth service did not build" }
}
$env:ASURA_TRACE = "1"
$auth = Start-Process -FilePath (Join-Path $repo "target\debug\asura-auth.exe") `
    -ArgumentList "--bind", "127.0.0.1:6112" -WorkingDirectory $repo -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput $outPath -RedirectStandardError $tracePath
Start-Sleep -Seconds 2
if (-not (Test-Port 6112)) { throw "the auth service did not start (see $tracePath)" }
Write-Host "     started (pid $($auth.Id)), trace: $tracePath"

Write-Host ""
Write-Host "4/5  launching the client"
$client = Start-Process -FilePath $ClientPath -ArgumentList '-authsrv', '127.0.0.1', '-portal', '127.0.0.1' `
    -WorkingDirectory (Split-Path -Parent $ClientPath) -PassThru
Write-Host "     pid $($client.Id)"

Write-Host ""
Write-Host "5/5  waiting for you to log in"
Write-Host ""
Write-Host "     In the Guild Wars window:"
Write-Host "       Login Name: $LoginName"
Write-Host "       Password:   anything (the web gate does not check it)"
Write-Host "       then click Log In."
Write-Host ""
Write-Host "     Watching $tracePath for up to $Seconds seconds. Each event is printed as it happens."
Write-Host ""

$startedAt = Get-Date
$seen = @()
$verdict = $null

while (((Get-Date) - $startedAt).TotalSeconds -lt $Seconds) {
    Start-Sleep -Milliseconds 750
    if (-not (Test-Path $tracePath)) { continue }

    $lines = @(Get-Content $tracePath -ErrorAction SilentlyContinue)
    if ($lines.Count -gt $seen.Count) {
        foreach ($line in $lines[$seen.Count..($lines.Count - 1)]) {
            if ($line -match "^\s+[0-9a-f]{2} ") { continue }
            $elapsed = [int]((Get-Date) - $startedAt).TotalSeconds
            Write-Host ("     [{0,4}s] {1}" -f $elapsed, $line)
        }
        $seen = $lines
    }

    $text = $lines -join "`n"
    if ($text -match "auth: login ok for (\S+) with (\d+) character") {
        $verdict = "ACCEPTED"
        $verdictDetail = "login accepted for $($Matches[1]), $($Matches[2]) character(s) on the list"
        break
    }
    if ($text -match "auth: no session for (\S+) / (\S+)") {
        $verdict = "REFUSED"
        $verdictDetail = "the identity pair did not resolve: $($Matches[1]) / $($Matches[2])"
        break
    }
    if ($client.HasExited) {
        $verdict = "CLIENT EXITED"
        $verdictDetail = "the client exited with code $($client.ExitCode) before a login arrived"
        break
    }
}

if (-not $verdict) {
    $text = @(Get-Content $tracePath -ErrorAction SilentlyContinue) -join "`n"
    if ($text -match "forcibly closed") {
        $verdict = "NO LOGIN ARRIVED"
        $verdictDetail = "the client opened a connection, did the key exchange, then reset it without sending a login"
    } elseif ($text -match "connection was idle") {
        $verdict = "NO LOGIN ARRIVED"
        $verdictDetail = "a connection sat idle until the service's timeout closed it"
    } elseif ($text -match "client seed") {
        $verdict = "NO LOGIN ARRIVED"
        $verdictDetail = "the client connected and exchanged keys, but no login came within $Seconds seconds"
    } else {
        $verdict = "NO CONNECTION"
        $verdictDetail = "the client never connected to the auth service at all"
    }
}

Write-Host ""
Write-Host "======================================================================"
Write-Host " VERDICT: $verdict"
Write-Host " $verdictDetail"
Write-Host "======================================================================"
Write-Host ""
Write-Host " trace:        $tracePath"
Write-Host " web gate log: $webgateLog"
$authLog = "F:\GW\GW1\ArenaNet.log"
if (Test-Path $authLog) {
    Write-Host " client log:   $authLog (only written on a client-side fatal error)"
}

if (-not $Keep) {
    Write-Host ""
    Write-Host " stopping the web gate and the auth service (pass -Keep to leave them running)"
    if ($auth -and -not $auth.HasExited) { Stop-Process -Id $auth.Id -Force -ErrorAction SilentlyContinue }
    if ($webgate -and -not $webgate.HasExited) { Stop-Process -Id $webgate.Id -Force -ErrorAction SilentlyContinue }
}
if (-not $client.HasExited) {
    Write-Host " the client is still running (pid $($client.Id)); close it yourself when you are done"
}

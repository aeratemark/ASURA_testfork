
<#
.SYNOPSIS
    Check a running ASURA server's status, default local ports and web gate.

.DESCRIPTION
    Run in a second PowerShell window while the supervisor stays running.
    Supports execution from either the repository root or the tools directory.
    Uses the default single-machine configuration in config/services.toml.
    Opens TCP connections and creates one temporary web-gate session through HTTP.
    Does not build, seed, apply migrations, or start or stop services.
    Exit 0 means these checks passed; exit 1 means a check failed.
    A passing result does not prove client login or gameplay.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File .\diagnose-server.ps1
#>

[CmdletBinding()]
param(
    [ValidateRange(1, 30)]
    [int]$TimeoutSeconds = 5
)

$ErrorActionPreference = 'Stop'
$failures = 0

# Resolve repository root.
# Works when this script is in the repository root or its tools directory.
$scriptDirectory = $PSScriptRoot

if ((Split-Path -Leaf $scriptDirectory) -eq 'tools') {
    $repo = Split-Path -Parent $scriptDirectory
}
else {
    $repo = $scriptDirectory
}

$repo = (Resolve-Path -LiteralPath $repo).Path

# Resolve supervisor executable relative to the repository.
$supervisor = Join-Path $repo 'target\debug\asura-supervisor.exe'

Write-Output 'ASURA running-server diagnostics (default local ports)'
Write-Output "Repository: $repo"
Write-Output "Supervisor: $supervisor"

Push-Location $repo

try {
    Write-Output ''
    Write-Output '== Supervisor status: check for ready services and fresh registry rows'

    if (Test-Path -LiteralPath $supervisor -PathType Leaf) {

        # Windows PowerShell can treat native stderr as error records.
        # Capture output and judge the command by its exit code.
        $previousPreference = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'

        try {
            & $supervisor status 2>&1 |
                ForEach-Object {
                    Write-Output "$_"
                }

            $statusExit = $LASTEXITCODE
        }
        finally {
            $ErrorActionPreference = $previousPreference
        }

        if ($statusExit -ne 0) {
            Write-Output "FAIL: supervisor status exited with $statusExit"
            $failures++
        }
    }
    else {
        Write-Output 'FAIL: target\debug\asura-supervisor.exe is missing.'
        Write-Output 'Build the workspace first if the executable has not been generated.'
        $failures++
    }

    Write-Output ''
    Write-Output '== TCP connections to 127.0.0.1'

    $ports = @(
        @{ Name = 'PostgreSQL'; Port = 5432 }
        @{ Name = 'Auth';       Port = 6112 }
        @{ Name = 'World';      Port = 6113 }
        @{ Name = 'Web gate';   Port = 6601 }
        @{ Name = 'Supervisor'; Port = 7711 }
    )

    foreach ($entry in $ports) {
        $client = New-Object System.Net.Sockets.TcpClient
        $pending = $null

        try {
            $pending = $client.BeginConnect(
                '127.0.0.1',
                $entry.Port,
                $null,
                $null
            )

            if (-not $pending.AsyncWaitHandle.WaitOne($TimeoutSeconds * 1000)) {
                throw 'Connection timed out'
            }

            $client.EndConnect($pending)

            Write-Output "PASS: $($entry.Name) port $($entry.Port) accepts TCP connections"
        }
        catch {
            Write-Output "FAIL: $($entry.Name) port $($entry.Port): $($_.Exception.Message)"
            $failures++
        }
        finally {
            $client.Close()

            if ($null -ne $pending) {
                $pending.AsyncWaitHandle.Close()
            }
        }
    }

    Write-Output ''
    Write-Output '== Web-gate HTTP responses'

    foreach ($path in @(
        '/health',
        '/Spawned/WebGate/session/create.xml'
    )) {
        try {
            $response = Invoke-WebRequest `
                -Uri "http://127.0.0.1:6601$path" `
                -UseBasicParsing `
                -TimeoutSec $TimeoutSeconds

            if ($response.StatusCode -ne 200) {
                throw "HTTP $($response.StatusCode)"
            }

            if ($path -eq '/health') {
                if ($response.Content.Trim() -ne 'ok') {
                    throw 'Expected health body: ok'
                }
            }
            else {
                $document = [xml]$response.Content
                $sessionId = [guid]::Empty

                if (-not [guid]::TryParse(
                    [string]$document.Reply.Session,
                    [ref]$sessionId
                )) {
                    throw 'Expected Reply/Session containing a UUID'
                }
            }

            Write-Output "PASS: $path -> HTTP 200, expected response body"
        }
        catch {
            Write-Output "FAIL: $path -> $($_.Exception.Message)"
            $failures++
        }
    }

    Write-Output ''
    Write-Output "Diagnostics finished: $failures failed check(s)."
    Write-Output 'TCP success proves reachability, not database credentials or the game handshake.'
    Write-Output 'Read the supervisor status output for service states and registry freshness.'
    Write-Output 'Next: test client login and entering a map.'
}
finally {
    Pop-Location
}

if ($failures -gt 0) {
    exit 1
}

exit 0

# The portal probe: what the client says when a character walks into a travel portal.
#
# Why this exists: a portal is a prop the client draws from the instance's own map data, so this server
# neither places it nor sees the player touch it. What the server does see is whether the client
# announced the crossing, which it does with `GAME_CMSG_TRAVEL_INIT` (`0x80C1`, ten bytes, `{portal, 0}`)
# - and, if it announces nothing, exactly what it did send and where the character actually walked.
#
# Usage, from the repository root:
#
#   .\tools\portal-probe.ps1                     # the last session in logs\world-8001.log
#   .\tools\portal-probe.ps1 -Lines 20000        # a longer tail
#   .\tools\portal-probe.ps1 -File logs\world-8001.log
#   .\tools\portal-probe.ps1 -Watch              # tail the log while you walk in, Ctrl+C to report
#
# The world must be started with `ASURA_TRACE=1` for the per-message lines (`world <- 0x80C1`,
# `world <- 0x803D`) to be in the log; the event lines (`travelling`, `asked for instance`, `touched a
# travel object`, `joined map`) are always written.
#
# This script only reads. It changes nothing on the wire and nothing in the database.

[CmdletBinding()]
param(
    [string]$File = "logs\world-8001.log",
    [int]$Lines = 0,
    [switch]$Watch
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $File)) {
    Write-Output "portal-probe: no log at $File - start the world first (asura-supervisor start)."
    exit 2
}

function Read-Log {
    param([string]$Path, [int]$Tail)
    if ($Tail -gt 0) {
        return @(Get-Content -Path $Path -Tail $Tail)
    }
    return @(Get-Content -Path $Path)
}

if ($Watch) {
    Write-Output "portal-probe: watching $File"
    Write-Output "portal-probe: log in, stand in the outpost, walk into the portal. Ctrl+C to report."
    Write-Output ""
    Get-Content -Path $File -Wait -Tail 0 | ForEach-Object {
        if ($_ -match 'world <- 0x80C1|touched a travel object|asked for instance|travelling:|unlocked instance|joined map|socket:') {
            Write-Output $_
        }
    }
    return
}

$log = Read-Log -Path $File -Tail $Lines

# The last run starts at the last "== world-8001: started ==" marker.
$start = 0
for ($i = 0; $i -lt $log.Count; $i++) {
    if ($log[$i] -match '^== .*: started ==') { $start = $i }
}
$session = $log[$start..($log.Count - 1)]

Write-Output "portal-probe: $($session.Count) line(s) of the current run in $File"
Write-Output ""

# ---- where the character entered ------------------------------------------------------------
$joined = $session | Select-String -Pattern 'joined map (\d+) .* standing at \((-?[\d.]+), (-?[\d.]+)\) plane (\d+)' | Select-Object -Last 1
if ($joined) {
    $map = $joined.Matches[0].Groups[1].Value
    $startX = [double]$joined.Matches[0].Groups[2].Value
    $startY = [double]$joined.Matches[0].Groups[3].Value
    $startPlane = $joined.Matches[0].Groups[4].Value
    Write-Output "entered instance $map at ($startX, $startY) plane $startPlane"
} else {
    Write-Output "no 'joined map' line in this run: nobody entered an instance yet"
}

# ---- what the client announced --------------------------------------------------------------
$announcements = @($session | Select-String -Pattern 'world <- 0x80C1 \((\d+) bytes\): (.+)$')
Write-Output ""
if ($announcements.Count -eq 0) {
    Write-Output "TRAVEL_INIT (0x80C1): NONE. The client did not announce a crossing, so the portal it"
    Write-Output "  draws was never armed. Everything below is what it did send instead."
} else {
    Write-Output "TRAVEL_INIT (0x80C1): $($announcements.Count) announcement(s)"
    foreach ($line in $announcements) {
        $hex = $line.Matches[0].Groups[2].Value.Trim()
        $bytes = $hex.Split(' ')
        # header(2) then the portal reference and the zero a live client sends with it
        $first = [BitConverter]::ToUInt32([byte[]]@(
            [Convert]::ToByte($bytes[2], 16), [Convert]::ToByte($bytes[3], 16),
            [Convert]::ToByte($bytes[4], 16), [Convert]::ToByte($bytes[5], 16)), 0)
        $second = [BitConverter]::ToUInt32([byte[]]@(
            [Convert]::ToByte($bytes[6], 16), [Convert]::ToByte($bytes[7], 16),
            [Convert]::ToByte($bytes[8], 16), [Convert]::ToByte($bytes[9], 16)), 0)
        Write-Output "  portal $first, second $second   ($hex)"
    }
}

# ---- what the client sent, by header ---------------------------------------------------------
Write-Output ""
Write-Output "what the client sent this run, by header:"
$sent = @{}
foreach ($line in ($session | Select-String -Pattern 'world <- (0x[0-9A-F]{4}) \((\d+) bytes\)')) {
    $header = $line.Matches[0].Groups[1].Value
    $sent[$header] = 1 + ($sent[$header] -as [int])
}
if ($sent.Count -eq 0) {
    Write-Output "  nothing - the log holds no per-message lines (start the world with ASURA_TRACE=1)"
} else {
    foreach ($key in ($sent.Keys | Sort-Object)) {
        Write-Output ("  {0} x{1}" -f $key, $sent[$key])
    }
}

# ---- where the character actually walked ----------------------------------------------------
$positions = @()
foreach ($line in ($session | Select-String -Pattern 'world <- 0x803D \(26 bytes\): (.+)$')) {
    $bytes = $line.Matches[0].Groups[1].Value.Trim().Split(' ')
    $x = [BitConverter]::ToSingle([byte[]]@(
        [Convert]::ToByte($bytes[2], 16), [Convert]::ToByte($bytes[3], 16),
        [Convert]::ToByte($bytes[4], 16), [Convert]::ToByte($bytes[5], 16)), 0)
    $y = [BitConverter]::ToSingle([byte[]]@(
        [Convert]::ToByte($bytes[6], 16), [Convert]::ToByte($bytes[7], 16),
        [Convert]::ToByte($bytes[8], 16), [Convert]::ToByte($bytes[9], 16)), 0)
    $positions += , @([double]$x, [double]$y)
}
Write-Output ""
if ($positions.Count -eq 0) {
    Write-Output "no position reports (0x803D) in this run"
} else {
    $last = $positions[$positions.Count - 1]
    Write-Output ("{0} position report(s); the last is ({1:F1}, {2:F1})" -f $positions.Count, $last[0], $last[1])
    if ($joined) {
        $far = $positions | ForEach-Object {
            [pscustomobject]@{ X = $_[0]; Y = $_[1]; From = [math]::Sqrt([math]::Pow($_[0] - $startX, 2) + [math]::Pow($_[1] - $startY, 2)) }
        } | Sort-Object From -Descending | Select-Object -First 1
        Write-Output ("furthest point from the entrance: ({0:F1}, {1:F1}), {2:F0} units away" -f $far.X, $far.Y, $far.From)
        Write-Output ""
        Write-Output "next step: compare that walk with the instance's own travel portals. A portal is a prop, so"
        Write-Output "it is in the map data the client draws from; if the walk never reaches one, no message can"
        Write-Output "follow, whatever the server does."
    }
}

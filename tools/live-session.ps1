<#
.SYNOPSIS
    Record the stream and dump the client structures for as long as the operator wants, with no
    input required. Ctrl+C ends it and keeps everything.

.DESCRIPTION
    Starts tools/sniff-capture.py in the background (both directions, raw: names are kept, so every
    file stays OUTSIDE the repository), then dumps the client's contexts on a cadence with
    tools/context-dump.py. Nothing is asked of the operator: no marker, no keypress, no window to
    watch. A load or an action is bracketed by the dumps either side of it and by the capture's own
    timestamps.

    Elevation is required because the library opens the client with write rights even for reads. The
    script relaunches itself elevated once, so approve that single UAC prompt.

.PARAMETER Preset
    init     - the contexts an instance entry changes, dumped every ten seconds.
    identify - every context the dump knows, every twenty seconds.

.PARAMETER Minutes
    0 (the default) runs until Ctrl+C. Any other value stops by itself at that mark.

.PARAMETER DumpEverySeconds
    Cadence of the structure dumps. Zero takes the preset's default.

.PARAMETER Only
    Comma-separated context names for the dump. Empty takes the preset's default set.

.PARAMETER OutDir
    Where the capture, the dumps and the log go. Outside the repository by default.

.EXAMPLE
    .\tools\live-session.ps1 -Preset init
    .\tools\live-session.ps1 -Preset init -DumpEverySeconds 6
    .\tools\live-session.ps1 -Minutes 20 -OutDir D:\captures\run2
#>
param(
    [ValidateSet('init', 'identify')]
    [string]$Preset = 'init',
    [int]$Minutes = 0,
    [int]$DumpEverySeconds = 0,
    [string]$Only = '',
    [string]$OutDir = 'C:\Users\Apo\captures\session',
    [switch]$Elevated
)

$ErrorActionPreference = 'Continue'

$presets = @{
    init = @(
        'enter an outpost from character select',
        'travel from that outpost to the explorable it exits to',
        'come back to the outpost',
        'change district inside that outpost',
        'open the world map, then close it',
        'walk to a portal and cross it'
    )
    identify = @(
        'toggle hard mode in an explorable (party panel)',
        'add one henchman in an outpost, then dismiss it',
        'accept one quest at a quest NPC',
        'die, then get resurrected',
        'kill one foe',
        'drop one item, then pick it up',
        'summon something (pet, minion or spirit)',
        'dye one equipped item',
        'click the eye on one inventory slot (display mode)',
        'enter the guild hall, or an outpost your guild holds',
        'open the world map, click a destination, close it',
        'trigger a cinematic',
        'press one adrenal skill on a warrior with adrenaline',
        'travel to an outpost with a second player in it',
        'talk to a merchant, buy nothing, then buy one thing'
    )
}
$actions = $presets[$Preset]

if ($DumpEverySeconds -le 0) { $DumpEverySeconds = if ($Preset -eq 'init') { 10 } else { 20 } }
if (-not $Only -and $Preset -eq 'init') {
    $Only = 'worldcontext,mapcontext,charcontext,partycontext,itemcontext,agentarray,accagentcontext,instanceinfo,missionmap,worldmap,serverregion'
}

function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    return (New-Object Security.Principal.WindowsPrincipal($id)).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
}

if (-not (Test-Admin)) {
    $shell = (Get-Process -Id $PID).Path
    $quoted = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath,
                '-Preset', $Preset, '-Minutes', $Minutes, '-DumpEverySeconds', $DumpEverySeconds,
                '-Only', $Only, '-OutDir', $OutDir, '-Elevated')
    Write-Host 'asking for an elevated shell: approve the UAC prompt' -ForegroundColor Yellow
    Start-Process -FilePath $shell -Verb RunAs -ArgumentList $quoted
    exit 0
}

$repo = Split-Path (Split-Path $PSCommandPath -Parent) -Parent
$capture = Join-Path $OutDir 'capture-raw.jsonl'
$dumpDir = Join-Path $OutDir 'dumps'
$log = Join-Path $OutDir 'session.log'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
New-Item -ItemType Directory -Force -Path $dumpDir | Out-Null
if (Test-Path $capture) { Remove-Item $capture -Force }

function Say([string]$text, [string]$colour = 'Gray') {
    Write-Host $text -ForegroundColor $colour
    Add-Content -Path $log -Value ('[{0}] {1}' -f (Get-Date -Format 'HH:mm:ss'), $text)
}

Set-Content -Path $log -Value ('session {0}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'))
Say ('repository: ' + $repo)
Say ('capture:    ' + $capture)
Say ('dumps:      ' + $dumpDir)
$howLong = if ($Minutes -gt 0) { "$Minutes minute(s)" } else { 'until Ctrl+C' }
Say ('preset: {0}, a dump every {1}s, {2}' -f $Preset, $DumpEverySeconds, $howLong)

if (-not (Test-Path (Join-Path $repo 'tools\sniff-capture.py'))) { Say 'sniff-capture.py not found' 'Red'; exit 3 }
if (-not (Test-Path (Join-Path $repo 'tools\context-dump.py')))  { Say 'context-dump.py not found' 'Red'; exit 3 }
$python = (Get-Command python -ErrorAction SilentlyContinue)
if (-not $python) { Say 'python is not on PATH in this shell' 'Red'; exit 3 }
Say ('python: ' + $python.Source)
$client = Get-Process | Where-Object { $_.ProcessName -match '^Gw' } | Select-Object -First 1
if (-not $client) { Say 'no Guild Wars client is running' 'Red'; exit 3 }
Say ('client: pid {0} {1}' -f $client.Id, $client.ProcessName)

Write-Host ''
Write-Host 'nothing is asked of you from here on. Actions worth doing, in no particular order:' -ForegroundColor Cyan
foreach ($action in $actions) { Write-Host ('  - ' + $action) }
Write-Host ''
Write-Host 'a load is bracketed by the dumps either side of it. Ctrl+C ends the run and keeps every file.'
Write-Host ''

# The sniffer cannot be told "until Ctrl+C", so it gets a ceiling far beyond any session and is
# stopped by this script's finally block when the operator interrupts.
$ceiling = if ($Minutes -gt 0) { $Minutes * 60 } else { 14400 }
Set-Location $repo
$sniffer = Start-Process -FilePath $python.Source -PassThru -WindowStyle Hidden -ArgumentList @(
    'tools\sniff-capture.py', '--dir', 'both', '--seconds', $ceiling, '--raw', '--out', $capture)
Say ('sniffer started: pid {0}, ceiling {1}s, both directions, raw' -f $sniffer.Id, $ceiling)

$started = Get-Date
$deadline = if ($Minutes -gt 0) { $started.AddSeconds($Minutes * 60) } else { $started.AddDays(1) }
$dumps = 0
$onlyArgs = @()
if ($Only) { $onlyArgs = @('--only', $Only) }

try {
    while ((Get-Date) -lt $deadline) {
        $stamp = Get-Date -Format 'HHmmss'
        $out = Join-Path $dumpDir ('t' + $stamp + '.jsonl')
        $read = & python tools\context-dump.py --label ('t' + $stamp) --out $out @onlyArgs 2>&1
        $summary = ($read | Where-Object { $_ -match 'context\(s\) read' } | Select-Object -Last 1)
        if (-not $summary) { $read | Select-Object -Last 3 | ForEach-Object { Say ('    ' + $_) 'DarkGray' } }
        $dumps++
        $elapsed = [int]((Get-Date) - $started).TotalSeconds
        Say ('dump {0,3} at {1,6}s (t{2}) {3}' -f $dumps, $elapsed, $stamp, ([string]$summary).Trim())

        $next = (Get-Date).AddSeconds($DumpEverySeconds)
        while ((Get-Date) -lt $next -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
    }
    Say 'the requested duration is up'
}
finally {
    if ($sniffer -and -not $sniffer.HasExited) {
        Say 'stopping the sniffer'
        try { Stop-Process -Id $sniffer.Id -Force } catch { Say ('could not stop the sniffer: ' + $_) 'Red' }
    }
    $ended = Get-Date
    $manifest = [ordered]@{
        started = $started.ToString('o')
        ended = $ended.ToString('o')
        seconds = [int]($ended - $started).TotalSeconds
        preset = $Preset
        actions_worth_doing = $actions
        capture = $capture
        dumps = $dumpDir
        dump_count = $dumps
        note = 'a load or an action is bracketed by the dumps either side of it and by the capture timestamps'
    }
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -Path (Join-Path $OutDir 'manifest.json') -Encoding utf8
    Say ('done: {0} dump(s) in {1}s' -f $dumps, $manifest.seconds) 'Cyan'
    Write-Host ''
    Write-Host 'hand back these two paths:' -ForegroundColor Cyan
    Write-Host ('  ' + $capture)
    Write-Host ('  ' + $dumpDir)
}

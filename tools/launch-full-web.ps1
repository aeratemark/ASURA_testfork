<#
.SYNOPSIS
    Launch an additional dsh web server that runs with full file access (danger-full-access).

.DESCRIPTION
    Developer convenience, not a deployment tool.

    A `dsh web` server started from inside a sandboxed command inherits the sandbox's restricted
    token, so even with DSH_PERMISSION_MODE=danger-full-access its own startup writes (the profile
    cordis.yml under DSH_HOME) would be denied. This launcher is therefore meant to be started
    ELEVATED (Start-Process -Verb RunAs), which goes through UAC and gives the server a normal,
    unrestricted user token.

    Everything it does is appended to <repo>\.dsh-web-full.log, including failures, so an elevated
    run that dies before printing anything still leaves a trace.

.PARAMETER Port
    The loopback port to bind. Default 3081.

.PARAMETER NodePath
    Explicit node.exe path. Default: `node` on PATH, else %ProgramFiles%\nodejs\node.exe.

.PARAMETER BinPath
    Explicit path to @deepseek-ai/dsh/lib/bin.js. Default: %APPDATA%\npm\node_modules\@deepseek-ai\dsh\lib\bin.js.

.PARAMETER DshHome
    Explicit DSH_HOME. Default: $env:DSH_HOME, else %USERPROFILE%\.dsh.

.EXAMPLE
    Start-Process powershell -Verb RunAs -ArgumentList '-NoLogo','-NoProfile','-ExecutionPolicy','Bypass','-File',"$PWD\tools\launch-full-web.ps1"
#>

param(
    [int] $Port = 3081,
    [string] $NodePath = '',
    [string] $BinPath = '',
    [string] $DshHome = ''
)

$repo = Split-Path -Parent $PSScriptRoot
$log  = Join-Path $repo ".dsh-web-full.log"

function Write-Log([string] $Line) {
    "$(Get-Date -Format o)  $Line" | Out-File -FilePath $log -Append -Encoding ascii
}

Write-Log "launcher: repo=$repo user=$env:USERNAME profile=$env:USERPROFILE"

try {
    if (-not $DshHome) {
        $DshHome = if ($env:DSH_HOME) { $env:DSH_HOME } else { Join-Path $env:USERPROFILE '.dsh' }
    }
    if (-not (Test-Path $DshHome)) { throw "DSH_HOME not found: $DshHome" }

    if (-not $NodePath) {
        $NodePath = (Get-Command node -ErrorAction SilentlyContinue).Source
        if (-not $NodePath) { $NodePath = Join-Path $env:ProgramFiles 'nodejs\node.exe' }
    }
    if (-not (Test-Path $NodePath)) { throw "node not found: $NodePath" }

    if (-not $BinPath) {
        $BinPath = Join-Path $env:APPDATA 'npm\node_modules\@deepseek-ai\dsh\lib\bin.js'
    }
    if (-not (Test-Path $BinPath)) { throw "dsh entry point not found: $BinPath (pass -BinPath)" }

    $env:DSH_HOME = $DshHome
    $env:DSH_PERMISSION_MODE = 'danger-full-access'

    Write-Log "launching: node=$NodePath bin=$BinPath dshHome=$DshHome port=$Port"
    & $NodePath $BinPath web --port $Port --no-open *>> $log
    Write-Log "dsh web exited with code $LASTEXITCODE"
}
catch {
    Write-Log "FAILED: $($_.Exception.Message)"
    throw
}

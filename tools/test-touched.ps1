# test-touched.ps1 - run only the tests the changed files justify.
#
# The rule is AGENTS.md #3, "Testing: one targeted run, and the client is the judge". It
# carries a tool because it was already written once and was ignored: a session
# ran the whole world socket suite four times for a change that needed two tests.
#
# So the run starts here. This script reads what changed, prints the one targeted command per
# touched crate, and runs it with -Run. It does NOT run the standing suite: that needs
# -WholeSuite -Justification "<milestone | shared machinery: what>", and the justification is the
# sentence the commit carries.
#
# Usage:
#   .\tools\test-touched.ps1                     print the scope, run nothing
#   .\tools\test-touched.ps1 -Run                run the targeted commands
#   .\tools\test-touched.ps1 -WholeSuite -Justification "milestone: M5 landing" -Run
#
# If a command needs the database, set ASURA_DATABASE_URL first (it is in .env).

param(
    [switch]$Run,
    [switch]$WholeSuite,
    [string]$Justification = ''
)

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

try {
    if ($WholeSuite) {
        if ([string]::IsNullOrWhiteSpace($Justification)) {
            Write-Host "test-touched: the whole suite needs a reason."
            Write-Host "  The rule (AGENTS.md #3): the standing suites run when the"
            Write-Host "  change reaches shared machinery (framing, the generated tables, schema,"
            Write-Host "  start-up) or before a commit that claims a milestone - not because a suite"
            Write-Host "  exists. Pass -Justification and put the same sentence in the commit."
            exit 1
        }
        $commands = @('cargo test --workspace')
        Write-Host "test-touched: whole suite, justified: $Justification"
    }
    else {
        $status = & git status --porcelain
        $touched = @{}
        $codeFiles = 0
        foreach ($line in $status) {
            $path = $line.Substring(3).Trim()
            if ($path -match ' -> ') { $path = $path.Split(' -> ')[1] }
            $path = $path -replace '\\', '/'
            if ($path -match '^crates/([^/]+)/') {
                $crate = $Matches[1]
                if (-not $touched.ContainsKey($crate)) { $touched[$crate] = @{} }
                if ($path -match '^crates/[^/]+/tests/([^/]+)\.rs$') {
                    $touched[$crate]["test:$($Matches[1])"] = $true
                }
                elseif ($path -match '^crates/[^/]+/src/') {
                    $touched[$crate]['lib'] = $true
                }
                else {
                    $touched[$crate]['other'] = $true
                }
                $codeFiles++
            }
        }

        if ($touched.Count -eq 0) {
            if ($codeFiles -eq 0) {
                Write-Host "test-touched: no crate changed (documents only) - nothing to run. The client is the judge."
                exit 0
            }
            exit 0
        }

        $commands = @()
        foreach ($crate in ($touched.Keys | Sort-Object)) {
            $marks = $touched[$crate]
            $needsCompile = $marks.ContainsKey('lib') -or $marks.ContainsKey('other')
            if ($needsCompile) {
                $commands += "cargo check -p $crate --all-targets"
            }
            if ($marks.ContainsKey('lib')) {
                $commands += "cargo test -p $crate --lib"
            }
            foreach ($key in ($marks.Keys | Where-Object { $_ -like 'test:*' } | Sort-Object)) {
                $target = $key.Substring('test:'.Length)
                $commands += "cargo test -p $crate --test $target"
            }
        }

        if ($commands.Count -eq 0) {
            Write-Host "test-touched: nothing in the scope runs tests."
            exit 0
        }
        Write-Host "test-touched: the changed files justify exactly these:"
        $commands | ForEach-Object { Write-Host "  $_" }
        Write-Host "  (one line per result; the live client is the acceptance test)"
        if (-not $env:ASURA_DATABASE_URL -and ($commands -match 'asura-world|asura-db|asura-auth')) {
            Write-Host "  note: ASURA_DATABASE_URL is not set; database tests would skip. It is in .env."
        }
    }

    if (-not $Run) {
        Write-Host "test-touched: printed only. Add -Run to execute."
        exit 0
    }

    foreach ($command in $commands) {
        Write-Host "== $command"
        $parts = $command -split ' '
        & $parts[0] @($parts[1..($parts.Length - 1)])
        if ($LASTEXITCODE -ne 0) {
            Write-Host "test-touched: FAILED at: $command"
            exit $LASTEXITCODE
        }
    }
    Write-Host "test-touched: done, $($commands.Count) command(s), all exit 0"
    exit 0
}
finally {
    Pop-Location
}

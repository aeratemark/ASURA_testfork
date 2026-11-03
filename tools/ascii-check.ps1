# ascii-check.ps1 - fail if any source or document holds a non-ASCII byte.
#
# The rule is AGENTS.md #5, "Every file is ASCII". Windows PowerShell 5.1
# reads a BOM-less file as ANSI, so a UTF-8 em dash comes back as mojibake and the file is written
# back corrupted. ASCII is the one encoding every tool here agrees on, so it is checked.
#
# recordings/ is excluded on purpose: those files are bytes captured from a live client, evidence
# rather than writing. logs/, target/ and .git/ are not source. Binary extensions are skipped.
#
# Usage:  .\tools\ascii-check.ps1        exit 0 = clean, exit 1 = offender listed

$root = Split-Path -Parent $PSScriptRoot
$skip = '\\(\.git|target|target-verify|logs|recordings)(\\|$)'
$binary = '^\.(bin|exe|dll|pdb|png|jpg|jpeg|webp|gif|zip|rar|dat|pyc)$'

$bad = @()
Get-ChildItem -Path $root -Recurse -File | Where-Object {
    $_.FullName -notmatch $skip -and $_.Extension -notmatch $binary
} | ForEach-Object {
    $bytes = [System.IO.File]::ReadAllBytes($_.FullName)
    if ($bytes -notcontains 0) {
        $text = [System.IO.File]::ReadAllText($_.FullName)
        if ($text -match '[^\x00-\x7F]') {
            $bad += $_.FullName.Substring($root.Length + 1)
        }
    }
}

if ($bad.Count -eq 0) {
    Write-Host "ascii-check: every source and document is ASCII"
    exit 0
}

Write-Host "ascii-check: non-ASCII characters found in:"
$bad | ForEach-Object { Write-Host "  $_" }
exit 1

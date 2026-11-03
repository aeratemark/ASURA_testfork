<#
.SYNOPSIS
    Create the ASURA role and database (if missing), apply the schema, and verify it.

.DESCRIPTION
    Development convenience, not a deployment tool. It assumes the PostgreSQL superuser password is
    already in $env:PGPASSWORD for the first two steps, then switches to the application role for the
    schema.

    Everything it does is idempotent: existing roles and databases are left alone, and the schema
    itself only creates what is missing. Safe to re-run after a failed attempt.

.EXAMPLE
    $env:PGPASSWORD = "<postgres superuser password>"
    .\tools\db-setup.ps1

.EXAMPLE
    # different names, and a role password of your choosing
    $env:PGPASSWORD = "<postgres superuser password>"
    .\tools\db-setup.ps1 -AsuraPassword 'change-me' -Database asura_dev
#>
param(
    [string] $SuperUser = "postgres",
    [string] $Role = "asura",
    [string] $Database = "asura",
    [string] $AsuraPassword = "asura-dev",
    [string] $HostName = "127.0.0.1",
    [int]    $Port = 5432
)

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

function Invoke-Psql {
    param(
        [string] $User,
        [string] $DatabaseName,
        [string] $Arguments
    )
    $target = @("-U", $User, "-h", $HostName, "-p", "$Port", "-v", "ON_ERROR_STOP=1", "-X", "-A", "-t")
    if ($DatabaseName) { $target += @("-d", $DatabaseName) }
    $target += $Arguments.Split(" ")

    # psql writes NOTICEs ("relation already exists, skipping") to stderr, and PowerShell turns native
    # stderr into error records that $ErrorActionPreference = "Stop" treats as fatal. Re-running this
    # script against an existing schema always produces those notices, so they must not abort it:
    # run psql with the preference relaxed and judge it by its exit code alone.
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $script:Psql @target 2>&1 | ForEach-Object { "$_" }
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previous
    }
    if ($exitCode -ne 0) {
        throw "psql failed (exit $exitCode): $Arguments`n$($output -join "`n")"
    }
    return $output
}

$script:Psql = Find-Psql
Write-Host "psql: $script:Psql"

if (-not $env:PGPASSWORD) {
    throw "Set `$env:PGPASSWORD to the PostgreSQL superuser password first (it is used only for the role/database steps)."
}

# --- role -------------------------------------------------------------------------------------
$superPassword = $env:PGPASSWORD
$exists = Invoke-Psql -User $SuperUser -DatabaseName "" -Arguments "-c `"SELECT 1 FROM pg_roles WHERE rolname = '$Role'`""
if ($exists -and $exists.Trim() -eq "1") {
    Write-Host "role '$Role' already exists; leaving it alone"
} else {
    Invoke-Psql -User $SuperUser -DatabaseName "" -Arguments "-c `"CREATE ROLE $Role LOGIN PASSWORD '$AsuraPassword'`"" | Out-Null
    Write-Host "created role '$Role'"
}

# --- database ---------------------------------------------------------------------------------
$exists = Invoke-Psql -User $SuperUser -DatabaseName "" -Arguments "-c `"SELECT 1 FROM pg_database WHERE datname = '$Database'`""
if ($exists -and $exists.Trim() -eq "1") {
    Write-Host "database '$Database' already exists; leaving it alone"
} else {
    Invoke-Psql -User $SuperUser -DatabaseName "" -Arguments "-c `"CREATE DATABASE $Database OWNER $Role`"" | Out-Null
    Write-Host "created database '$Database' owned by '$Role'"
}

# --- schema -----------------------------------------------------------------------------------
$migrations = Get-ChildItem (Join-Path (Split-Path -Parent $PSScriptRoot) "crates\asura-db\migrations") -Filter "*.sql" |
    Sort-Object Name
if (-not $migrations) { throw "no migrations found under crates\asura-db\migrations" }

$env:PGPASSWORD = $AsuraPassword
foreach ($migration in $migrations) {
    # -1: the whole file in one transaction, so a failure part-way leaves neither the change nor the
    # schema_version row that records it (crates/asura-db/src/migrations.rs).
    Invoke-Psql -User $Role -DatabaseName $Database -Arguments "-1 -f `"$($migration.FullName)`"" | Out-Null
    Write-Host "applied $($migration.Name)"
}

# --- verification -----------------------------------------------------------------------------
Write-Host ""
Write-Host "tables:"
$tables = Invoke-Psql -User $Role -DatabaseName $Database -Arguments "-c `"SELECT tablename FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename`""
$tables.Trim() -split "`r?`n" | Where-Object { $_ -ne "" } | ForEach-Object { "  $_" }

Write-Host ""
$pool = Invoke-Psql -User $Role -DatabaseName $Database -Arguments "-c `"SELECT count(*) FROM world_pool`""
Write-Host "world_pool rows: $($pool.Trim())"

Write-Host ""
Write-Host "ready. connection string:"
Write-Host "  host=$HostName port=$Port user=$Role dbname=$Database password=$AsuraPassword"
Write-Host ""
Write-Host "next: the M2 acceptance test - .\tools\m2-verify.ps1"
Write-Host "      (capacity N-1, N concurrent reservations, exactly N-1 succeed, the CHECK never fires)"

# Prüft install.ps1 gegen ein lokales Release (Windows): installieren, erneut installieren,
# aktualisieren, `railit self`, entfernen, verfälschte Prüfsumme.
#
#   pwsh scripts/test-installer.ps1 [-Archive dist\railit-x86_64-pc-windows-msvc.zip] [-Version X.Y.Z]
#
# Ohne Archiv baut das Skript eines mit Platzhalterdateien.
param(
    [string] $Archive,
    [string] $Version = '0.0.0-test'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$install = Join-Path $root 'scripts\install.ps1'
$target = 'x86_64-pc-windows-msvc'
$name = "railit-$target.zip"
$work = Join-Path ([IO.Path]::GetTempPath()) ("railit-test-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
$bins = @('railit', 'railitd', 'railit-field', 'railit-node')

function Check([bool] $Ok, [string] $What) {
    if (-not $Ok) { throw "FEHLGESCHLAGEN: $What" }
}

$real = [bool] $Archive
if (-not $real) {
    $stage = Join-Path $work "stage\railit-$target"
    New-Item -ItemType Directory -Path (Join-Path $stage 'examples') | Out-Null
    foreach ($b in $bins) { Set-Content -Path (Join-Path $stage "$b.exe") -Value 'Platzhalter' }
    Copy-Item (Join-Path $root 'examples\*.toml') (Join-Path $stage 'examples')
    Copy-Item $install, (Join-Path $root 'LICENSE') $stage
    Set-Content -Path (Join-Path $stage 'VERSION') -Value $Version
    $Archive = Join-Path $work $name
    Compress-Archive -Path $stage -DestinationPath $Archive
}

foreach ($dir in @("download\v$Version", 'latest\download')) {
    $d = Join-Path $work "rel\$dir"
    New-Item -ItemType Directory -Force -Path $d | Out-Null
    Copy-Item $Archive (Join-Path $d $name)
    $hash = (Get-FileHash -Algorithm SHA256 (Join-Path $d $name)).Hash.ToLower()
    Set-Content -Path (Join-Path $d 'SHA256SUMS') -Value "$hash  $name"
}
$port = 20000 + (Get-Random -Maximum 20000)
$python = if ($env:OS -eq 'Windows_NT') { 'python' } else { 'python3' }
$server = Start-Process $python -ArgumentList '-m', 'http.server', $port, '--bind', '127.0.0.1' `
    -WorkingDirectory (Join-Path $work 'rel') -PassThru
try {
    $base = "http://127.0.0.1:$port"
    for ($i = 0; $i -lt 50; $i++) {
        try { Invoke-WebRequest -UseBasicParsing "$base/latest/download/SHA256SUMS" | Out-Null; break }
        catch { Start-Sleep -Milliseconds 200 }
    }
    $env:RAILIT_DOWNLOAD_URL = $base
    $prefix = Join-Path $work 'prefix'
    $bindir = Join-Path $prefix 'bin'
    $data = Join-Path $prefix 'share\rail-it'

    & $install install -Prefix $prefix
    foreach ($b in $bins) { Check (Test-Path (Join-Path $bindir "$b.exe")) "$b.exe installiert" }
    Check (Test-Path (Join-Path $data 'manifest')) 'Manifest'
    Check (Test-Path (Join-Path $data 'examples\beispielstadt.toml')) 'Beispielnetze'
    Check ((Get-Content (Join-Path $data 'manifest')) -contains "version=$Version") 'Version im Manifest'
    $windows = $env:OS -eq 'Windows_NT'
    if ($windows) {
        Check (([Environment]::GetEnvironmentVariable('Path', 'User') -split ';') -contains $bindir) 'PATH'
    }

    & $install install -Prefix $prefix
    & $install update -Prefix $prefix -Version $Version -Force

    if ($real) {
        $railit = Join-Path $bindir 'railit.exe'
        & $railit --version
        $info = & $railit self info
        Check (($info -join "`n") -match 'Installiert über den Installer') 'railit self info'
        & $railit net validate (Join-Path $data 'examples\beispielstadt.toml')
        & $railit self update
        Check ($LASTEXITCODE -eq 0) 'railit self update'
        & $railit self uninstall
        for ($i = 0; $i -lt 100 -and (Test-Path (Join-Path $data 'manifest')); $i++) { Start-Sleep -Milliseconds 200 }
    } else {
        & $install uninstall -Prefix $prefix
    }
    Check (-not (Test-Path (Join-Path $data 'manifest'))) 'Manifest entfernt'
    foreach ($b in $bins) { Check (-not (Test-Path (Join-Path $bindir "$b.exe"))) "$b.exe entfernt" }
    if ($windows) {
        Check (-not (([Environment]::GetEnvironmentVariable('Path', 'User') -split ';') -contains $bindir)) 'PATH bereinigt'
    }

    $sums = Join-Path $work 'rel\latest\download\SHA256SUMS'
    $text = Get-Content $sums -Raw
    Set-Content -Path $sums -Value ('0' + $text.Substring(1))
    $rejected = $false
    try { & $install install -Prefix $prefix } catch { $rejected = $true }
    Check $rejected 'verfälschte Prüfsumme abgewiesen'
    Write-Host "Installer: alle Prüfungen bestanden ($target, $Version)"
}
finally {
    Stop-Process -Id $server.Id -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

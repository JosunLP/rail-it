# rail-it – Installation, Aktualisierung und Deinstallation (Windows).
#
#   irm https://josunlp.github.io/rail-it/install.ps1 | iex
#   & ([scriptblock]::Create((irm https://josunlp.github.io/rail-it/install.ps1))) update
#   & ([scriptblock]::Create((irm https://josunlp.github.io/rail-it/install.ps1))) uninstall
#
# Nach der Installation genügen `railit self update` und `railit self uninstall`.
#
# Programme nach <Prefix>\bin (Vorgabe %LOCALAPPDATA%\Programs\rail-it), Daten nach
# <Prefix>\share\rail-it. Das bin-Verzeichnis wird dem PATH des Benutzers hinzugefügt. Jede
# heruntergeladene Datei wird gegen SHA256SUMS des Releases geprüft.
#
# Umgebung: RAILIT_VERSION, RAILIT_PREFIX, RAILIT_REPO, RAILIT_DOWNLOAD_URL

[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('install', 'update', 'uninstall', 'help')]
    [string] $Command = 'install',
    [string] $Version = $env:RAILIT_VERSION,
    [string] $Prefix = $env:RAILIT_PREFIX,
    [switch] $Force
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Repo = if ($env:RAILIT_REPO) { $env:RAILIT_REPO } else { 'JosunLP/rail-it' }
$Base = if ($env:RAILIT_DOWNLOAD_URL) { $env:RAILIT_DOWNLOAD_URL } else { "https://github.com/$Repo/releases" }
$Binaries = @('railit', 'railitd', 'railit-field', 'railit-node')

function Say([string] $Text) { Write-Host "rail-it: $Text" }
function Fail([string] $Text) { throw "rail-it: Fehler: $Text" }

function Get-Target {
    $arch = $env:PROCESSOR_ARCHITECTURE
    if ($arch -ne 'AMD64') { Fail "Prozessor $arch wird nicht unterstützt (x86_64)" }
    'x86_64-pc-windows-msvc'
}

function Get-Url([string] $Ver, [string] $File) {
    if ($Ver) { "$Base/download/v$Ver/$File" } else { "$Base/latest/download/$File" }
}

function Read-Manifest([string] $Path) {
    $m = @{ files = @() }
    if (-not (Test-Path $Path)) { return $null }
    foreach ($line in Get-Content $Path) {
        $k, $v = $line -split '=', 2
        if ($k -eq 'file') { $m.files += $v } elseif ($k) { $m[$k] = $v }
    }
    $m
}

function Add-UserPath([string] $Dir) {
    $path = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @($path -split ';' | Where-Object { $_ })
    if ($parts -notcontains $Dir) {
        [Environment]::SetEnvironmentVariable('Path', (($parts + $Dir) -join ';'), 'User')
        Say "$Dir zum PATH des Benutzers hinzugefügt (neue Konsole öffnen)"
    }
}

function Remove-UserPath([string] $Dir) {
    $path = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @($path -split ';' | Where-Object { $_ -and $_ -ne $Dir })
    [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
}

function Install-Release([string] $Prefix, [string] $Ver, [bool] $Force, [bool] $Path) {
    $target = Get-Target
    $archive = "railit-$target.zip"
    $bindir = Join-Path $Prefix 'bin'
    $data = Join-Path $Prefix 'share\rail-it'
    $manifest = Join-Path $data 'manifest'
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("railit-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $label = if ($Ver) { $Ver } else { 'neueste Version' }
        Say "lade $label für $target"
        Invoke-WebRequest -UseBasicParsing -Uri (Get-Url $Ver $archive) -OutFile (Join-Path $tmp $archive)
        Invoke-WebRequest -UseBasicParsing -Uri (Get-Url $Ver 'SHA256SUMS') -OutFile (Join-Path $tmp 'SHA256SUMS')
        $expected = (Get-Content (Join-Path $tmp 'SHA256SUMS') |
            Where-Object { ($_ -split '\s+')[1] -in @($archive, "*$archive") } |
            ForEach-Object { ($_ -split '\s+')[0] }) | Select-Object -First 1
        if (-not $expected) { Fail "keine Prüfsumme für $archive" }
        $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $archive)).Hash.ToLower()
        if ($actual -ne $expected.ToLower()) { Fail "Prüfsumme von $archive stimmt nicht" }

        Expand-Archive -Path (Join-Path $tmp $archive) -DestinationPath $tmp -Force
        $stage = Join-Path $tmp "railit-$target"
        $new = (Get-Content (Join-Path $stage 'VERSION') -Raw).Trim()
        $old = $null
        $m = Read-Manifest $manifest
        if ($m) { $old = $m.version }
        if ($old -eq $new -and -not $Force) {
            Say "rail-it $new ist schon installiert ($bindir)"
            return
        }

        New-Item -ItemType Directory -Force -Path $bindir, $data | Out-Null
        $files = @()
        foreach ($bin in $Binaries) {
            $dest = Join-Path $bindir "$bin.exe"
            # Ein laufendes Programm lässt sich nicht überschreiben, wohl aber umbenennen.
            if (Test-Path $dest) {
                $stale = "$dest.old"
                Remove-Item -Force $stale -ErrorAction SilentlyContinue
                Rename-Item -Path $dest -NewName (Split-Path $stale -Leaf)
            }
            Copy-Item (Join-Path $stage "$bin.exe") $dest
            Remove-Item -Force "$dest.old" -ErrorAction SilentlyContinue
            $files += $dest
        }
        $examples = Join-Path $data 'examples'
        if (Test-Path $examples) { Remove-Item -Recurse -Force $examples }
        Copy-Item -Recurse (Join-Path $stage 'examples') $examples
        foreach ($doc in @('LICENSE', 'README.md', 'CHANGELOG.md', 'VERSION', 'install.ps1', 'install.sh')) {
            $src = Join-Path $stage $doc
            if (Test-Path $src) { Copy-Item $src (Join-Path $data $doc) -Force }
        }
        $lines = @("version=$new", "target=$target", "prefix=$Prefix", "bindir=$bindir", "data=$data", "repo=$Repo")
        $lines += $files | ForEach-Object { "file=$_" }
        Set-Content -Path $manifest -Value $lines -Encoding UTF8

        if ($old) { Say "rail-it $old -> $new aktualisiert ($bindir)" }
        else {
            Say "rail-it $new installiert: $($Binaries -join ', ') nach $bindir"
            Say "Beispielnetze: $examples"
        }
        if ($Path) { Add-UserPath $bindir }
    }
    finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}

function Uninstall-Release([string] $Prefix) {
    $data = Join-Path $Prefix 'share\rail-it'
    $m = Read-Manifest (Join-Path $data 'manifest')
    if (-not $m) { Fail "keine Installation unter $Prefix gefunden" }
    foreach ($f in $m.files) {
        Remove-Item -Force $f -ErrorAction SilentlyContinue
        Remove-Item -Force "$f.old" -ErrorAction SilentlyContinue
    }
    Remove-Item -Recurse -Force $data
    Remove-UserPath $m.bindir
    if ((Test-Path $m.bindir) -and -not (Get-ChildItem $m.bindir)) { Remove-Item $m.bindir }
    Say "rail-it $($m.version) entfernt"
}

# `railit self uninstall` startet den Installer und beendet sich: erst danach lässt sich
# railit.exe löschen.
if ($env:RAILIT_WAIT_PID) {
    Wait-Process -Id ([int] $env:RAILIT_WAIT_PID) -Timeout 30 -ErrorAction SilentlyContinue
}

if ($Command -eq 'help') { Get-Help $PSCommandPath -ErrorAction SilentlyContinue; return }
if (-not $Prefix) { $Prefix = Join-Path $env:LOCALAPPDATA 'Programs\rail-it' }
if ($Version) { $Version = $Version.TrimStart('v') }

switch ($Command) {
    'install' { Install-Release $Prefix $Version $Force.IsPresent $true }
    'update' {
        if (-not (Test-Path (Join-Path $Prefix 'share\rail-it\manifest'))) { Fail "keine Installation unter $Prefix – erst installieren" }
        Install-Release $Prefix $Version $Force.IsPresent $false
    }
    'uninstall' { Uninstall-Release $Prefix }
}

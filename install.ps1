# Installs or updates taiko.rs for the current user (no admin rights needed).
#
#   irm https://raw.githubusercontent.com/Lynerrr/taiko-rs/worktree-taiko-client/install.ps1 | iex
#
# $env:TAIKO_RS_CHANNEL = 'dev' first installs the latest test build instead.
$ErrorActionPreference = 'Stop'
# The progress bar makes Invoke-WebRequest many times slower.
$ProgressPreference = 'SilentlyContinue'

$tag = if ($env:TAIKO_RS_CHANNEL -eq 'dev') { 'dev-build' } else { 'latest' }
$url = "https://github.com/Lynerrr/taiko-rs/releases/download/$tag/taiko-rs-windows-x64.zip"
$dir = Join-Path $env:LOCALAPPDATA 'Programs\taiko-rs'
$exe = Join-Path $dir 'taiko-rs.exe'
$zip = Join-Path $env:TEMP 'taiko-rs-windows-x64.zip'

# GitHub sometimes answers 504 on the release download link for a while: retry, then take the
# API's link to the same file, which goes through a different front end.
function Get-Archive($from, $headers) {
    for ($try = 1; ; $try++) {
        try {
            Invoke-WebRequest $from -OutFile $zip -UseBasicParsing -Headers $headers
            return
        } catch {
            if ($try -ge 3) { throw }
            Start-Sleep -Seconds 3
        }
    }
}

Write-Host 'Downloading taiko.rs...'
try {
    Get-Archive $url @{}
} catch {
    Write-Host "Trying GitHub's other download link..."
    $release = Invoke-RestMethod "https://api.github.com/repos/Lynerrr/taiko-rs/releases/tags/$tag" -UseBasicParsing
    $asset = $release.assets | Where-Object name -eq 'taiko-rs-windows-x64.zip' | Select-Object -First 1
    if (-not $asset) { throw }
    Get-Archive $asset.url @{ Accept = 'application/octet-stream' }
}

# Updating: the old exe can't be replaced while it's running.
Get-Process taiko-rs -ErrorAction SilentlyContinue | Stop-Process -Force
New-Item -ItemType Directory -Force $dir | Out-Null
Expand-Archive $zip -DestinationPath $dir -Force
Remove-Item $zip

$shell = New-Object -ComObject WScript.Shell
foreach ($folder in [Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('Desktop')) {
    $link = $shell.CreateShortcut((Join-Path $folder 'taiko-rs.lnk'))
    $link.TargetPath = $exe
    $link.WorkingDirectory = $dir
    $ico = Join-Path $dir 'taiko-rs.ico'
    if (Test-Path $ico) { $link.IconLocation = $ico }
    $link.Save()
}

Write-Host "Installed to $dir (Start menu and desktop shortcuts added)."
Write-Host 'Maps: taiko.rs reads your osu! Songs folder (%LOCALAPPDATA%\osu!\Songs).'
Write-Host 'No osu!? Put .osz files in %APPDATA%\taiko-rs\Songs'
Start-Process $exe

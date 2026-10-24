# Installs or updates taiko-rs for the current user (no admin rights needed).
#
#   irm https://raw.githubusercontent.com/Lynerrr/taiko-rs/worktree-taiko-client/install.ps1 | iex
#
# $env:TAIKO_RS_CHANNEL = 'dev' first installs the latest test build instead.
$ErrorActionPreference = 'Stop'
# The progress bar makes Invoke-WebRequest many times slower.
$ProgressPreference = 'SilentlyContinue'

$tag = if ($env:TAIKO_RS_CHANNEL -eq 'dev') { 'dev' } else { 'latest' }
$url = "https://github.com/Lynerrr/taiko-rs/releases/download/$tag/taiko-rs-windows-x64.zip"
$dir = Join-Path $env:LOCALAPPDATA 'Programs\taiko-rs'
$exe = Join-Path $dir 'taiko-rs.exe'
$zip = Join-Path $env:TEMP 'taiko-rs-windows-x64.zip'

Write-Host 'Downloading taiko-rs...'
Invoke-WebRequest $url -OutFile $zip -UseBasicParsing

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
Write-Host 'Maps: taiko-rs reads your osu! Songs folder (%LOCALAPPDATA%\osu!\Songs).'
Write-Host 'No osu!? Put .osz files in %APPDATA%\taiko-rs\Songs'
Start-Process $exe

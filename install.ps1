#Requires -Version 5.1
# Clawdometer one-command installer.
# Usage:
#   powershell -ExecutionPolicy Bypass -Command "irm https://raw.githubusercontent.com/cthpAiden/Clawdometer/main/install.ps1 | iex"

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$repo    = 'cthpAiden/Clawdometer'
$appName = 'Clawdometer'

Write-Host ""
Write-Host "Installing $appName" -ForegroundColor Cyan

# 1. Look up the latest release and its NSIS setup asset.
Write-Host "  Fetching latest release..."
$release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'clawdometer-installer' }
$asset   = $release.assets | Where-Object { $_.name -like '*-setup.exe' } | Select-Object -First 1
if (-not $asset) {
    throw "No -setup.exe asset found in the latest release ($($release.tag_name))."
}
Write-Host "  Found $($asset.name) [$($release.tag_name)]"

# 2. Download the installer to a temp file.
$installer = Join-Path $env:TEMP $asset.name
Write-Host "  Downloading..."
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $installer -UseBasicParsing

# 3. Stop any running instance (NSIS cannot overwrite a running exe),
#    clear Mark-of-the-Web so SmartScreen does not block the silent install,
#    then run the installer silently (no installer window).
$running = Get-Process -Name $appName -ErrorAction SilentlyContinue
if ($running) {
    Write-Host "  Stopping running $appName..."
    $running | Stop-Process -Force
    Start-Sleep -Seconds 1
}
Unblock-File -Path $installer -ErrorAction SilentlyContinue
Write-Host "  Installing (silent)..."
$proc = Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru
if ($proc.ExitCode -ne 0) {
    throw "Installer exited with code $($proc.ExitCode)."
}

# 4. Confirm the app landed, then clean up the download.
$exe = Join-Path $env:LOCALAPPDATA "$appName\$appName.exe"
if (-not (Test-Path $exe)) {
    $exe = Get-ChildItem -Path $env:LOCALAPPDATA -Recurse -Filter "$appName.exe" -ErrorAction SilentlyContinue |
           Select-Object -First 1 -ExpandProperty FullName
}
if (-not $exe -or -not (Test-Path $exe)) {
    throw "Install finished but $appName.exe was not found under $env:LOCALAPPDATA."
}
Remove-Item $installer -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "  $appName installed successfully." -ForegroundColor Green
Write-Host "  Location: $exe" -ForegroundColor DarkGray

# 5. Auto-launch.
Write-Host "  Launching..."
Start-Process $exe
Write-Host ""

# Install Taildrop for the current user on Windows.
#
# Run from the unpacked zip, next to taildrop-gui.exe:
#   powershell -ExecutionPolicy Bypass -File install-windows.ps1
#
# Everything goes under %LOCALAPPDATA%\Taildrop, because that needs no
# administrator rights and survives updates. Nothing is written to Program Files:
# a per-user app that asks for elevation just to copy two files teaches people to
# click through UAC prompts.

$ErrorActionPreference = 'Stop'

$source = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $source 'taildrop-gui.exe'
if (-not (Test-Path $exe)) {
    throw "taildrop-gui.exe was not found next to this script ($source). Unpack the whole zip first."
}

$target = Join-Path $env:LOCALAPPDATA 'Taildrop'
New-Item -ItemType Directory -Force -Path $target | Out-Null

# The Qt runtime, the engine and the app itself all have to land together:
# windeployqt puts the DLLs beside the exe, and the app looks for
# taildrop-cli.exe in its own directory before anywhere else.
Copy-Item -Path (Join-Path $source '*') -Destination $target -Recurse -Force
$installedExe = Join-Path $target 'taildrop-gui.exe'

function New-Shortcut($path, $arguments, $description) {
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($path)
    $shortcut.TargetPath = $installedExe
    $shortcut.Arguments = $arguments
    $shortcut.WorkingDirectory = $target
    $shortcut.IconLocation = "$installedExe,0"
    $shortcut.Description = $description
    $shortcut.Save()
}

# Start menu: the normal way to find it.
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
New-Shortcut (Join-Path $startMenu 'Taildrop.lnk') '' 'Send and receive files over your tailnet'

# Startup: listen from login, with no window in the way. Without this the app
# only receives files while someone happens to have opened it, which is exactly
# the thing a receiver must not depend on.
$startup = [Environment]::GetFolderPath('Startup')
New-Shortcut (Join-Path $startup 'Taildrop.lnk') '--background' 'Taildrop receiver'

Write-Host "Installed to $target"
Write-Host "Starting Taildrop now; look for its icon in the notification area."

# Start it listening right away, so the install is not followed by "nothing
# happened". A second launch only brings the running window forward, so this is
# safe to run again.
Start-Process -FilePath $installedExe -ArgumentList '--background' -WorkingDirectory $target

Write-Host ""
Write-Host "To remove it:"
Write-Host "  Remove-Item '$target' -Recurse -Force"
Write-Host "  Remove-Item (Join-Path '$startMenu' 'Taildrop.lnk') -Force"
Write-Host "  Remove-Item (Join-Path '$startup' 'Taildrop.lnk') -Force"

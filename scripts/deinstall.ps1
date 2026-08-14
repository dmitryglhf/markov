##############################################################################
# Markov uninstaller for Windows.
#
#   irm https://github.com/dmitryglhf/markov/releases/download/stable/deinstall.ps1 | iex
#
# Options need the script block form, because a piped script takes no arguments:
#
#   & ([scriptblock]::Create((irm <same url>))) -KeepData
#   & ([scriptblock]::Create((irm <same url>))) -Yes
#
# Removes the desktop app, the CLI and, unlike `install.ps1 -Uninstall`, the
# settings, credentials and sessions as well. Everything it touches lives in
# the user's own profile, so no administrator rights are needed.
#
# Environment:
#   MARKOV_APP_DIR      where the app was put (default: %LOCALAPPDATA%\Programs\Markov)
#   MARKOV_INSTALL_DIR  where the CLI was put (default: %USERPROFILE%\markov)
##############################################################################

param(
    [switch]$KeepData,
    [switch]$Yes,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'

if ($Help) {
    Write-Host @'
Markov uninstaller for Windows.

  deinstall.ps1             remove the app, the CLI, settings and sessions
  deinstall.ps1 -KeepData   remove the app and the CLI, keep settings
  deinstall.ps1 -Yes        do not ask for confirmation

Environment: MARKOV_APP_DIR, MARKOV_INSTALL_DIR
'@
    return
}

$appDir     = if ($env:MARKOV_APP_DIR) { $env:MARKOV_APP_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\Markov' }
$installDir = if ($env:MARKOV_INSTALL_DIR) { $env:MARKOV_INSTALL_DIR } else { Join-Path $env:USERPROFILE 'markov' }

$cliPath  = Join-Path $installDir 'markov.exe'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Markov.lnk'

$settings = Join-Path $env:APPDATA 'postgrespro\markov'
# Electron keeps its own store under the product name.
$desktopData = Join-Path $env:APPDATA 'Markov'

function Remove-FromUserPath([string]$dir) {
    $current = [Environment]::GetEnvironmentVariable('PATH', 'User')
    if (-not $current) { return }
    $kept = @($current -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -ne $dir.TrimEnd('\') })
    $updated = $kept -join ';'
    if ($updated -ne $current) {
        [Environment]::SetEnvironmentVariable('PATH', $updated, 'User')
        Write-Host "removed $dir from PATH"
    }
}

##############################################################################
# What is actually here
##############################################################################

$programTargets = @($shortcut, $appDir, $cliPath) | Where-Object { Test-Path $_ }

$dataTargets = @()
if (-not $KeepData) {
    $dataTargets = @($settings, $desktopData) | Where-Object { Test-Path $_ }
}

if ($programTargets.Count -eq 0 -and $dataTargets.Count -eq 0) {
    Write-Host 'Nothing to remove — markov is not installed here.'
    return
}

if ($programTargets.Count -gt 0 -and (Get-Process -Name 'Markov' -ErrorAction SilentlyContinue)) {
    throw 'Markov is running — quit it first, then run this again'
}

##############################################################################
# Confirm
##############################################################################

Write-Host 'About to remove:'
foreach ($target in $programTargets + $dataTargets) {
    Write-Host "  $target"
}

if ($dataTargets.Count -gt 0) {
    Write-Host ''
    Write-Host 'This includes your settings, provider keys and every saved session.'
    Write-Host 'Pass -KeepData to remove the program only.'
}

if (-not $Yes) {
    Write-Host ''
    # Read-Host still reaches the console under `irm | iex`, but not when the
    # script is driven by something that has no console at all.
    if (-not [Environment]::UserInteractive) {
        throw 'no terminal to confirm on — rerun with -Yes'
    }
    $reply = Read-Host 'Continue? [y/N]'
    if ($reply -notmatch '^(y|Y|yes|YES)$') {
        Write-Host 'Cancelled.'
        return
    }
}

##############################################################################
# Remove
##############################################################################

foreach ($target in $programTargets + $dataTargets) {
    Remove-Item $target -Recurse -Force
    Write-Host "removed $target"
}

# Ours to make, ours to take back, but only while nobody else lives there.
if ((Test-Path $installDir) -and -not (Get-ChildItem $installDir -Force)) {
    Remove-Item $installDir -Force
}
Remove-FromUserPath $installDir

Write-Host ''
Write-Host 'Done.'
if ($KeepData) {
    Write-Host 'Settings and sessions were left alone:'
    Write-Host "  $settings"
}
# Shared with other agents, so never ours alone to delete.
$agents = Join-Path $env:USERPROFILE '.agents'
if (Test-Path $agents) {
    Write-Host "Left in place: $agents (plugins and skills, shared with other agents)"
}

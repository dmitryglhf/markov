##############################################################################
# Markov installer for Windows.
#
#   irm https://github.com/dmitryglhf/markov/releases/download/stable/install.ps1 | iex
#
# Options need the script block form, because a piped script takes no arguments:
#
#   & ([scriptblock]::Create((irm <same url>))) -CliOnly
#   & ([scriptblock]::Create((irm <same url>))) -Uninstall
#
# The desktop app goes to %LOCALAPPDATA%\Programs\Markov with a Start Menu
# shortcut, and the CLI to %USERPROFILE%\markov, which is put on the user PATH.
# Everything is checked against SHA256SUMS before it is unpacked.
#
# Installs into the user's own profile, so no administrator rights are needed.
#
# Environment:
#   MARKOV_VERSION          exact version to install, e.g. 1.45.0
#   MARKOV_CHANNEL          stable|canary (default: stable)
#   MARKOV_REPO             GitHub repository releases are read from
#   MARKOV_BASE_URL         full release asset base, overrides the two above
#   MARKOV_APP_DIR          where the app goes (default: %LOCALAPPDATA%\Programs\Markov)
#   MARKOV_INSTALL_DIR      where the CLI goes (default: %USERPROFILE%\markov)
##############################################################################

param(
    [switch]$CliOnly,
    [switch]$Uninstall,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'

# Windows PowerShell 5.1 may still offer TLS 1.0 first, which GitHub refuses.
[Net.ServicePointManager]::SecurityProtocol =
    [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

# The progress bar costs more than the download itself on 5.1.
$ProgressPreference = 'SilentlyContinue'

if ($Help) {
    Write-Host @'
Markov installer for Windows.

  install.ps1              desktop app + CLI
  install.ps1 -CliOnly     CLI only, no desktop app
  install.ps1 -Uninstall   remove the app and the CLI, keep settings

Environment: MARKOV_VERSION, MARKOV_CHANNEL, MARKOV_REPO, MARKOV_BASE_URL,
MARKOV_APP_DIR, MARKOV_INSTALL_DIR, MARKOV_WINDOWS_VARIANT
'@
    return
}

##############################################################################
# Platform
##############################################################################

$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -ne 'AMD64') {
    throw "no Windows build for $arch — x64 only"
}

$channel = if ($env:MARKOV_CHANNEL) { $env:MARKOV_CHANNEL } else { 'stable' }
if ($channel -ne 'stable' -and $channel -ne 'canary') {
    throw "unsupported MARKOV_CHANNEL '$channel' (stable|canary)"
}

$target = 'x86_64-pc-windows-msvc'

$repo       = if ($env:MARKOV_REPO) { $env:MARKOV_REPO } else { 'dmitryglhf/markov' }
$appDir     = if ($env:MARKOV_APP_DIR) { $env:MARKOV_APP_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\Markov' }
$installDir = if ($env:MARKOV_INSTALL_DIR) { $env:MARKOV_INSTALL_DIR } else { Join-Path $env:USERPROFILE 'markov' }

$appExe     = Join-Path $appDir 'Markov.exe'
$bundledCli = Join-Path $appDir 'resources\bin\markov.exe'
$cliPath    = Join-Path $installDir 'markov.exe'
$shortcut   = Join-Path ([Environment]::GetFolderPath('Programs')) 'Markov.lnk'

$cliArchive = "markov-cli-$target.zip"
$appArchive = "markov-desktop-$target.zip"

function Get-WithRetry([string]$uri, [string]$outFile) {
    # Invoke-WebRequest learned -MaximumRetryCount only in PowerShell 6, and a stock
    # Windows still runs 5.1.
    for ($attempt = 1; $true; $attempt++) {
        try {
            Invoke-WebRequest -Uri $uri -OutFile $outFile -UseBasicParsing
            return
        } catch {
            # A file that is not there stays not there; only a broken transfer is
            # worth repeating.
            $code = $_.Exception.Response.StatusCode
            if ($attempt -ge 3 -or ($code -and [int]$code -ge 400 -and [int]$code -lt 500)) {
                throw
            }
            Start-Sleep -Seconds 2
        }
    }
}

function Assert-NotRunning {
    # A running app holds Markov.exe open, so an overwrite would fail halfway.
    if (Get-Process -Name 'Markov' -ErrorAction SilentlyContinue) {
        throw 'Markov is running — quit it first, then run this again'
    }
}

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
# Uninstall
##############################################################################

if ($Uninstall) {
    Assert-NotRunning

    if (Test-Path $shortcut) {
        Remove-Item $shortcut -Force
        Write-Host "removed $shortcut"
    }
    if (Test-Path $appDir) {
        Remove-Item $appDir -Recurse -Force
        Write-Host "removed $appDir"
    }
    if (Test-Path $cliPath) {
        Remove-Item $cliPath -Force
        Write-Host "removed $cliPath"
    }
    # Only a directory we created and nobody else filled.
    if ((Test-Path $installDir) -and -not (Get-ChildItem $installDir -Force)) {
        Remove-Item $installDir -Force
    }
    Remove-FromUserPath $installDir

    Write-Host ''
    Write-Host 'Settings and sessions were left alone:'
    Write-Host "  $(Join-Path $env:APPDATA 'postgrespro\markov')"
    return
}

##############################################################################
# Download
##############################################################################

if ($env:MARKOV_BASE_URL) {
    $baseUrl = $env:MARKOV_BASE_URL
    $label   = $channel
} elseif ($env:MARKOV_VERSION) {
    $version = $env:MARKOV_VERSION.TrimStart('v')
    if ($version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(-.*)?$') {
        throw "invalid MARKOV_VERSION '$env:MARKOV_VERSION' (expected X.Y.Z)"
    }
    $baseUrl = "https://github.com/$repo/releases/download/v$version"
    $label   = $version
} else {
    $baseUrl = "https://github.com/$repo/releases/download/$channel"
    $label   = $channel
}

Assert-NotRunning

$workDir = Join-Path $env:TEMP "markov-install-$(Get-Random)"
New-Item -ItemType Directory -Path $workDir -Force | Out-Null

try {
    Write-Host "Installing Markov $label ($target)"

    $sums = Join-Path $workDir 'SHA256SUMS'
    try {
        Get-WithRetry "$baseUrl/SHA256SUMS" $sums
    } catch {
        throw "no release '$label' at $baseUrl"
    }

    $archive = if ($CliOnly) { $cliArchive } else { $appArchive }
    $archivePath = Join-Path $workDir $archive

    Write-Host "Downloading $archive..."
    try {
        Get-WithRetry "$baseUrl/$archive" $archivePath
    } catch {
        throw "could not download $archive"
    }

    # An archive missing from the list is never silently accepted.
    $listed = Get-Content $sums |
        Where-Object { $_ -match "^([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($archive))$" } |
        Select-Object -First 1
    if (-not $listed) { throw "$archive is not listed in SHA256SUMS" }

    $expected = ($listed -split '\s+')[0]
    $actual = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash
    if ($actual -ne $expected.ToUpperInvariant()) {
        throw 'checksum mismatch — the download is not what the release published'
    }

    $unpacked = Join-Path $workDir 'unpacked'
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    # Expand-Archive spends minutes on a quarter-gigabyte zip.
    [System.IO.Compression.ZipFile]::ExtractToDirectory($archivePath, $unpacked)

    ##########################################################################
    # Install
    ##########################################################################

    New-Item -ItemType Directory -Path $installDir -Force | Out-Null

    if ($CliOnly) {
        $source = Get-ChildItem -Path $unpacked -Filter 'markov.exe' -Recurse |
            Select-Object -First 1
        if (-not $source) { throw "$archive does not contain markov.exe" }

        Copy-Item $source.FullName $cliPath -Force
    } else {
        $dist = Join-Path $unpacked 'dist-windows'
        if (-not (Test-Path $dist)) { throw "$archive does not contain dist-windows" }
        if (-not (Test-Path (Join-Path $dist 'Markov.exe'))) {
            throw "$archive does not contain Markov.exe"
        }

        if (Test-Path $appDir) { Remove-Item $appDir -Recurse -Force }
        New-Item -ItemType Directory -Path (Split-Path $appDir -Parent) -Force | Out-Null
        Copy-Item $dist $appDir -Recurse

        # A symlink into the app would need admin rights or Developer Mode.
        Copy-Item $bundledCli $cliPath -Force

        $wsh = New-Object -ComObject WScript.Shell
        $link = $wsh.CreateShortcut($shortcut)
        $link.TargetPath = $appExe
        $link.WorkingDirectory = $appDir
        $link.Description = 'Markov'
        $link.Save()
    }
} finally {
    Remove-Item $workDir -Recurse -Force -ErrorAction SilentlyContinue
}

##############################################################################
# PATH
##############################################################################

$userPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
$onPath = $userPath -and (@($userPath -split ';') | Where-Object { $_.TrimEnd('\') -eq $installDir.TrimEnd('\') })
if (-not $onPath) {
    $joined = if ($userPath) { "$userPath;$installDir" } else { $installDir }
    [Environment]::SetEnvironmentVariable('PATH', $joined, 'User')
    # The parent shell keeps the environment it started with.
    $env:PATH = "$env:PATH;$installDir"
    $pathNote = $true
}

Write-Host ''
Write-Host 'Markov installed.'
if (-not $CliOnly) {
    Write-Host "  app: $appExe"
}
Write-Host "  cli: $cliPath"
if ($pathNote) {
    Write-Host ''
    Write-Host "$installDir was added to your PATH — open a new terminal for it to take effect."
}
Write-Host ''
Write-Host 'Updates are not automatic — run this installer again for a new version.'

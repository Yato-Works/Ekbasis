#requires -Version 5.1
<#
.SYNOPSIS
    Ekbasis universal installer script for Windows (PowerShell).
.DESCRIPTION
    Installs Ekbasis (Software Timeline & Counterfactual Experiment Engine).
    Downloads pre-built binaries from GitHub Releases or compiles via Cargo if available.
.EXAMPLE
    irm https://raw.githubusercontent.com/Yato-Works/Ekbasis/main/install.ps1 | iex
#>

$ErrorActionPreference = 'Stop'
$repo = 'Yato-Works/Ekbasis'
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Ekbasis\bin'

Write-Host "==> Installing Ekbasis (Software Timeline & Counterfactual Experiment Engine)..." -ForegroundColor Cyan

$arch = if ([System.Environment]::Is64BitOperatingSystem) {
    if ($env:PROCESSOR_ARCHITECTURE -match 'ARM') { 'aarch64' } else { 'x86_64' }
} else {
    Write-Error "Unsupported 32-bit architecture. Ekbasis requires a 64-bit operating system."
    exit 1
}

if (-not (Test-Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}

$downloadSuccess = $false
try {
    $releaseUrl = "https://api.github.com/repos/$repo/releases/latest"
    $headers = @{ "User-Agent" = "Ekbasis-Installer" }
    $release = Invoke-RestMethod -Uri $releaseUrl -Headers $headers -ErrorAction SilentlyContinue
    if ($release -and $release.tag_name) {
        $tag = $release.tag_name
        $assetName = "ekbasis-$tag-$arch-windows.zip"
        $asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
        if ($asset -and $asset.browser_download_url) {
            $zipPath = Join-Path ([System.IO.Path]::GetTempPath()) $assetName
            Write-Host "Downloading $assetName from GitHub Releases..." -ForegroundColor Gray
            Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath
            Expand-Archive -Path $zipPath -DestinationPath $installDir -Force
            Remove-Item -Force $zipPath
            $downloadSuccess = $true
        }
    }
} catch {
    # Fallback to Cargo
}

if (-not $downloadSuccess) {
    if (Get-Command cargo -ErrorAction SilentlyContinue) {
        Write-Host "Note: Pre-built release binary not found for windows-$arch. Building via Cargo..." -ForegroundColor Yellow
        cargo install --git "https://github.com/$repo.git" ekbasis-cli --bin ekbasis --root (Split-Path $installDir -Parent)
        $downloadSuccess = $true
    } else {
        Write-Error @"
Neither pre-built binary nor Cargo was found on your system.
Please install Rust (https://rustup.rs/) or manually download releases from:
https://github.com/$repo/releases
"@
        exit 1
    }
}

$exePath = Join-Path $installDir 'ekbasis.exe'
if (Test-Path $exePath) {
    Write-Host "==> Ekbasis successfully installed to $exePath" -ForegroundColor Green

    # Check and configure user PATH
    $currentPath = [System.Environment]::GetEnvironmentVariable('PATH', 'User')
    if ($currentPath -split ';' -notcontains $installDir) {
        $newPath = "$currentPath;$installDir".Trim(';')
        [System.Environment]::SetEnvironmentVariable('PATH', $newPath, 'User')
        $env:PATH = "$env:PATH;$installDir"
        Write-Host "==> Added $installDir to User PATH." -ForegroundColor Green
        Write-Host "    (Please restart your terminal session for PATH changes to take full effect)" -ForegroundColor Yellow
    }

    Write-Host "`nRun 'ekbasis --help' to get started!" -ForegroundColor Cyan
} else {
    Write-Error "Installation failed: ekbasis.exe was not found in $installDir"
    exit 1
}

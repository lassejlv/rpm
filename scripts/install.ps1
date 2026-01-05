#!/usr/bin/env pwsh
# RPM Installer Script for Windows
# This script builds and installs rpm from source

$ErrorActionPreference = "Stop"

# Colors for output
function Write-Warning-Banner {
    Write-Host ""
    Write-Host "╔══════════════════════════════════════════════════════════════════╗" -ForegroundColor Yellow
    Write-Host "║                           WARNING                                 ║" -ForegroundColor Yellow
    Write-Host "╠══════════════════════════════════════════════════════════════════╣" -ForegroundColor Yellow
    Write-Host "║  This script will build rpm from source and install it to your   ║" -ForegroundColor Yellow
    Write-Host "║  Cargo bin directory (~/.cargo/bin).                             ║" -ForegroundColor Yellow
    Write-Host "║                                                                  ║" -ForegroundColor Yellow
    Write-Host "║  Requirements:                                                   ║" -ForegroundColor Yellow
    Write-Host "║    - Rust toolchain (rustc, cargo)                               ║" -ForegroundColor Yellow
    Write-Host "║    - Internet connection to download dependencies                ║" -ForegroundColor Yellow
    Write-Host "║                                                                  ║" -ForegroundColor Yellow
    Write-Host "║  The build process may take several minutes and will use         ║" -ForegroundColor Yellow
    Write-Host "║  significant CPU and memory resources.                           ║" -ForegroundColor Yellow
    Write-Host "╚══════════════════════════════════════════════════════════════════╝" -ForegroundColor Yellow
    Write-Host ""
}

function Write-Step {
    param([string]$Message)
    Write-Host "[*] " -ForegroundColor Cyan -NoNewline
    Write-Host $Message
}

function Write-Success {
    param([string]$Message)
    Write-Host "[✓] " -ForegroundColor Green -NoNewline
    Write-Host $Message
}

function Write-Error-Message {
    param([string]$Message)
    Write-Host "[✗] " -ForegroundColor Red -NoNewline
    Write-Host $Message
}

# Display warning banner
Write-Warning-Banner

# Ask for confirmation
$confirmation = Read-Host "Do you want to continue with the installation? (y/N)"
if ($confirmation -notmatch "^[Yy]$") {
    Write-Host "Installation cancelled." -ForegroundColor Yellow
    exit 0
}

Write-Host ""

# Check for Rust installation
Write-Step "Checking for Rust installation..."
try {
    $rustVersion = rustc --version 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "rustc not found"
    }
    Write-Success "Found $rustVersion"
} catch {
    Write-Error-Message "Rust is not installed!"
    Write-Host ""
    Write-Host "Please install Rust first:" -ForegroundColor Yellow
    Write-Host "  https://rustup.rs" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Or run this command in PowerShell:" -ForegroundColor Yellow
    Write-Host "  irm https://win.rustup.rs | iex" -ForegroundColor Cyan
    exit 1
}

# Check for Cargo
Write-Step "Checking for Cargo..."
try {
    $cargoVersion = cargo --version 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo not found"
    }
    Write-Success "Found $cargoVersion"
} catch {
    Write-Error-Message "Cargo is not installed!"
    Write-Host "Please reinstall Rust using rustup: https://rustup.rs" -ForegroundColor Yellow
    exit 1
}

# Get script directory (where the source code is)
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $scriptDir) {
    $scriptDir = Get-Location
}

# Check if Cargo.toml exists
Write-Step "Checking for Cargo.toml..."
$cargoToml = Join-Path $scriptDir "Cargo.toml"
if (-not (Test-Path $cargoToml)) {
    Write-Error-Message "Cargo.toml not found in $scriptDir"
    Write-Host "Please run this script from the rpm source directory." -ForegroundColor Yellow
    exit 1
}
Write-Success "Found Cargo.toml"

# Build and install
Write-Host ""
Write-Step "Building and installing rpm (this may take a few minutes)..."
Write-Host ""

Push-Location $scriptDir
try {
    cargo install --path .
    if ($LASTEXITCODE -ne 0) {
        throw "Build failed"
    }
} catch {
    Write-Error-Message "Failed to build rpm!"
    Write-Host "Please check the error messages above." -ForegroundColor Yellow
    Pop-Location
    exit 1
}
Pop-Location

Write-Host ""
Write-Success "rpm has been successfully installed!"
Write-Host ""

# Check if cargo bin is in PATH
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if ($env:PATH -notlike "*$cargoBin*") {
    Write-Host "Note: Make sure $cargoBin is in your PATH" -ForegroundColor Yellow
    Write-Host ""
}

# Verify installation
Write-Step "Verifying installation..."
try {
    $rpmVersion = rpm --version 2>&1
    if ($LASTEXITCODE -eq 0) {
        Write-Success "rpm $rpmVersion is ready to use!"
    } else {
        Write-Host "rpm was installed but may require a shell restart to use." -ForegroundColor Yellow
    }
} catch {
    Write-Host "rpm was installed but may require a shell restart to use." -ForegroundColor Yellow
}

Write-Host ""
Write-Host "Run 'rpm --help' to get started." -ForegroundColor Cyan
Write-Host ""

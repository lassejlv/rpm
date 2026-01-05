#!/usr/bin/env bash
# RPM Installer Script for Unix/Linux/macOS
# This script builds and installs rpm from source

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Print warning banner
print_warning_banner() {
    echo ""
    echo -e "${YELLOW}╔══════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${YELLOW}║                           WARNING                                 ║${NC}"
    echo -e "${YELLOW}╠══════════════════════════════════════════════════════════════════╣${NC}"
    echo -e "${YELLOW}║  This script will build rpm from source and install it to your   ║${NC}"
    echo -e "${YELLOW}║  Cargo bin directory (~/.cargo/bin).                             ║${NC}"
    echo -e "${YELLOW}║                                                                  ║${NC}"
    echo -e "${YELLOW}║  Requirements:                                                   ║${NC}"
    echo -e "${YELLOW}║    - Rust toolchain (rustc, cargo)                               ║${NC}"
    echo -e "${YELLOW}║    - Internet connection to download dependencies                ║${NC}"
    echo -e "${YELLOW}║                                                                  ║${NC}"
    echo -e "${YELLOW}║  The build process may take several minutes and will use         ║${NC}"
    echo -e "${YELLOW}║  significant CPU and memory resources.                           ║${NC}"
    echo -e "${YELLOW}╚══════════════════════════════════════════════════════════════════╝${NC}"
    echo ""
}

print_step() {
    echo -e "${CYAN}[*]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[✓]${NC} $1"
}

print_error() {
    echo -e "${RED}[✗]${NC} $1"
}

# Display warning banner
print_warning_banner

# Ask for confirmation
read -p "Do you want to continue with the installation? (y/N) " -n 1 -r
echo ""
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo -e "${YELLOW}Installation cancelled.${NC}"
    exit 0
fi

echo ""

# Check for Rust installation
print_step "Checking for Rust installation..."
if command -v rustc &> /dev/null; then
    RUST_VERSION=$(rustc --version)
    print_success "Found $RUST_VERSION"
else
    print_error "Rust is not installed!"
    echo ""
    echo -e "${YELLOW}Please install Rust first:${NC}"
    echo -e "${CYAN}  https://rustup.rs${NC}"
    echo ""
    echo -e "${YELLOW}Or run this command:${NC}"
    echo -e "${CYAN}  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${NC}"
    exit 1
fi

# Check for Cargo
print_step "Checking for Cargo..."
if command -v cargo &> /dev/null; then
    CARGO_VERSION=$(cargo --version)
    print_success "Found $CARGO_VERSION"
else
    print_error "Cargo is not installed!"
    echo -e "${YELLOW}Please reinstall Rust using rustup: https://rustup.rs${NC}"
    exit 1
fi

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Check if Cargo.toml exists
print_step "Checking for Cargo.toml..."
if [[ ! -f "$SCRIPT_DIR/Cargo.toml" ]]; then
    print_error "Cargo.toml not found in $SCRIPT_DIR"
    echo -e "${YELLOW}Please run this script from the rpm source directory.${NC}"
    exit 1
fi
print_success "Found Cargo.toml"

# Build and install
echo ""
print_step "Building and installing rpm (this may take a few minutes)..."
echo ""

cd "$SCRIPT_DIR"
if ! cargo install --path .; then
    print_error "Failed to build rpm!"
    echo -e "${YELLOW}Please check the error messages above.${NC}"
    exit 1
fi

echo ""
print_success "rpm has been successfully installed!"
echo ""

# Check if cargo bin is in PATH
CARGO_BIN="$HOME/.cargo/bin"
if [[ ":$PATH:" != *":$CARGO_BIN:"* ]]; then
    echo -e "${YELLOW}Note: Make sure $CARGO_BIN is in your PATH${NC}"
    echo ""
    echo -e "${YELLOW}Add this line to your shell profile (~/.bashrc, ~/.zshrc, etc.):${NC}"
    echo -e "${CYAN}  export PATH=\"\$HOME/.cargo/bin:\$PATH\"${NC}"
    echo ""
fi

# Verify installation
print_step "Verifying installation..."
if command -v rpm &> /dev/null; then
    RPM_VERSION=$(rpm --version 2>/dev/null || echo "installed")
    print_success "rpm $RPM_VERSION is ready to use!"
else
    echo -e "${YELLOW}rpm was installed but may require a shell restart to use.${NC}"
fi

echo ""
echo -e "${CYAN}Run 'rpm --help' to get started.${NC}"
echo ""

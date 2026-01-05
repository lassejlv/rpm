# rpm

A simple and fast Rust-based Package Manager for the npm registry. It allows you to install dependencies from `package.json`, add new packages, run scripts, and manage a local cache, serving as a lightweight alternative to npm or yarn.

## Installation

### Using the installer script (recommended)

Clone the repository and run the installer script:

```bash
git clone https://github.com/lassejlv/rpm
cd rpm

# On Windows (PowerShell)
.\scripts\install.ps1

# On Linux/macOS
./scripts/install.sh
```

> **Warning:** The installer will build rpm from source, which requires the Rust toolchain and may take several minutes. It will use significant CPU and memory resources during compilation.

### Manual installation

Alternatively, you can install manually using Cargo:

```bash
git clone https://github.com/lassejlv/rpm
cd rpm
cargo install --path .
```

## Usage

### Install Dependencies

Install all dependencies from `package.json`:

```bash
rpm install
# or simply
rpm
```

### Add Packages

Add one or more packages to your project:

```bash
# Add a package to dependencies
rpm add lodash

# Add a specific version
rpm add lodash@4.17.21

# Add multiple packages
rpm add react react-dom

# Add as dev dependency
rpm add -D typescript
rpm add --save-dev eslint
rpm add --dev prettier
```

### Remove Packages

Remove one or more packages from your project:

```bash
rpm remove lodash

# Using aliases
rpm rm lodash
rpm uninstall lodash
rpm un lodash

# Remove multiple packages
rpm remove lodash express axios
```

### Run Scripts

Run scripts defined in your `package.json`:

```bash
rpm run test
rpm run build
rpm run dev

# Pass arguments to the script
rpm run test -- --watch --coverage
```

### Execute Packages (npx alternative)

Execute a package binary without installing it permanently:

```bash
# Execute a package
rpm x cowsay hello

# Execute a specific version
rpm x prettier@3.0.0 --check .

# Using the exec alias
rpm exec eslint .

# Pass arguments
rpm x typescript -- --init
```

If the package is already installed locally in `node_modules/.bin`, it will use that version. Otherwise, it will fetch and cache the package temporarily.

### List Packages

List all installed packages:

```bash
rpm list

# Using the alias
rpm ls
```

### Why Package

Show why a package is installed (what depends on it):

```bash
rpm why lodash
```

### Cache Management

Manage the global package cache:

```bash
# Show cache info (location, size, package count)
rpm cache info

# Clear the cache
rpm cache clean
```

## Commands

| Command | Aliases | Description |
|---------|---------|-------------|
| `install` | (none) | Install dependencies from package.json |
| `add` | (none) | Add one or more packages |
| `remove` | `rm`, `uninstall`, `un` | Remove one or more packages |
| `run` | (none) | Run a script from package.json |
| `x` | `exec` | Execute a package binary (like npx) |
| `list` | `ls` | List installed packages |
| `why` | (none) | Show why a package is installed |
| `cache` | (none) | Manage package cache |

## Global Options

These options can be used with any command:

| Option | Description |
|--------|-------------|
| `--force-no-cache` | Force download and ignore cache |
| `--yes` | Skip postinstall script confirmation |
| `--ignore-scripts` | Skip postinstall scripts entirely |
| `-h, --help` | Print help information |
| `-V, --version` | Print version |

## Features

- **Fast**: Written in Rust with concurrent package downloads
- **Simple**: Minimal and intuitive CLI interface
- **Caching**: Global package cache to speed up repeated installs
- **Lockfile**: Generates `rpm-lock.json` for reproducible builds
- **Binary Linking**: Automatically links package binaries to `node_modules/.bin`
- **Postinstall Scripts**: Supports postinstall scripts with confirmation prompt
- **Dev Dependencies**: Full support for dev dependencies

## Files

| File | Description |
|------|-------------|
| `package.json` | Project manifest with dependencies and scripts |
| `rpm-lock.json` | Lockfile for reproducible installs |
| `node_modules/` | Installed packages directory |
| `node_modules/.bin/` | Linked package binaries |

## License

MIT
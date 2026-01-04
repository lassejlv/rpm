# rpm

A simple and fast Rust-based Package Manager for the npm registry. It allows you to install dependencies from `package.json`, add new packages, run scripts, and manage a local cache, serving as a lightweight alternative to npm or yarn.

## Installation

You can install `rpm` by cloning the repository and building it from source:

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

### Cache Management

Manage the global package cache:

```bash
# Show cache info (location, size, package count)
rpm cache info

# Clear the cache
rpm cache clean
```

## Global Options

These options can be used with any command:

| Option | Description |
|--------|-------------|
| `--force-no-cache` | Force download and ignore cache |
| `--yes` | Skip postinstall script confirmation |
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
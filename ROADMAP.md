# RPM Roadmap: npm Compatibility

This document outlines the planned features and improvements to achieve better npm compatibility.

## Current Status

RPM currently supports:
- [x] Installing dependencies from `package.json`
- [x] Adding/removing packages
- [x] Dev dependencies (`--save-dev`)
- [x] Running scripts (`rpm run`)
- [x] Package execution (`rpm x` / `rpm exec`)
- [x] Lockfile generation (`rpm-lock.json`)
- [x] Binary linking to `node_modules/.bin`
- [x] Postinstall scripts
- [x] Global package cache

## Short-term Goals

### Package Management
- [x] Peer dependencies support
- [x] Optional dependencies support
- [x] `npm update` equivalent command
- [x] `npm outdated` equivalent command
- [x] `npm list` / `npm ls` equivalent command
- [x] `npm dedupe` equivalent command

### Versioning
- [x] Support for `~` version ranges (patch-level changes)
- [x] Support for `^` version ranges (minor-level changes)
- [x] Support for `>=`, `<=`, `>`, `<` version constraints
- [x] Support for `||` (OR) in version ranges
- [ ] Git repository dependencies (`git+https://...`)
- [ ] GitHub shorthand (`user/repo`)
- [ ] Local file dependencies (`file:../path`)

### Workspaces
- [ ] Basic workspace support
- [ ] `workspaces` field in `package.json`
- [ ] Hoisting shared dependencies
- [ ] Running scripts across workspaces

## Medium-term Goals

### Security
- [ ] `npm audit` equivalent command
- [ ] Vulnerability scanning
- [ ] Package signature verification

### Publishing
- [ ] `npm login` equivalent
- [ ] `npm publish` equivalent
- [ ] `npm pack` equivalent
- [ ] `.npmignore` support

### Configuration
- [ ] `.npmrc` file support
- [ ] Custom registry configuration
- [ ] Scoped package registry configuration
- [ ] Proxy support

### Compatibility
- [ ] `package-lock.json` reading (npm lockfile)
- [ ] `yarn.lock` reading
- [ ] `pnpm-lock.yaml` reading
- [ ] Lifecycle scripts (`preinstall`, `prepare`, `prepublish`, etc.)

## Long-term Goals

### Performance
- [ ] Parallel script execution
- [ ] Lazy dependency resolution
- [ ] Incremental installs
- [ ] Hard linking for duplicate packages

### Advanced Features
- [ ] `npm link` for local development
- [ ] `npm ci` equivalent (clean install from lockfile)
- [ ] `npm shrinkwrap` equivalent
- [ ] `npm prune` equivalent
- [ ] Overrides/resolutions support
- [ ] Platform-specific optional dependencies (`os`, `cpu` fields)

### Developer Experience
- [ ] Interactive mode for adding packages
- [ ] Better error messages with suggestions
- [ ] Progress reporting improvements
- [ ] Offline mode

## Non-goals

Some npm features are intentionally out of scope:
- npm organizations management
- npm token management
- npm hooks
- npm stars/profile features

## Contributing

Contributions are welcome! If you'd like to work on any of these features, please open an issue first to discuss the implementation approach.

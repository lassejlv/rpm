use crate::installer::Installer;
use crate::output::{colors, RpmError};
use crate::registry::{parse_package_alias, Registry};
use crate::types::{LockFile, LockPackage, PackageJson, RegistryVersion};
use crate::workspace::Workspace;
use anyhow::{Context, Result};
use dashmap::DashMap;
use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Semaphore;

/// Get the current OS name in npm's format
fn get_current_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "freebsd") {
        "freebsd"
    } else if cfg!(target_os = "openbsd") {
        "openbsd"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        "unknown"
    }
}

/// Get the current CPU architecture in npm's format
fn get_current_cpu() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86") {
        "ia32"
    } else if cfg!(target_arch = "arm") {
        "arm"
    } else {
        "unknown"
    }
}

/// Check if a package is compatible with the current platform
fn is_platform_compatible(os: &[String], cpu: &[String]) -> bool {
    let os_ok = if os.is_empty() {
        true
    } else {
        let current_os = get_current_os();
        // Check for negation pattern (e.g., "!win32" means "not windows")
        let has_negation = os.iter().any(|s| s.starts_with('!'));
        if has_negation {
            // If there are negations, the package is compatible if current OS is NOT in the negated list
            !os.iter().any(|s| s.strip_prefix('!') == Some(current_os))
        } else {
            // Otherwise, current OS must be in the list
            os.iter().any(|s| s == current_os)
        }
    };

    let cpu_ok = if cpu.is_empty() {
        true
    } else {
        let current_cpu = get_current_cpu();
        let has_negation = cpu.iter().any(|s| s.starts_with('!'));
        if has_negation {
            !cpu.iter().any(|s| s.strip_prefix('!') == Some(current_cpu))
        } else {
            cpu.iter().any(|s| s == current_cpu)
        }
    };

    os_ok && cpu_ok
}

/// Check if a RegistryVersion is compatible with current platform
fn is_version_platform_compatible(version: &RegistryVersion) -> bool {
    is_platform_compatible(&version.os, &version.cpu)
}

#[derive(Clone)]
pub struct Manager {
    registry: Registry,
    installer: Installer,
    installed: Arc<DashMap<String, String>>,
    semaphore: Arc<Semaphore>,
    multi_progress: MultiProgress,
    lockfile: Arc<tokio::sync::Mutex<LockFile>>,
    postinstalls: Arc<DashMap<String, (PathBuf, String)>>,
    auto_confirm: bool,
    ignore_scripts: bool,
    // Progress tracking
    packages_installed: Arc<AtomicUsize>,
    packages_resolved: Arc<AtomicUsize>,
    packages_cached: Arc<AtomicUsize>,
    progress_bar: Arc<tokio::sync::Mutex<Option<ProgressBar>>>,
    // Track currently processing packages for better progress display
    current_packages: Arc<DashMap<String, String>>, // name -> status ("resolving", "installing")
    install_start_time: Arc<tokio::sync::Mutex<Option<std::time::Instant>>>,
}

impl Manager {
    pub fn new(force_no_cache: bool, auto_confirm: bool, ignore_scripts: bool) -> Self {
        Self {
            registry: Registry::new(),
            installer: Installer::new(force_no_cache),
            installed: Arc::new(DashMap::new()),
            semaphore: Arc::new(Semaphore::new(50)), // Limit concurrency
            multi_progress: MultiProgress::new(),
            lockfile: Arc::new(tokio::sync::Mutex::new(LockFile {
                name: "".to_string(),
                version: "".to_string(),
                lockfile_version: 3,
                packages: BTreeMap::new(),
            })),
            postinstalls: Arc::new(DashMap::new()),
            auto_confirm,
            ignore_scripts,
            packages_installed: Arc::new(AtomicUsize::new(0)),
            packages_resolved: Arc::new(AtomicUsize::new(0)),
            packages_cached: Arc::new(AtomicUsize::new(0)),
            progress_bar: Arc::new(tokio::sync::Mutex::new(None)),
            current_packages: Arc::new(DashMap::new()),
            install_start_time: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    fn create_spinner(&self) -> ProgressBar {
        let spinner = self.multi_progress.add(ProgressBar::new_spinner());
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
        );
        spinner.enable_steady_tick(std::time::Duration::from_millis(80));
        spinner
    }

    fn create_install_progress(&self) -> ProgressBar {
        let pb = self.multi_progress.add(ProgressBar::new_spinner());
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        pb
    }

    fn update_progress(&self) {
        let installed = self.packages_installed.load(Ordering::Relaxed);
        let resolved = self.packages_resolved.load(Ordering::Relaxed);
        let cached = self.packages_cached.load(Ordering::Relaxed);

        // Get current package being processed (most recent one)
        let current_pkg: Option<String> = self
            .current_packages
            .iter()
            .next()
            .map(|e| format!("{} {}", e.value(), e.key()));

        let mut msg = format!(
            "{}Resolved{} {}{}{} {}│{}  {}Installed{} {}{}{}",
            colors::BOLD,
            colors::RESET,
            colors::CYAN,
            resolved,
            colors::RESET,
            colors::GRAY,
            colors::RESET,
            colors::BOLD,
            colors::RESET,
            colors::GREEN,
            installed,
            colors::RESET
        );

        if cached > 0 {
            msg.push_str(&format!(
                "  {}│{}  {}Cached{} {}{}{}",
                colors::GRAY,
                colors::RESET,
                colors::GRAY,
                colors::RESET,
                colors::YELLOW,
                cached,
                colors::RESET
            ));
        }

        // Show current package if available
        if let Some(pkg) = current_pkg {
            msg.push_str(&format!(
                "  {}│{}  {}{}{}",
                colors::GRAY,
                colors::RESET,
                colors::GRAY,
                pkg,
                colors::RESET
            ));
        }

        if let Ok(guard) = self.progress_bar.try_lock() {
            if let Some(pb) = guard.as_ref() {
                pb.set_message(msg);
            }
        }
    }

    /// Mark a package as currently being processed
    fn set_current_package(&self, name: &str, status: &str) {
        self.current_packages.insert(name.to_string(), status.to_string());
    }

    /// Remove a package from the current processing list
    fn clear_current_package(&self, name: &str) {
        self.current_packages.remove(name);
    }

    fn reset_progress(&self) {
        self.packages_installed.store(0, Ordering::Relaxed);
        self.packages_resolved.store(0, Ordering::Relaxed);
        self.packages_cached.store(0, Ordering::Relaxed);
    }

    async fn load_lockfile(&self) -> Result<()> {
        if let Ok(content) = fs::read_to_string("rpm-lock.json").await {
            let lock: LockFile = serde_json::from_str(&content).unwrap_or_else(|_| LockFile {
                name: "".to_string(),
                version: "".to_string(),
                lockfile_version: 3,
                packages: BTreeMap::new(),
            });
            *self.lockfile.lock().await = lock;
        }
        Ok(())
    }

    async fn save_lockfile(&self, package_name: &str, package_version: &str) -> Result<()> {
        let mut lock = self.lockfile.lock().await;
        lock.name = package_name.to_string();
        lock.version = package_version.to_string();
        let content = serde_json::to_string_pretty(&*lock)?;
        fs::write("rpm-lock.json", content).await?;
        Ok(())
    }

    pub async fn list_packages(&self) -> Result<()> {
        let package_json_content = fs::read_to_string("package.json")
            .await
            .context("Could not find package.json in current directory")?;
        let package_json: PackageJson = serde_json::from_str(&package_json_content)?;

        println!(
            "\x1b[1m{}@{}\x1b[0m",
            package_json.name, package_json.version
        );

        let has_deps = !package_json.dependencies.is_empty();
        let has_dev_deps = !package_json.dev_dependencies.is_empty();

        if !has_deps && !has_dev_deps {
            println!("\x1b[90m(no dependencies)\x1b[0m");
            return Ok(());
        }

        if has_deps {
            println!("\n\x1b[1;36mDependencies:\x1b[0m");
            for (name, version) in &package_json.dependencies {
                let installed = self.get_installed_version(name).await;
                match installed {
                    Some(v) => println!(
                        "  \x1b[32m├─\x1b[0m {}@\x1b[90m{}\x1b[0m (installed: \x1b[36m{}\x1b[0m)",
                        name, version, v
                    ),
                    None => println!(
                        "  \x1b[33m├─\x1b[0m {}@\x1b[90m{}\x1b[0m \x1b[33m(not installed)\x1b[0m",
                        name, version
                    ),
                }
            }
        }

        if has_dev_deps {
            println!("\n\x1b[1;35mDev Dependencies:\x1b[0m");
            for (name, version) in &package_json.dev_dependencies {
                let installed = self.get_installed_version(name).await;
                match installed {
                    Some(v) => println!(
                        "  \x1b[32m├─\x1b[0m {}@\x1b[90m{}\x1b[0m (installed: \x1b[36m{}\x1b[0m)",
                        name, version, v
                    ),
                    None => println!(
                        "  \x1b[33m├─\x1b[0m {}@\x1b[90m{}\x1b[0m \x1b[33m(not installed)\x1b[0m",
                        name, version
                    ),
                }
            }
        }

        Ok(())
    }

    pub async fn outdated_packages(&self) -> Result<()> {
        let package_json_content = fs::read_to_string("package.json")
            .await
            .context("Could not find package.json in current directory")?;
        let package_json: PackageJson = serde_json::from_str(&package_json_content)?;

        let has_deps = !package_json.dependencies.is_empty();
        let has_dev_deps = !package_json.dev_dependencies.is_empty();

        if !has_deps && !has_dev_deps {
            println!("\x1b[90m(no dependencies)\x1b[0m");
            return Ok(());
        }

        let spinner = self.create_spinner();
        spinner.set_message("\x1b[1mChecking\x1b[0m for updates...");

        let mut outdated: Vec<(String, String, String, String, bool)> = Vec::new(); // (name, current, wanted, latest, is_dev)

        // Collect all deps to check
        let deps_to_check: Vec<(String, String, bool)> = package_json
            .dependencies
            .iter()
            .map(|(n, v)| (n.clone(), v.clone(), false))
            .chain(
                package_json
                    .dev_dependencies
                    .iter()
                    .map(|(n, v)| (n.clone(), v.clone(), true)),
            )
            .collect();

        // Check all dependencies in parallel
        let mut tasks = FuturesUnordered::new();
        for (name, version_range, is_dev) in deps_to_check {
            let manager = self.clone();
            tasks.push(async move {
                let result = manager.check_outdated(&name, &version_range).await;
                (name, result, is_dev)
            });
        }

        while let Some((name, result, is_dev)) = tasks.next().await {
            if let Some((current, wanted, latest)) = result {
                outdated.push((name, current, wanted, latest, is_dev));
            }
        }

        spinner.finish_and_clear();

        if outdated.is_empty() {
            println!("\x1b[32m✓\x1b[0m All packages are up to date!");
            return Ok(());
        }

        // Print header
        println!(
            "\x1b[1m{:<30} {:>12} {:>12} {:>12}  {}\x1b[0m",
            "Package", "Current", "Wanted", "Latest", "Type"
        );
        println!("{}", "─".repeat(78));

        for (name, current, wanted, latest, is_dev) in &outdated {
            let type_label = if *is_dev {
                "\x1b[35mdev\x1b[0m"
            } else {
                "\x1b[36mdep\x1b[0m"
            };

            let wanted_color = if wanted != current {
                "\x1b[33m"
            } else {
                "\x1b[90m"
            };
            let latest_color = if latest != current {
                "\x1b[31m"
            } else {
                "\x1b[90m"
            };

            println!(
                "{:<30} \x1b[90m{:>12}\x1b[0m {:>12} {:>12}  {}",
                name,
                current,
                format!("{}{}\x1b[0m", wanted_color, wanted),
                format!("{}{}\x1b[0m", latest_color, latest),
                type_label
            );
        }

        println!();
        println!(
            "\x1b[90m{} package(s) can be updated\x1b[0m",
            outdated.len()
        );

        Ok(())
    }

    async fn check_outdated(
        &self,
        name: &str,
        version_range: &str,
    ) -> Option<(String, String, String)> {
        // Get installed version
        let current = self.get_installed_version(name).await?;

        // Fetch latest from registry
        let package = self.registry.get_package(name).await.ok()?;
        let latest = package.dist_tags.get("latest")?.clone();

        // Resolve wanted version based on version range
        let wanted = self
            .registry
            .resolve_version(&package, version_range)
            .ok()
            .map(|v| v.version.clone())
            .unwrap_or_else(|| current.clone());

        // Only return if there's an update available
        if current != wanted || current != latest {
            Some((current, wanted, latest))
        } else {
            None
        }
    }

    pub async fn update_packages(&self, packages: Vec<String>) -> Result<()> {
        self.load_lockfile().await?;
        let package_json_content = fs::read_to_string("package.json")
            .await
            .context("Could not find package.json in current directory")?;
        let mut package_json: PackageJson = serde_json::from_str(&package_json_content)?;

        let spinner = self.create_spinner();
        spinner.set_message("\x1b[1mChecking\x1b[0m for updates...");

        let mut to_update: Vec<(String, String, String, bool)> = Vec::new(); // (name, old_version, new_version, is_dev)

        // Determine which packages to check
        let check_all = packages.is_empty();

        // Collect deps to check
        let deps_to_check: Vec<(String, bool)> = package_json
            .dependencies
            .keys()
            .filter(|n| check_all || packages.contains(*n))
            .map(|n| (n.clone(), false))
            .chain(
                package_json
                    .dev_dependencies
                    .keys()
                    .filter(|n| check_all || packages.contains(*n))
                    .map(|n| (n.clone(), true)),
            )
            .collect();

        // Check all dependencies in parallel
        let mut tasks = FuturesUnordered::new();
        for (name, is_dev) in deps_to_check {
            let manager = self.clone();
            tasks.push(async move {
                let result = manager.get_latest_version(&name).await;
                (name, result, is_dev)
            });
        }

        while let Some((name, result, is_dev)) = tasks.next().await {
            if let Some((current, latest)) = result {
                if current != latest {
                    to_update.push((name, current, latest, is_dev));
                }
            }
        }

        spinner.finish_and_clear();

        if to_update.is_empty() {
            println!("\x1b[32m✓\x1b[0m All packages are up to date!");
            return Ok(());
        }

        // Update package.json with new versions
        for (name, old_version, new_version, is_dev) in &to_update {
            println!(
                "\x1b[36m↑\x1b[0m \x1b[1m{}\x1b[0m \x1b[90m{}\x1b[0m → \x1b[32m{}\x1b[0m",
                name, old_version, new_version
            );

            if *is_dev {
                package_json
                    .dev_dependencies
                    .insert(name.clone(), format!("^{}", new_version));
            } else {
                package_json
                    .dependencies
                    .insert(name.clone(), format!("^{}", new_version));
            }

            // Remove from lockfile to force re-fetch
            {
                let mut lock = self.lockfile.lock().await;
                let key = format!("node_modules/{}", name);
                lock.packages.remove(&key);
            }

            // Remove from node_modules
            let pkg_path = PathBuf::from("node_modules").join(name);
            if pkg_path.exists() {
                let _ = fs::remove_dir_all(&pkg_path).await;
            }
        }

        // Save updated package.json
        let new_content = serde_json::to_string_pretty(&package_json)?;
        fs::write("package.json", new_content).await?;

        println!();

        // Reset and setup progress tracking
        self.reset_progress();
        let pb = self.create_install_progress();
        pb.set_message("\x1b[1mInstalling\x1b[0m updates...");
        *self.progress_bar.lock().await = Some(pb.clone());

        self.install_deps(&package_json).await?;

        let installed = self.packages_installed.load(Ordering::Relaxed);
        let cached = self.packages_cached.load(Ordering::Relaxed);

        pb.finish_and_clear();
        *self.progress_bar.lock().await = None;

        // Print summary
        if installed > 0 || cached > 0 {
            let mut parts = Vec::new();
            if installed > 0 {
                parts.push(format!("\x1b[32m+{}\x1b[0m installed", installed));
            }
            if cached > 0 {
                parts.push(format!("\x1b[33m{}\x1b[0m cached", cached));
            }
            println!("{}", parts.join("  \x1b[90m│\x1b[0m  "));
        }

        self.run_postinstalls().await?;
        self.save_lockfile(&package_json.name, &package_json.version)
            .await?;

        println!("\n\x1b[32m✓\x1b[0m Updated {} package(s)", to_update.len());

        Ok(())
    }

    pub async fn dedupe_packages(&self) -> Result<()> {
        let package_json_content = fs::read_to_string("package.json")
            .await
            .context("Could not find package.json in current directory")?;
        let package_json: PackageJson = serde_json::from_str(&package_json_content)?;

        let node_modules = std::env::current_dir()?.join("node_modules");
        if !node_modules.exists() {
            println!("\x1b[33m!\x1b[0m No node_modules found. Run 'rpm install' first.");
            return Ok(());
        }

        let spinner = self.create_spinner();
        spinner.set_message("\x1b[1mAnalyzing\x1b[0m dependencies...");

        let mut duplicates_found = 0;
        let mut bytes_saved: u64 = 0;

        // Scan for nested node_modules
        let mut to_check: Vec<PathBuf> = vec![node_modules.clone()];

        while let Some(dir) = to_check.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();

                if !path.is_dir() || name.starts_with('.') {
                    continue;
                }

                // Handle scoped packages
                if name.starts_with('@') {
                    to_check.push(path);
                    continue;
                }

                // Check for nested node_modules
                let nested_nm = path.join("node_modules");
                if nested_nm.exists() {
                    if let Ok(mut nested_entries) = tokio::fs::read_dir(&nested_nm).await {
                        while let Ok(Some(nested_entry)) = nested_entries.next_entry().await {
                            let nested_path = nested_entry.path();
                            let nested_name =
                                nested_entry.file_name().to_string_lossy().to_string();

                            if nested_name.starts_with('.') || !nested_path.is_dir() {
                                continue;
                            }

                            // Handle scoped packages in nested node_modules
                            if nested_name.starts_with('@') {
                                if let Ok(mut scoped_entries) =
                                    tokio::fs::read_dir(&nested_path).await
                                {
                                    while let Ok(Some(scoped_entry)) =
                                        scoped_entries.next_entry().await
                                    {
                                        let scoped_path = scoped_entry.path();
                                        let scoped_pkg_name = format!(
                                            "{}/{}",
                                            nested_name,
                                            scoped_entry.file_name().to_string_lossy()
                                        );

                                        if let Some(saved) = self
                                            .try_dedupe_package(
                                                &node_modules,
                                                &scoped_path,
                                                &scoped_pkg_name,
                                            )
                                            .await
                                        {
                                            duplicates_found += 1;
                                            bytes_saved += saved;
                                        }
                                    }
                                }
                                continue;
                            }

                            // Check if this package exists at the top level with compatible version
                            if let Some(saved) = self
                                .try_dedupe_package(&node_modules, &nested_path, &nested_name)
                                .await
                            {
                                duplicates_found += 1;
                                bytes_saved += saved;
                            }
                        }
                    }
                    to_check.push(nested_nm);
                }
            }
        }

        spinner.finish_and_clear();

        if duplicates_found == 0 {
            println!("\x1b[32m✓\x1b[0m No duplicates found. Dependencies are already optimized.");
        } else {
            println!(
                "\x1b[32m✓\x1b[0m Removed \x1b[1m{}\x1b[0m duplicate(s), saved \x1b[36m{:.2} MB\x1b[0m",
                duplicates_found,
                bytes_saved as f64 / 1024.0 / 1024.0
            );
        }

        // Rebuild lockfile
        self.save_lockfile(&package_json.name, &package_json.version)
            .await?;

        Ok(())
    }

    async fn try_dedupe_package(
        &self,
        root_nm: &PathBuf,
        nested_path: &PathBuf,
        pkg_name: &str,
    ) -> Option<u64> {
        // Get nested package version
        let nested_pkg_json = nested_path.join("package.json");
        let nested_content = fs::read_to_string(&nested_pkg_json).await.ok()?;
        let nested_pkg: PackageJson = serde_json::from_str(&nested_content).ok()?;
        let nested_version = &nested_pkg.version;

        // Check if same version exists at root
        let root_path = root_nm.join(pkg_name);
        if !root_path.exists() {
            return None;
        }

        let root_pkg_json = root_path.join("package.json");
        let root_content = fs::read_to_string(&root_pkg_json).await.ok()?;
        let root_pkg: PackageJson = serde_json::from_str(&root_content).ok()?;
        let root_version = &root_pkg.version;

        // If versions match, we can dedupe
        if nested_version == root_version {
            // Calculate size before removing
            let size = fs_extra::dir::get_size(nested_path).unwrap_or(0);

            // Remove the nested duplicate
            if fs::remove_dir_all(nested_path).await.is_ok() {
                let _ = self.multi_progress.println(format!(
                    "\x1b[33m-\x1b[0m \x1b[1m{}\x1b[0m@{} (duplicate)",
                    pkg_name, nested_version
                ));
                return Some(size);
            }
        }

        None
    }

    async fn get_latest_version(&self, name: &str) -> Option<(String, String)> {
        let current = self.get_installed_version(name).await?;
        let package = self.registry.get_package(name).await.ok()?;
        let latest = package.dist_tags.get("latest")?.clone();
        Some((current, latest))
    }

    async fn get_installed_version(&self, name: &str) -> Option<String> {
        let pkg_json_path = std::env::current_dir()
            .ok()?
            .join("node_modules")
            .join(name)
            .join("package.json");

        if let Ok(content) = fs::read_to_string(&pkg_json_path).await {
            if let Ok(pkg) = serde_json::from_str::<PackageJson>(&content) {
                return Some(pkg.version);
            }
        }
        None
    }

    pub async fn why_package(&self, name: &str) -> Result<()> {
        let package_json_content = fs::read_to_string("package.json")
            .await
            .context("Could not find package.json in current directory")?;
        let package_json: PackageJson = serde_json::from_str(&package_json_content)?;

        let mut found = false;
        let mut dependents: Vec<(String, String, bool)> = Vec::new(); // (name, version, is_dev)

        // Check if it's a direct dependency
        if let Some(version) = package_json.dependencies.get(name) {
            println!("\x1b[1m{}\x1b[0m@\x1b[90m{}\x1b[0m", name, version);
            println!(
                "  \x1b[32m├─\x1b[0m Direct dependency in \x1b[1m{}\x1b[0m",
                package_json.name
            );
            found = true;
        }

        // Check if it's a direct dev dependency
        if let Some(version) = package_json.dev_dependencies.get(name) {
            if !found {
                println!("\x1b[1m{}\x1b[0m@\x1b[90m{}\x1b[0m", name, version);
            }
            println!(
                "  \x1b[35m├─\x1b[0m Dev dependency in \x1b[1m{}\x1b[0m",
                package_json.name
            );
            found = true;
        }

        // Check transitive dependencies by scanning node_modules
        let node_modules = std::env::current_dir()?.join("node_modules");
        if node_modules.exists() {
            if let Ok(mut entries) = tokio::fs::read_dir(&node_modules).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let path = entry.path();
                    let pkg_name = entry.file_name().to_string_lossy().to_string();

                    // Skip hidden folders and the target package itself
                    if pkg_name.starts_with('.') || pkg_name == name {
                        continue;
                    }

                    // Handle scoped packages
                    if pkg_name.starts_with('@') {
                        if let Ok(mut scoped_entries) = tokio::fs::read_dir(&path).await {
                            while let Ok(Some(scoped_entry)) = scoped_entries.next_entry().await {
                                let scoped_path = scoped_entry.path();
                                let scoped_name = format!(
                                    "{}/{}",
                                    pkg_name,
                                    scoped_entry.file_name().to_string_lossy()
                                );

                                if let Some(dep_info) = self
                                    .check_package_depends_on(&scoped_path, &scoped_name, name)
                                    .await
                                {
                                    let is_dev =
                                        package_json.dev_dependencies.contains_key(&scoped_name);
                                    dependents.push((scoped_name, dep_info, is_dev));
                                }
                            }
                        }
                        continue;
                    }

                    if let Some(dep_info) =
                        self.check_package_depends_on(&path, &pkg_name, name).await
                    {
                        let is_dev = package_json.dev_dependencies.contains_key(&pkg_name);
                        dependents.push((pkg_name, dep_info, is_dev));
                    }
                }
            }
        }

        if !dependents.is_empty() {
            if !found {
                let installed_version = self
                    .get_installed_version(name)
                    .await
                    .unwrap_or_else(|| "?".to_string());
                println!(
                    "\x1b[1m{}\x1b[0m@\x1b[90m{}\x1b[0m",
                    name, installed_version
                );
            }
            println!("\n\x1b[1;36mRequired by:\x1b[0m");
            for (dep_name, version_req, is_dev) in &dependents {
                let marker = if *is_dev { "\x1b[35m" } else { "\x1b[32m" };
                println!(
                    "  {}├─\x1b[0m \x1b[1m{}\x1b[0m requires \x1b[90m{}\x1b[0m",
                    marker, dep_name, version_req
                );
            }
            found = true;
        }

        if !found {
            println!(
                "\x1b[33mPackage '{}' is not installed or not a dependency\x1b[0m",
                name
            );
        }

        Ok(())
    }

    async fn check_package_depends_on(
        &self,
        pkg_path: &std::path::Path,
        _pkg_name: &str,
        target: &str,
    ) -> Option<String> {
        let pkg_json_path = pkg_path.join("package.json");
        if let Ok(content) = fs::read_to_string(&pkg_json_path).await {
            if let Ok(pkg) = serde_json::from_str::<PackageJson>(&content) {
                if let Some(version) = pkg.dependencies.get(target) {
                    return Some(format!("{}@{}", target, version));
                }
            }
        }
        None
    }

    pub async fn handle_cache_command(&self, command: crate::CacheCommands) -> Result<()> {
        match command {
            crate::CacheCommands::Clean => {
                if self.installer.cache_dir.exists() {
                    fs::remove_dir_all(&self.installer.cache_dir).await?;
                    println!("\x1b[32mCache cleared\x1b[0m");
                } else {
                    println!("\x1b[90mCache is already empty\x1b[0m");
                }
            }
            crate::CacheCommands::Info => {
                let path = &self.installer.cache_dir;
                println!("\x1b[1mLocation:\x1b[0m  {}", path.display());

                if path.exists() {
                    let size = fs_extra::dir::get_size(path).unwrap_or(0);
                    println!(
                        "\x1b[1mSize:\x1b[0m      \x1b[36m{:.2} MB\x1b[0m",
                        size as f64 / 1024.0 / 1024.0
                    );

                    let count = std::fs::read_dir(path)?.count();
                    println!("\x1b[1mPackages:\x1b[0m  \x1b[36m{}\x1b[0m", count);
                } else {
                    println!("\x1b[1mSize:\x1b[0m      \x1b[90m0 MB\x1b[0m");
                    println!("\x1b[1mPackages:\x1b[0m  \x1b[90m0\x1b[0m");
                }
            }
        }
        Ok(())
    }

    pub async fn add_packages(&self, packages: Vec<String>, dev: bool) -> Result<()> {
        self.load_lockfile().await?;
        let package_json_content = fs::read_to_string("package.json").await?;
        let mut package_json: PackageJson = serde_json::from_str(&package_json_content)?;

        let spinner = self.create_spinner();
        let mut added_packages: Vec<(String, String)> = Vec::new();

        for pkg_input in packages {
            let (name, range) = if let Some(idx) = pkg_input.rfind('@') {
                if idx == 0 {
                    (pkg_input.as_str(), "latest")
                } else {
                    (&pkg_input[..idx], &pkg_input[idx + 1..])
                }
            } else {
                (pkg_input.as_str(), "latest")
            };

            spinner.set_message(format!("\x1b[1mResolving\x1b[0m {}...", name));
            let package = self
                .registry
                .get_package(name)
                .await
                .with_context(|| format!("Failed to fetch metadata for {}", name))?;
            let resolved = self
                .registry
                .resolve_version(&package, range)
                .with_context(|| format!("Failed to resolve version for {}", name))?;

            if dev {
                package_json
                    .dev_dependencies
                    .insert(name.to_string(), format!("^{}", resolved.version));
            } else {
                package_json
                    .dependencies
                    .insert(name.to_string(), format!("^{}", resolved.version));
            }
            added_packages.push((name.to_string(), resolved.version.clone()));
        }
        spinner.finish_and_clear();

        // Print added packages
        for (name, version) in &added_packages {
            println!(
                "\x1b[32m+\x1b[0m \x1b[1m{}\x1b[0m@\x1b[90m{}\x1b[0m",
                name, version
            );
        }

        let new_content = serde_json::to_string_pretty(&package_json)?;
        fs::write("package.json", new_content).await?;

        // Reset and setup progress tracking for dependencies
        self.reset_progress();
        let pb = self.create_install_progress();
        pb.set_message("\x1b[1mInstalling\x1b[0m dependencies...");
        *self.progress_bar.lock().await = Some(pb.clone());

        self.install_deps(&package_json).await?;

        let installed = self.packages_installed.load(Ordering::Relaxed);
        let cached = self.packages_cached.load(Ordering::Relaxed);

        pb.finish_and_clear();
        *self.progress_bar.lock().await = None;

        // Print summary
        if installed > 0 || cached > 0 {
            let mut parts = Vec::new();
            if installed > 0 {
                parts.push(format!("\x1b[32m+{}\x1b[0m installed", installed));
            }
            if cached > 0 {
                parts.push(format!("\x1b[33m{}\x1b[0m cached", cached));
            }
            println!("{}", parts.join("  \x1b[90m│\x1b[0m  "));
        }

        self.run_postinstalls().await?;
        self.save_lockfile(&package_json.name, &package_json.version)
            .await?;
        Ok(())
    }

    pub async fn remove_packages(&self, packages: Vec<String>) -> Result<()> {
        self.load_lockfile().await?;
        let package_json_content = fs::read_to_string("package.json").await?;
        let mut package_json: PackageJson = serde_json::from_str(&package_json_content)?;

        let mut removed_any = false;

        for name in &packages {
            let was_dep = package_json.dependencies.remove(name).is_some();
            let was_dev_dep = package_json.dev_dependencies.remove(name).is_some();

            if was_dep || was_dev_dep {
                removed_any = true;
                println!("\x1b[31m-\x1b[0m \x1b[1m{}\x1b[0m", name);

                // Remove from node_modules
                let pkg_path = PathBuf::from("node_modules").join(name);
                if pkg_path.exists() {
                    fs::remove_dir_all(&pkg_path).await?;
                }

                // Remove from lockfile
                {
                    let mut lock = self.lockfile.lock().await;
                    let key = format!("node_modules/{}", name);
                    lock.packages.remove(&key);
                }

                // Remove binary links
                let bin_dir = PathBuf::from("node_modules").join(".bin");
                if bin_dir.exists() {
                    if let Ok(mut entries) = fs::read_dir(&bin_dir).await {
                        while let Ok(Some(entry)) = entries.next_entry().await {
                            let path = entry.path();
                            // Check if this symlink points to the removed package
                            if let Ok(target) = fs::read_link(&path).await {
                                if target.to_string_lossy().contains(&format!("/{}/", name))
                                    || target.to_string_lossy().contains(&format!("\\{}\\", name))
                                {
                                    let _ = fs::remove_file(&path).await;
                                    // Also remove .cmd and .ps1 on Windows
                                    let _ = fs::remove_file(path.with_extension("cmd")).await;
                                    let _ = fs::remove_file(path.with_extension("ps1")).await;
                                }
                            }
                        }
                    }
                }
            } else {
                println!(
                    "\x1b[33mwarn:\x1b[0m \x1b[1m{}\x1b[0m is not installed",
                    name
                );
            }
        }

        if removed_any {
            // Save updated package.json
            let new_content = serde_json::to_string_pretty(&package_json)?;
            fs::write("package.json", new_content).await?;

            // Save updated lockfile
            self.save_lockfile(&package_json.name, &package_json.version)
                .await?;
        }

        Ok(())
    }

    pub async fn exec_package(&self, package: &str, args: Vec<String>) -> Result<()> {
        // Parse package name and version
        let (name, version_range) = if let Some(idx) = package.rfind('@') {
            if idx == 0 {
                (package, "latest")
            } else {
                (&package[..idx], &package[idx + 1..])
            }
        } else {
            (package, "latest")
        };

        // Extract the binary name (last part of scoped package or package name)
        let bin_name = if name.starts_with('@') {
            name.split('/').last().unwrap_or(name)
        } else {
            name
        };

        // First, check if binary exists locally in node_modules/.bin
        let local_bin = PathBuf::from("node_modules").join(".bin").join(bin_name);
        if local_bin.exists() {
            println!("\x1b[90mUsing local\x1b[0m \x1b[1m{}\x1b[0m\n", bin_name);
            return self.run_binary(&local_bin, args).await;
        }

        // Not found locally, need to fetch and run
        let spinner = self.create_spinner();
        spinner.set_message(format!("\x1b[1mFetching\x1b[0m {}...", name));

        // Fetch package metadata
        let pkg = self
            .registry
            .get_package(name)
            .await
            .with_context(|| format!("Failed to fetch package {}", name))?;
        let resolved = self
            .registry
            .resolve_version(&pkg, version_range)
            .with_context(|| format!("Failed to resolve version for {}", name))?;

        spinner.set_message(format!(
            "\x1b[1mInstalling\x1b[0m {}@{}...",
            name, resolved.version
        ));

        // Install to a temporary location within the cache
        let temp_dir = self.installer.cache_dir.join("_npx").join(format!(
            "{}@{}",
            name.replace('/', "+"),
            resolved.version
        ));

        // Install the main package
        self.installer
            .install_package(name, &resolved.version, &resolved.dist.tarball, &temp_dir)
            .await?;

        // Install dependencies recursively
        spinner.set_message(format!(
            "\x1b[1mInstalling\x1b[0m dependencies for {}...",
            name
        ));

        // Collect regular dependencies
        let mut to_install: Vec<(String, String, bool)> = resolved
            .dependencies
            .iter()
            .map(|(k, v)| (k.clone(), v.clone(), false)) // false = not optional
            .collect();

        // Collect optional dependencies (platform-specific binaries)
        for (k, v) in &resolved.optional_dependencies {
            to_install.push((k.clone(), v.clone(), true)); // true = optional
        }

        while let Some((dep_name, dep_version, is_optional)) = to_install.pop() {
            let dep_install_path = temp_dir.join("node_modules").join(&dep_name);
            if dep_install_path.exists() {
                continue;
            }

            if let Ok(dep_pkg) = self.registry.get_package(&dep_name).await {
                if let Ok(dep_resolved) = self.registry.resolve_version(&dep_pkg, &dep_version) {
                    // For optional dependencies, check platform compatibility
                    if is_optional && !is_version_platform_compatible(dep_resolved) {
                        continue; // Skip platform-incompatible optional deps
                    }

                    let _ = self
                        .installer
                        .install_package(
                            &dep_name,
                            &dep_resolved.version,
                            &dep_resolved.dist.tarball,
                            &temp_dir,
                        )
                        .await;

                    // Add transitive dependencies (not optional)
                    for (k, v) in &dep_resolved.dependencies {
                        let nested_path = temp_dir.join("node_modules").join(k);
                        if !nested_path.exists() {
                            to_install.push((k.clone(), v.clone(), false));
                        }
                    }

                    // Add transitive optional dependencies
                    for (k, v) in &dep_resolved.optional_dependencies {
                        let nested_path = temp_dir.join("node_modules").join(k);
                        if !nested_path.exists() {
                            to_install.push((k.clone(), v.clone(), true));
                        }
                    }
                }
            }
        }

        spinner.finish_and_clear();

        // Find the binary
        let bin_path = if let Some(bin) = &resolved.bin {
            match bin {
                serde_json::Value::String(s) => temp_dir.join("node_modules").join(name).join(s),
                serde_json::Value::Object(o) => {
                    if let Some(serde_json::Value::String(s)) = o.get(bin_name) {
                        temp_dir.join("node_modules").join(name).join(s)
                    } else if let Some((_, serde_json::Value::String(s))) = o.iter().next() {
                        temp_dir.join("node_modules").join(name).join(s)
                    } else {
                        anyhow::bail!("No binary found in package {}", name);
                    }
                }
                _ => anyhow::bail!("No binary found in package {}", name),
            }
        } else {
            anyhow::bail!("Package {} does not have a binary", name);
        };

        if !bin_path.exists() {
            anyhow::bail!("Binary not found at {}", bin_path.display());
        }

        println!(
            "\x1b[90mExecuting\x1b[0m \x1b[1m{}@{}\x1b[0m\n",
            name, resolved.version
        );

        self.run_binary(&bin_path, args).await
    }

    async fn run_binary(&self, bin_path: &PathBuf, args: Vec<String>) -> Result<()> {
        let current_dir = std::env::current_dir()?;
        let local_bin_path = current_dir.join("node_modules").join(".bin");
        let path_env = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", local_bin_path.display(), path_env);

        let status = Command::new("node")
            .arg(bin_path)
            .args(&args)
            .env("PATH", &new_path)
            .status()
            .await?;

        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }

        Ok(())
    }

    pub async fn run_script(&self, script_name: &str, args: Vec<String>) -> Result<()> {
        let package_json_content = fs::read_to_string("package.json").await?;
        let package_json: PackageJson = serde_json::from_str(&package_json_content)?;

        let script = match package_json.scripts.get(script_name) {
            Some(s) => s,
            None => {
                let available: Vec<String> = package_json.scripts.keys().cloned().collect();
                return Err(RpmError::ScriptNotFound {
                    script: script_name.to_string(),
                    available,
                }
                .into());
            }
        };

        println!(
            "{}${} {}{}{}\n",
            colors::GRAY,
            colors::RESET,
            colors::BOLD,
            script,
            colors::RESET
        );

        // Build the full command with args
        let full_command = if args.is_empty() {
            script.clone()
        } else {
            format!("{} {}", script, args.join(" "))
        };

        // Add node_modules/.bin to PATH
        let current_dir = std::env::current_dir()?;
        let bin_path = current_dir.join("node_modules").join(".bin");
        let path_env = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", bin_path.display(), path_env);

        let status = Command::new("sh")
            .arg("-c")
            .arg(&full_command)
            .env("PATH", &new_path)
            .status()
            .await?;

        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }

        Ok(())
    }

    /// Run a script across all workspaces that have it defined (in parallel)
    pub async fn run_script_workspaces(
        &self,
        script_name: &str,
        args: Vec<String>,
        filter: Option<&str>,
    ) -> Result<()> {
        let root = std::env::current_dir()?;
        let workspace = Workspace::discover(&root)
            .await?
            .context("Not in a workspace. Use 'rpm run' without --workspaces flag.")?;

        // Find all workspaces with this script
        let scripts = workspace.get_scripts(script_name);

        if scripts.is_empty() {
            println!(
                "\x1b[33mNo workspaces have script '{}'\x1b[0m",
                script_name
            );
            return Ok(());
        }

        // Filter workspaces if specified
        let scripts_to_run: Vec<_> = if let Some(filter_pattern) = filter {
            scripts
                .into_iter()
                .filter(|(m, _)| {
                    m.name.contains(filter_pattern) || m.name == filter_pattern
                })
                .collect()
        } else {
            scripts
        };

        if scripts_to_run.is_empty() {
            println!(
                "\x1b[33mNo matching workspaces have script '{}'\x1b[0m",
                script_name
            );
            return Ok(());
        }

        println!(
            "\x1b[1;36mRunning '{}' in {} workspace(s) (parallel)\x1b[0m\n",
            script_name,
            scripts_to_run.len()
        );

        let root_bin_path = workspace.root.join("node_modules").join(".bin");
        let path_env = std::env::var("PATH").unwrap_or_default();
        let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let multi_progress = &self.multi_progress;

        // Execute all scripts in parallel
        let mut tasks = FuturesUnordered::new();
        
        for (member, script) in scripts_to_run {
            let root_bin_path = root_bin_path.clone();
            let path_env = path_env.clone();
            let args = args.clone();
            let failed = failed.clone();
            let workspace_root = workspace.root.clone();
            
            tasks.push(async move {
                let relative_path = member
                    .path
                    .strip_prefix(&workspace_root)
                    .unwrap_or(&member.path);

                // Build the full command with args
                let full_command = if args.is_empty() {
                    script.clone()
                } else {
                    format!("{} {}", script, args.join(" "))
                };

                // Add both workspace's node_modules/.bin and root node_modules/.bin to PATH
                let local_bin_path = member.path.join("node_modules").join(".bin");
                let new_path = format!(
                    "{}:{}:{}",
                    local_bin_path.display(),
                    root_bin_path.display(),
                    path_env
                );

                let status = Command::new("sh")
                    .arg("-c")
                    .arg(&full_command)
                    .current_dir(&member.path)
                    .env("PATH", &new_path)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output()
                    .await;

                let (success, output, stderr) = match status {
                    Ok(output) => (
                        output.status.success(),
                        String::from_utf8_lossy(&output.stdout).to_string(),
                        String::from_utf8_lossy(&output.stderr).to_string(),
                    ),
                    Err(e) => (false, String::new(), e.to_string()),
                };

                if !success {
                    failed.store(true, Ordering::Relaxed);
                }

                (member.name.clone(), relative_path.to_path_buf(), script, success, output, stderr)
            });
        }

        // Collect results and print them as they complete
        while let Some((name, relative_path, script, success, output, stderr)) = tasks.next().await {
            let _ = multi_progress.println(format!(
                "\x1b[1;36m{}\x1b[0m \x1b[90m({})\x1b[0m",
                name,
                relative_path.display()
            ));
            let _ = multi_progress.println(format!("\x1b[90m$\x1b[0m {}", script));
            
            if !output.is_empty() {
                for line in output.lines() {
                    let _ = multi_progress.println(format!("  {}", line));
                }
            }
            if !stderr.is_empty() {
                for line in stderr.lines() {
                    let _ = multi_progress.println(format!("  \x1b[90m{}\x1b[0m", line));
                }
            }

            if !success {
                let _ = multi_progress.println(format!(
                    "\x1b[31m✗\x1b[0m \x1b[1m{}\x1b[0m failed\n",
                    name
                ));
            } else {
                let _ = multi_progress.println(format!("\x1b[32m✓\x1b[0m \x1b[1m{}\x1b[0m completed\n", name));
            }
        }

        if failed.load(Ordering::Relaxed) {
            std::process::exit(1);
        }

        Ok(())
    }

    /// List all workspaces
    pub async fn list_workspaces(&self) -> Result<()> {
        let root = std::env::current_dir()?;
        let workspace = Workspace::discover(&root)
            .await?
            .context("Not in a workspace root")?;

        workspace.print_info();
        Ok(())
    }

    /// Check if a package in node_modules matches what's expected in the lockfile
    async fn is_package_up_to_date(&self, name: &str, expected_version: &str) -> bool {
        let pkg_json_path = std::env::current_dir()
            .ok()
            .map(|p| p.join("node_modules").join(name).join("package.json"));
        
        if let Some(path) = pkg_json_path {
            if let Ok(content) = fs::read_to_string(&path).await {
                if let Ok(pkg) = serde_json::from_str::<PackageJson>(&content) {
                    return pkg.version == expected_version;
                }
            }
        }
        false
    }

    /// Compute which packages need to be installed (incremental install optimization)
    async fn compute_packages_to_install(&self, package_json: &PackageJson) -> Vec<(String, String)> {
        let lockfile = self.lockfile.lock().await;
        
        // Collect all declared dependencies
        let all_deps: Vec<(String, String)> = package_json
            .dependencies
            .iter()
            .chain(package_json.dev_dependencies.iter())
            .map(|(name, version)| (name.clone(), version.clone()))
            .collect();
        
        drop(lockfile);
        
        // Check which packages are already up-to-date in node_modules
        let mut packages_to_install = Vec::new();
        let mut up_to_date_count = 0;
        
        for (name, version_range) in all_deps {
            // Get expected version from lockfile
            let expected_version = {
                let lock = self.lockfile.lock().await;
                let key = format!("node_modules/{}", name);
                lock.packages.get(&key).map(|e| e.version.clone())
            };
            
            if let Some(expected) = expected_version {
                // Check if the installed version matches the lockfile
                if self.is_package_up_to_date(&name, &expected).await {
                    up_to_date_count += 1;
                    // Mark as already processed to skip in resolve_and_install
                    self.installed.insert(name.clone(), expected);
                    continue;
                }
            }
            
            packages_to_install.push((name, version_range));
        }
        
        if up_to_date_count > 0 {
            let _ = self.multi_progress.println(format!(
                "\x1b[90m{} packages already up-to-date\x1b[0m",
                up_to_date_count
            ));
        }
        
        packages_to_install
    }

    pub async fn install(&self) -> Result<()> {
        let root = std::env::current_dir()?;
        
        // Check if this is a workspace
        if let Some(workspace) = Workspace::discover(&root).await? {
            return self.install_workspace(&workspace).await;
        }

        self.load_lockfile().await?;
        let package_json_content = fs::read_to_string("package.json").await?;
        let package_json: PackageJson = serde_json::from_str(&package_json_content)?;

        // Reset and setup progress tracking
        self.reset_progress();
        let pb = self.create_install_progress();
        pb.set_message("\x1b[1mChecking\x1b[0m installed packages...");
        *self.progress_bar.lock().await = Some(pb.clone());

        // Incremental install: compute which packages actually need to be installed
        let packages_to_install = self.compute_packages_to_install(&package_json).await;
        
        if packages_to_install.is_empty() {
            pb.finish_and_clear();
            *self.progress_bar.lock().await = None;
            println!("\x1b[32m✓\x1b[0m All packages up-to-date");
            return Ok(());
        }

        pb.set_message(format!(
            "\x1b[1mInstalling\x1b[0m {} package(s)...",
            packages_to_install.len()
        ));

        // Install only packages that need updating
        self.install_deps_incremental(&package_json, packages_to_install).await?;

        let installed = self.packages_installed.load(Ordering::Relaxed);
        let cached = self.packages_cached.load(Ordering::Relaxed);

        pb.finish_and_clear();
        *self.progress_bar.lock().await = None;

        // Print summary
        if installed > 0 || cached > 0 {
            let mut parts = Vec::new();
            if installed > 0 {
                parts.push(format!("\x1b[32m+{}\x1b[0m installed", installed));
            }
            if cached > 0 {
                parts.push(format!("\x1b[33m{}\x1b[0m cached", cached));
            }
            println!("{}", parts.join("  \x1b[90m│\x1b[0m  "));
        } else {
            println!("\x1b[90mNo packages to install\x1b[0m");
        }

        self.run_postinstalls().await?;
        self.save_lockfile(&package_json.name, &package_json.version)
            .await?;

        Ok(())
    }

    /// Install dependencies for a workspace (monorepo)
    async fn install_workspace(&self, workspace: &Workspace) -> Result<()> {
        workspace.print_info();
        println!();

        self.load_lockfile().await?;

        // Reset and setup progress tracking
        self.reset_progress();
        let pb = self.create_install_progress();
        pb.set_message("\x1b[1mResolving\x1b[0m workspace dependencies...");
        *self.progress_bar.lock().await = Some(pb.clone());

        // Get hoisted dependencies (installed at root)
        let hoisted = workspace.get_hoisted_dependencies();
        let workspace_packages = workspace.get_workspace_package_names();

        // Install hoisted dependencies at root
        let mut tasks = FuturesUnordered::new();
        for (name, version) in &hoisted {
            let root = workspace.root.clone();
            let manager = self.clone();
            let name = name.clone();
            let version = version.clone();
            tasks.push(async move { manager.resolve_and_install(name, version, root).await });
        }

        while let Some(result) = tasks.next().await {
            if let Err(e) = result {
                let _ = self
                    .multi_progress
                    .println(format!("\x1b[31merror:\x1b[0m {}", e));
            }
        }

        // Create symlinks for workspace packages in root node_modules
        let root_node_modules = workspace.root.join("node_modules");
        fs::create_dir_all(&root_node_modules).await?;

        for member in &workspace.members {
            let link_path = root_node_modules.join(&member.name);
            
            // Handle scoped packages (@scope/name)
            if member.name.contains('/') {
                if let Some(scope) = member.name.split('/').next() {
                    fs::create_dir_all(root_node_modules.join(scope)).await?;
                }
            }

            // Remove existing link/dir
            if link_path.exists() || link_path.is_symlink() {
                let _ = fs::remove_file(&link_path).await;
                let _ = fs::remove_dir_all(&link_path).await;
            }

            // Create symlink to workspace member
            #[cfg(unix)]
            {
                let relative = pathdiff::diff_paths(&member.path, &root_node_modules)
                    .unwrap_or_else(|| member.path.clone());
                let _ = fs::symlink(&relative, &link_path).await;
            }

            #[cfg(windows)]
            {
                let _ = tokio::fs::symlink_dir(&member.path, &link_path).await;
            }
        }

        // Link binaries from workspace packages
        for member in &workspace.members {
            if let Some(bin) = &member.package_json.bin {
                let _ = self.link_binaries(&workspace.root, &member.name, bin).await;
            }
        }

        let installed = self.packages_installed.load(Ordering::Relaxed);
        let cached = self.packages_cached.load(Ordering::Relaxed);

        pb.finish_and_clear();
        *self.progress_bar.lock().await = None;

        // Print summary
        println!();
        if installed > 0 || cached > 0 {
            let mut parts = Vec::new();
            if installed > 0 {
                parts.push(format!("\x1b[32m+{}\x1b[0m installed", installed));
            }
            if cached > 0 {
                parts.push(format!("\x1b[33m{}\x1b[0m cached", cached));
            }
            parts.push(format!(
                "\x1b[36m{}\x1b[0m linked",
                workspace_packages.len()
            ));
            println!("{}", parts.join("  \x1b[90m│\x1b[0m  "));
        } else if !workspace_packages.is_empty() {
            println!(
                "\x1b[36m{}\x1b[0m workspace packages linked",
                workspace_packages.len()
            );
        } else {
            println!("\x1b[90mNo packages to install\x1b[0m");
        }

        self.run_postinstalls().await?;
        self.save_lockfile(&workspace.root_package.name, &workspace.root_package.version)
            .await?;

        Ok(())
    }

    async fn install_deps(&self, package_json: &PackageJson) -> Result<()> {
        let root = std::env::current_dir()?;
        
        // Collect all dependencies (regular + dev)
        let all_deps: Vec<(String, String)> = package_json
            .dependencies
            .iter()
            .chain(package_json.dev_dependencies.iter())
            .map(|(name, version)| (name.clone(), version.clone()))
            .collect();

        // Lazy resolution optimization: identify which packages need registry fetch
        // vs which can be resolved entirely from lockfile
        let lockfile = self.lockfile.lock().await;
        let mut needs_fetch: Vec<(String, String)> = Vec::new();
        let mut from_lockfile: Vec<(String, String)> = Vec::new();
        
        for (name, version_range) in &all_deps {
            let key = format!("node_modules/{}", name);
            if let Some(entry) = lockfile.packages.get(&key) {
                let matches = semver::Version::parse(&entry.version)
                    .ok()
                    .and_then(|v| {
                        semver::VersionReq::parse(version_range)
                            .ok()
                            .map(|r| r.matches(&v))
                    })
                    .unwrap_or(false);
                
                if matches || version_range == &entry.version {
                    from_lockfile.push((name.clone(), version_range.clone()));
                } else {
                    needs_fetch.push((name.clone(), version_range.clone()));
                }
            } else {
                needs_fetch.push((name.clone(), version_range.clone()));
            }
        }
        drop(lockfile);

        // Combine all deps (lockfile-resolvable first for lazy optimization)
        let ordered_deps: Vec<(String, String)> = from_lockfile
            .into_iter()
            .chain(needs_fetch.into_iter())
            .collect();

        let mut tasks = FuturesUnordered::new();
        for (name, version) in ordered_deps {
            let root = root.clone();
            let manager = self.clone();
            tasks.push(async move { manager.resolve_and_install(name, version, root).await });
        }

        while let Some(result) = tasks.next().await {
            if let Err(e) = result {
                let _ = self
                    .multi_progress
                    .println(format!("\x1b[31merror:\x1b[0m {}", e));
            }
        }
        Ok(())
    }

    /// Install only the specified packages (incremental install)
    async fn install_deps_incremental(
        &self,
        _package_json: &PackageJson,
        packages_to_install: Vec<(String, String)>,
    ) -> Result<()> {
        let root = std::env::current_dir()?;

        // Lazy resolution: identify which packages need registry fetch
        let lockfile = self.lockfile.lock().await;
        let mut needs_fetch: Vec<(String, String)> = Vec::new();
        let mut from_lockfile: Vec<(String, String)> = Vec::new();
        
        for (name, version_range) in &packages_to_install {
            let key = format!("node_modules/{}", name);
            if let Some(entry) = lockfile.packages.get(&key) {
                let matches = semver::Version::parse(&entry.version)
                    .ok()
                    .and_then(|v| {
                        semver::VersionReq::parse(version_range)
                            .ok()
                            .map(|r| r.matches(&v))
                    })
                    .unwrap_or(false);
                
                if matches || version_range == &entry.version {
                    from_lockfile.push((name.clone(), version_range.clone()));
                } else {
                    needs_fetch.push((name.clone(), version_range.clone()));
                }
            } else {
                needs_fetch.push((name.clone(), version_range.clone()));
            }
        }
        drop(lockfile);

        // Process lockfile-resolvable packages first, then those needing fetch
        let ordered_deps: Vec<(String, String)> = from_lockfile
            .into_iter()
            .chain(needs_fetch.into_iter())
            .collect();

        let mut tasks = FuturesUnordered::new();
        for (name, version) in ordered_deps {
            let root = root.clone();
            let manager = self.clone();
            tasks.push(async move { manager.resolve_and_install(name, version, root).await });
        }

        while let Some(result) = tasks.next().await {
            if let Err(e) = result {
                let _ = self
                    .multi_progress
                    .println(format!("\x1b[31merror:\x1b[0m {}", e));
            }
        }
        Ok(())
    }

    async fn run_postinstalls(&self) -> Result<()> {
        if self.postinstalls.is_empty() || self.ignore_scripts {
            return Ok(());
        }

        let scripts_to_run: Vec<_> = if !self.auto_confirm {
            println!("\n\x1b[1;33mPending postinstall scripts:\x1b[0m");
            for entry in self.postinstalls.iter() {
                println!(
                    "  \x1b[90m-\x1b[0m \x1b[36m{}\x1b[0m \x1b[90m{}\x1b[0m",
                    entry.key(),
                    entry.value().1
                );
            }

            println!("\n\x1b[1mRun these scripts?\x1b[0m \x1b[90m[y/N]\x1b[0m");

            let mut stdin = BufReader::new(tokio::io::stdin());
            let mut line = String::new();
            stdin.read_line(&mut line).await?;

            if line.trim().eq_ignore_ascii_case("y") {
                self.postinstalls
                    .iter()
                    .map(|e| (e.key().clone(), e.value().clone()))
                    .collect()
            } else {
                println!("\x1b[90mSkipped postinstall scripts\x1b[0m");
                return Ok(());
            }
        } else {
            self.postinstalls
                .iter()
                .map(|e| (e.key().clone(), e.value().clone()))
                .collect()
        };

        if scripts_to_run.is_empty() {
            return Ok(());
        }

        let total = scripts_to_run.len();
        let completed = Arc::new(AtomicUsize::new(0));
        
        let pb = self
            .multi_progress
            .add(ProgressBar::new(total as u64));
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.cyan} [{bar:40.cyan/blue}] {pos}/{len} \x1b[1mRunning\x1b[0m postinstall scripts (parallel)...")
            .unwrap()
            .progress_chars("━╸─")
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"));

        // Execute postinstall scripts in parallel
        let mut tasks = FuturesUnordered::new();
        
        // Limit concurrent postinstall scripts to avoid overwhelming the system
        let postinstall_semaphore = Arc::new(Semaphore::new(10));
        
        for (name, (path, script)) in scripts_to_run {
            let completed = completed.clone();
            let postinstall_semaphore = postinstall_semaphore.clone();
            
            tasks.push(async move {
                let _permit = postinstall_semaphore.acquire().await;
                
                let status = Command::new("sh")
                    .arg("-c")
                    .arg(&script)
                    .current_dir(&path)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await;

                // Fallback to cmd on Windows if sh fails
                let success = match status {
                    Ok(s) => s.success(),
                    Err(_) if cfg!(windows) => {
                        Command::new("cmd")
                            .arg("/C")
                            .arg(&script)
                            .current_dir(&path)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .status()
                            .await
                            .map(|s| s.success())
                            .unwrap_or(false)
                    }
                    Err(_) => false,
                };

                completed.fetch_add(1, Ordering::Relaxed);
                (name, success)
            });
        }

        // Process results as they complete
        let mut failed_scripts = Vec::new();
        while let Some((name, success)) = tasks.next().await {
            pb.inc(1);
            if !success {
                failed_scripts.push(name);
            }
        }

        pb.finish_and_clear();
        
        // Report any failures
        if !failed_scripts.is_empty() {
            for name in &failed_scripts {
                let _ = self.multi_progress.println(format!(
                    "\x1b[33mwarn:\x1b[0m postinstall script for \x1b[1m{}\x1b[0m failed",
                    name
                ));
            }
        }
        
        Ok(())
    }

    async fn link_binaries(
        &self,
        target_dir: &PathBuf,
        package_name: &str,
        bin: &serde_json::Value,
    ) -> Result<()> {
        let bin_dir = target_dir.join("node_modules").join(".bin");
        fs::create_dir_all(&bin_dir).await?;

        let bins: BTreeMap<String, String> = match bin {
            serde_json::Value::String(s) => {
                let mut map = BTreeMap::new();
                map.insert(package_name.to_string(), s.clone());
                map
            }
            serde_json::Value::Object(o) => {
                let mut map = BTreeMap::new();
                for (k, v) in o {
                    if let Some(s) = v.as_str() {
                        map.insert(k.clone(), s.to_string());
                    }
                }
                map
            }
            _ => BTreeMap::new(),
        };

        for (name, path) in bins {
            let target_path = target_dir
                .join("node_modules")
                .join(package_name)
                .join(&path);
            let link_path = bin_dir.join(&name);

            #[cfg(unix)]
            {
                // Relative path from .bin to target file
                // .bin/tool -> ../package/cli.js
                let relative = PathBuf::from("..").join(package_name).join(&path);
                if link_path.exists() || link_path.is_symlink() {
                    let _ = fs::remove_file(&link_path).await;
                }

                // This needs to be spawned blocking if using std::fs, but we can use tokio::fs::symlink
                let _ = fs::symlink(&relative, &link_path).await;

                // Make executable
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = fs::metadata(&target_path).await {
                    let mut perms = metadata.permissions();
                    perms.set_mode(0o755);
                    let _ = fs::set_permissions(&target_path, perms).await;
                }
            }

            #[cfg(windows)]
            {
                let cmd_content = format!(
                    "@ECHO off\r\n\"%~dp0\\..\\{}\\{}\" %*\r\n",
                    package_name,
                    path.replace("/", "\\")
                );
                fs::write(link_path.with_extension("cmd"), cmd_content).await?;

                // Optional: Powershell shim
                let ps1_content = format!(
                    "& \"$PSScriptRoot\\..\\{}\\{}\" $args\r\n",
                    package_name,
                    path.replace("/", "\\")
                );
                fs::write(link_path.with_extension("ps1"), ps1_content).await?;
            }
        }
        Ok(())
    }

    #[async_recursion::async_recursion]
    async fn resolve_and_install(
        &self,
        name: String,
        version_range: String,
        target_dir: PathBuf,
    ) -> Result<()> {
        if self.installed.contains_key(&name) {
            return Ok(());
        }

        // Track current package being resolved
        self.set_current_package(&name, "resolving");

        // Lazy resolution: First check lockfile, then check if already installed on disk
        let lock_entry = {
            let lock = self.lockfile.lock().await;
            let key = format!("node_modules/{}", name);
            lock.packages.get(&key).cloned()
        };

        let (version, tarball, deps, peer_deps, optional_deps, postinstall, bin) =
            if let Some(entry) = lock_entry {
                // Check if lockfile version satisfies the requested range
                let matches = semver::Version::parse(&entry.version)
                    .ok()
                    .and_then(|v| {
                        semver::VersionReq::parse(&version_range)
                            .ok()
                            .map(|r| r.matches(&v))
                    })
                    .unwrap_or(false);

                if matches || version_range == entry.version {
                    // Lockfile entry is valid - use it without any network request (lazy)
                    (
                        entry.version,
                        entry.resolved,
                        entry.dependencies,
                        entry.peer_dependencies,
                        entry.optional_dependencies,
                        entry.postinstall,
                        entry.bin,
                    )
                } else {
                    // Version mismatch - need to fetch from registry
                    self.fetch_and_resolve(&name, &version_range).await?
                }
            } else {
                // Not in lockfile - need to fetch from registry
                self.fetch_and_resolve(&name, &version_range).await?
            };

        // Track resolved packages
        self.packages_resolved.fetch_add(1, Ordering::Relaxed);
        self.clear_current_package(&name);
        self.update_progress();

        if self.installed.contains_key(&name) {
            return Ok(());
        }
        self.installed.insert(name.clone(), version.clone());

        let install_path = target_dir.join("node_modules").join(&name);
        let already_exists = install_path.join("package.json").exists();

        if !already_exists {
            // Track current package being installed
            self.set_current_package(&name, "installing");
            
            let install_res = async {
                let install_dir = std::env::current_dir().unwrap();
                self.installer
                    .install_package(&name, &version, &tarball, &install_dir)
                    .await
            }
            .await;

            self.clear_current_package(&name);

            match install_res {
                Ok(_) => {
                    // Track installed packages
                    self.packages_installed.fetch_add(1, Ordering::Relaxed);
                    self.update_progress();

                    // Collect postinstall if exists
                    if let Some(script) = &postinstall {
                        self.postinstalls
                            .insert(name.clone(), (install_path.clone(), script.clone()));
                    }
                }
                Err(e) => {
                    let _ = self.multi_progress.println(format!(
                        "{}✗{} {}{}{}@{} failed: {}",
                        colors::RED,
                        colors::RESET,
                        colors::BOLD,
                        name,
                        colors::RESET,
                        version,
                        e
                    ));
                    return Err(e);
                }
            }
        } else {
            // Package was cached/already existed
            self.packages_cached.fetch_add(1, Ordering::Relaxed);
            self.update_progress();
        }

        // Always try to link binaries if they exist
        if let Some(bin_val) = &bin {
            let _ = self.link_binaries(&target_dir, &name, bin_val).await;
        }

        {
            let mut lock = self.lockfile.lock().await;
            let key = format!("node_modules/{}", name);
            lock.packages.insert(
                key,
                LockPackage {
                    version: version.clone(),
                    resolved: tarball.clone(),
                    integrity: None,
                    dependencies: deps.clone(),
                    peer_dependencies: peer_deps.clone(),
                    optional_dependencies: optional_deps.clone(),
                    postinstall: postinstall.clone(),
                    bin: bin.clone(),
                },
            );
        }

        // Collect all dependencies to install
        let mut all_deps: Vec<(String, String)> = Vec::new();

        // Regular dependencies
        for (dep_name, dep_ver) in deps {
            all_deps.push((dep_name.clone(), dep_ver.clone()));
        }

        // Peer dependencies (auto-installed like npm 7+)
        for (dep_name, dep_ver) in peer_deps {
            if !self.installed.contains_key(&dep_name) {
                all_deps.push((dep_name.clone(), dep_ver.clone()));
            }
        }

        // Optional dependencies
        let optional_deps_list: Vec<(String, String)> = optional_deps
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Install regular and peer dependencies
        let mut tasks = FuturesUnordered::new();
        for (dep_name, dep_ver) in all_deps {
            let target_dir = target_dir.clone();
            let manager = self.clone();
            tasks.push(async move {
                manager
                    .resolve_and_install(dep_name, dep_ver, target_dir)
                    .await
            });
        }

        while let Some(result) = tasks.next().await {
            if let Err(e) = result {
                let _ = self
                    .multi_progress
                    .println(format!("\x1b[33mwarn:\x1b[0m {} - {}", name, e));
            }
        }

        // Install optional dependencies (with platform checking, failures are silently ignored)
        for (dep_name, dep_ver) in optional_deps_list {
            // Skip if already installed
            if self.installed.contains_key(&dep_name) {
                continue;
            }

            // Check platform compatibility before attempting to install
            match self
                .check_optional_dep_compatible(&dep_name, &dep_ver)
                .await
            {
                Ok(true) => {
                    let target_dir = target_dir.clone();
                    let _ = self
                        .resolve_and_install(dep_name, dep_ver, target_dir)
                        .await;
                }
                Ok(false) => {
                    // Package is not compatible with current platform, skip silently
                }
                Err(_) => {
                    // Failed to check compatibility, skip silently (it's optional)
                }
            }
        }
        Ok(())
    }

    /// Check if an optional dependency is compatible with the current platform
    async fn check_optional_dep_compatible(&self, name: &str, range: &str) -> Result<bool> {
        // Handle package aliases (e.g., "npm:@babel/traverse@^7.25.3")
        let (actual_name, actual_range) = if let Some(alias) = parse_package_alias(range) {
            (alias.actual_name, alias.version_range)
        } else {
            (name.to_string(), range.to_string())
        };
        
        let package = self.registry.get_package(&actual_name).await?;
        let resolved = self.registry.resolve_version(&package, &actual_range)?;
        Ok(is_version_platform_compatible(resolved))
    }

    async fn fetch_and_resolve(
        &self,
        name: &str,
        range: &str,
    ) -> Result<(
        String,
        String,
        BTreeMap<String, String>,
        BTreeMap<String, String>,
        BTreeMap<String, String>,
        Option<String>,
        Option<serde_json::Value>,
    )> {
        let _permit = self.semaphore.acquire().await?;
        
        // Handle package aliases (e.g., "npm:@babel/traverse@^7.25.3")
        let (actual_name, actual_range) = if let Some(alias) = parse_package_alias(range) {
            (alias.actual_name, alias.version_range)
        } else {
            (name.to_string(), range.to_string())
        };
        
        let package = self.registry.get_package(&actual_name).await?;
        let resolved = self.registry.resolve_version(&package, &actual_range)?;

        let postinstall = resolved
            .scripts
            .get("postinstall")
            .or(resolved.scripts.get("install"))
            .cloned();

        Ok((
            resolved.version.clone(),
            resolved.dist.tarball.clone(),
            resolved.dependencies.clone(),
            resolved.peer_dependencies.clone(),
            resolved.optional_dependencies.clone(),
            postinstall,
            resolved.bin.clone(),
        ))
    }
}

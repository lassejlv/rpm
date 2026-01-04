use crate::installer::Installer;
use crate::registry::Registry;
use crate::types::{LockFile, LockPackage, PackageJson};
use anyhow::{Context, Result};
use dashmap::DashMap;
use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Semaphore;

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
}

impl Manager {
    pub fn new(force_no_cache: bool, auto_confirm: bool) -> Self {
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
        }
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

        let spinner = self.multi_progress.add(ProgressBar::new_spinner());
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
        );
        spinner.enable_steady_tick(std::time::Duration::from_millis(80));

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
            let _ = self.multi_progress.println(format!(
                "\x1b[32m+\x1b[0m \x1b[1m{}\x1b[0m@\x1b[90m{}\x1b[0m",
                name, resolved.version
            ));
        }
        spinner.finish_and_clear();

        let new_content = serde_json::to_string_pretty(&package_json)?;
        fs::write("package.json", new_content).await?;

        self.install_deps(&package_json).await?;
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

    pub async fn run_script(&self, script_name: &str, args: Vec<String>) -> Result<()> {
        let package_json_content = fs::read_to_string("package.json").await?;
        let package_json: PackageJson = serde_json::from_str(&package_json_content)?;

        let script = package_json.scripts.get(script_name).with_context(|| {
            let available: Vec<_> = package_json.scripts.keys().collect();
            if available.is_empty() {
                format!(
                    "Script '{}' not found. No scripts defined in package.json",
                    script_name
                )
            } else {
                format!(
                    "Script '{}' not found. Available scripts: {}",
                    script_name,
                    available
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        })?;

        println!("\x1b[90m$\x1b[0m \x1b[1m{}\x1b[0m\n", script);

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

    pub async fn install(&self) -> Result<()> {
        self.load_lockfile().await?;
        let package_json_content = fs::read_to_string("package.json").await?;
        let package_json: PackageJson = serde_json::from_str(&package_json_content)?;

        // Main spinner for overall progress
        let pb = self.multi_progress.add(ProgressBar::new(0));
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} \x1b[1mInstalling\x1b[0m dependencies...")
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(80));

        self.install_deps(&package_json).await?;
        pb.finish_and_clear();

        self.run_postinstalls().await?;
        self.save_lockfile(&package_json.name, &package_json.version)
            .await?;

        Ok(())
    }

    async fn install_deps(&self, package_json: &PackageJson) -> Result<()> {
        let root = std::env::current_dir()?;
        let mut tasks = FuturesUnordered::new();

        for (name, version) in &package_json.dependencies {
            let root = root.clone();
            let name = name.clone();
            let version = version.clone();
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
        if self.postinstalls.is_empty() {
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

        let pb = self
            .multi_progress
            .add(ProgressBar::new(scripts_to_run.len() as u64));
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.cyan} [{bar:40.cyan/blue}] {pos}/{len} \x1b[1mRunning\x1b[0m postinstall scripts...")
            .unwrap()
            .progress_chars("━╸─")
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"));

        for (name, (path, script)) in scripts_to_run {
            pb.set_message(format!("Running postinstall for {}", name));

            let status = Command::new("sh")
                .arg("-c")
                .arg(&script)
                .current_dir(&path)
                .status()
                .await;

            if status.is_err() && cfg!(windows) {
                let _ = Command::new("cmd")
                    .arg("/C")
                    .arg(&script)
                    .current_dir(&path)
                    .status()
                    .await;
            }

            pb.inc(1);
        }

        pb.finish_and_clear();
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

        let lock_entry = {
            let lock = self.lockfile.lock().await;
            let key = format!("node_modules/{}", name);
            lock.packages.get(&key).cloned()
        };

        let (version, tarball, deps, postinstall, bin) = if let Some(entry) = lock_entry {
            let matches = semver::Version::parse(&entry.version)
                .ok()
                .and_then(|v| {
                    semver::VersionReq::parse(&version_range)
                        .ok()
                        .map(|r| r.matches(&v))
                })
                .unwrap_or(false);

            if matches || version_range == entry.version {
                (
                    entry.version,
                    entry.resolved,
                    entry.dependencies,
                    entry.postinstall,
                    entry.bin,
                )
            } else {
                self.fetch_and_resolve(&name, &version_range).await?
            }
        } else {
            self.fetch_and_resolve(&name, &version_range).await?
        };

        if self.installed.contains_key(&name) {
            return Ok(());
        }
        self.installed.insert(name.clone(), version.clone());

        let install_path = target_dir.join("node_modules").join(&name);
        let already_exists = install_path.join("package.json").exists();

        if !already_exists {
            // Create a temporary spinner for this package download
            let pb = self.multi_progress.add(ProgressBar::new(0));
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
            );
            pb.enable_steady_tick(std::time::Duration::from_millis(100));
            pb.set_message(format!("\x1b[1m{}\x1b[0m@\x1b[90m{}\x1b[0m", name, version));

            let install_res = async {
                let install_dir = std::env::current_dir().unwrap();
                self.installer
                    .install_package(&name, &version, &tarball, &install_dir)
                    .await
            }
            .await;

            match install_res {
                Ok(_) => {
                    // Success! Clear the spinner
                    pb.finish_and_clear();

                    // Collect postinstall if exists
                    if let Some(script) = &postinstall {
                        self.postinstalls
                            .insert(name.clone(), (install_path.clone(), script.clone()));
                    }
                }
                Err(e) => {
                    pb.finish_with_message(format!(
                        "\x1b[31mx\x1b[0m \x1b[1m{}\x1b[0m failed: {}",
                        name, e
                    ));
                    return Err(e);
                }
            }
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
                    postinstall: postinstall.clone(),
                    bin: bin.clone(),
                },
            );
        }

        let mut tasks = FuturesUnordered::new();
        for (dep_name, dep_ver) in deps {
            let dep_name = dep_name.clone();
            let dep_ver = dep_ver.clone();
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
        Ok(())
    }

    async fn fetch_and_resolve(
        &self,
        name: &str,
        range: &str,
    ) -> Result<(
        String,
        String,
        BTreeMap<String, String>,
        Option<String>,
        Option<serde_json::Value>,
    )> {
        let _permit = self.semaphore.acquire().await?;
        let package = self.registry.get_package(name).await?;
        let resolved = self.registry.resolve_version(&package, range)?;

        let postinstall = resolved
            .scripts
            .get("postinstall")
            .or(resolved.scripts.get("install"))
            .cloned();

        Ok((
            resolved.version.clone(),
            resolved.dist.tarball.clone(),
            resolved.dependencies.clone(),
            postinstall,
            resolved.bin.clone(),
        ))
    }
}

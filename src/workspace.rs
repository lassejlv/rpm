use crate::types::{PackageJson, WorkspaceMember};
use anyhow::{Context, Result};
use glob::glob;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tokio::fs;

/// pnpm-workspace.yaml structure
#[derive(Debug, Deserialize)]
struct PnpmWorkspace {
    packages: Vec<String>,
}

/// Workspace manager for handling monorepo operations
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Root directory of the workspace
    pub root: PathBuf,
    /// Root package.json
    pub root_package: PackageJson,
    /// All workspace members
    pub members: Vec<WorkspaceMember>,
}

impl Workspace {
    /// Discover workspace from the current directory
    /// Supports both npm/yarn style (package.json workspaces) and pnpm style (pnpm-workspace.yaml)
    pub async fn discover(root: &Path) -> Result<Option<Self>> {
        let package_json_path = root.join("package.json");
        if !package_json_path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&package_json_path)
            .await
            .context("Failed to read package.json")?;
        let root_package: PackageJson =
            serde_json::from_str(&content).context("Failed to parse package.json")?;

        // Try to get workspace patterns from multiple sources
        let workspace_patterns = Self::get_workspace_patterns(root, &root_package).await;

        // Check if this is a workspace root
        if workspace_patterns.is_empty() {
            return Ok(None);
        }

        let members = Self::discover_members(root, &workspace_patterns).await?;

        Ok(Some(Self {
            root: root.to_path_buf(),
            root_package,
            members,
        }))
    }

    /// Get workspace patterns from package.json or pnpm-workspace.yaml
    async fn get_workspace_patterns(root: &Path, root_package: &PackageJson) -> Vec<String> {
        // 1. Check package.json workspaces field (npm/yarn style)
        if !root_package.workspaces.is_empty() {
            return root_package.workspaces.clone();
        }

        // 2. Check pnpm-workspace.yaml (pnpm style)
        let pnpm_workspace_path = root.join("pnpm-workspace.yaml");
        if let Ok(content) = fs::read_to_string(&pnpm_workspace_path).await {
            if let Ok(pnpm_workspace) = serde_yaml::from_str::<PnpmWorkspace>(&content) {
                return pnpm_workspace.packages;
            }
        }

        // 3. Also check pnpm-workspace.yml (alternative extension)
        let pnpm_workspace_yml = root.join("pnpm-workspace.yml");
        if let Ok(content) = fs::read_to_string(&pnpm_workspace_yml).await {
            if let Ok(pnpm_workspace) = serde_yaml::from_str::<PnpmWorkspace>(&content) {
                return pnpm_workspace.packages;
            }
        }

        Vec::new()
    }

    /// Discover all workspace members from glob patterns
    async fn discover_members(root: &Path, patterns: &[String]) -> Result<Vec<WorkspaceMember>> {
        let mut members = Vec::new();

        for pattern in patterns {
            let full_pattern = root.join(pattern).to_string_lossy().to_string();

            // Use glob to find matching directories
            let paths: Vec<PathBuf> = glob(&full_pattern)
                .context(format!("Invalid glob pattern: {}", pattern))?
                .filter_map(|p| p.ok())
                .filter(|p| p.is_dir())
                .collect();

            for path in paths {
                let pkg_json_path = path.join("package.json");
                if !pkg_json_path.exists() {
                    continue;
                }

                let content = match fs::read_to_string(&pkg_json_path).await {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let package_json: PackageJson = match serde_json::from_str(&content) {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                members.push(WorkspaceMember {
                    name: package_json.name.clone(),
                    path,
                    package_json,
                });
            }
        }

        // Sort by name for consistent ordering
        members.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(members)
    }

    /// Get all dependencies across the workspace, with version conflict detection
    /// Returns (package_name -> (version, list of workspaces using it))
    pub fn collect_all_dependencies(&self) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
        let mut deps: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();

        // Helper to add deps from a package
        let mut add_deps = |pkg_name: &str, dependencies: &BTreeMap<String, String>| {
            for (dep_name, version) in dependencies {
                deps.entry(dep_name.clone())
                    .or_default()
                    .entry(version.clone())
                    .or_default()
                    .push(pkg_name.to_string());
            }
        };

        // Root package dependencies
        add_deps(&self.root_package.name, &self.root_package.dependencies);
        add_deps(&self.root_package.name, &self.root_package.dev_dependencies);

        // Workspace member dependencies
        for member in &self.members {
            add_deps(&member.name, &member.package_json.dependencies);
            add_deps(&member.name, &member.package_json.dev_dependencies);
        }

        deps
    }

    /// Get hoisted dependencies (shared across workspaces, resolved to single version)
    /// Uses the highest version when there are conflicts
    pub fn get_hoisted_dependencies(&self) -> BTreeMap<String, String> {
        let all_deps = self.collect_all_dependencies();
        let mut hoisted: BTreeMap<String, String> = BTreeMap::new();

        for (dep_name, versions) in all_deps {
            // Skip workspace packages (they're local)
            if self.members.iter().any(|m| m.name == dep_name) {
                continue;
            }

            // Pick the best version (prefer the most commonly used, then highest)
            let best_version = versions
                .iter()
                .max_by(|(v1, users1), (v2, users2)| {
                    // First compare by usage count
                    match users1.len().cmp(&users2.len()) {
                        std::cmp::Ordering::Equal => {
                            // Then by version (higher is better)
                            Self::compare_versions(v1, v2)
                        }
                        other => other,
                    }
                })
                .map(|(v, _)| v.clone())
                .unwrap_or_default();

            if !best_version.is_empty() {
                hoisted.insert(dep_name, best_version);
            }
        }

        hoisted
    }

    /// Compare two version strings (simple comparison, prefers higher versions)
    fn compare_versions(v1: &str, v2: &str) -> std::cmp::Ordering {
        // Strip prefixes like ^, ~, >=, etc.
        fn clean_version(v: &str) -> &str {
            v.trim_start_matches('^')
                .trim_start_matches('~')
                .trim_start_matches(">=")
                .trim_start_matches("<=")
                .trim_start_matches('>')
                .trim_start_matches('<')
        }

        let v1_clean = clean_version(v1);
        let v2_clean = clean_version(v2);

        // Try semver parsing
        match (
            semver::Version::parse(v1_clean),
            semver::Version::parse(v2_clean),
        ) {
            (Ok(sv1), Ok(sv2)) => sv1.cmp(&sv2),
            _ => v1_clean.cmp(v2_clean),
        }
    }

    /// Get the list of workspace package names (for linking)
    pub fn get_workspace_package_names(&self) -> Vec<String> {
        self.members.iter().map(|m| m.name.clone()).collect()
    }

    /// Find a workspace member by name
    pub fn find_member(&self, name: &str) -> Option<&WorkspaceMember> {
        self.members.iter().find(|m| m.name == name)
    }

    /// Find a workspace member by path
    pub fn find_member_by_path(&self, path: &Path) -> Option<&WorkspaceMember> {
        self.members.iter().find(|m| m.path == path)
    }

    /// Get all scripts of a given name across workspaces
    pub fn get_scripts(&self, script_name: &str) -> Vec<(&WorkspaceMember, &String)> {
        self.members
            .iter()
            .filter_map(|m| m.package_json.scripts.get(script_name).map(|s| (m, s)))
            .collect()
    }

    /// Print workspace info
    pub fn print_info(&self) {
        println!(
            "\x1b[1;36mWorkspace:\x1b[0m \x1b[1m{}\x1b[0m",
            self.root_package.name
        );
        println!("\x1b[90m{} packages\x1b[0m\n", self.members.len());

        for member in &self.members {
            let relative_path = member.path.strip_prefix(&self.root).unwrap_or(&member.path);
            println!(
                "  \x1b[32m•\x1b[0m \x1b[1m{}\x1b[0m \x1b[90m({})\x1b[0m",
                member.name,
                relative_path.display()
            );
        }
    }
}

use crate::output::RpmError;
use crate::types::{RegistryPackage, RegistryVersion};
use anyhow::Result;
use dashmap::DashMap;
use reqwest::Client;
use semver::{Version, VersionReq};
use std::sync::Arc;
use std::time::Duration;

/// Represents a resolved package alias
/// e.g., "npm:@babel/traverse@^7.25.3" -> actual_name: "@babel/traverse", version_range: "^7.25.3"
#[derive(Debug, Clone)]
pub struct ResolvedAlias {
    pub actual_name: String,
    pub version_range: String,
}

/// Parse an npm package alias (e.g., "npm:@babel/traverse@^7.25.3")
/// Returns None if not an alias, Some(ResolvedAlias) if it is
pub fn parse_package_alias(version_spec: &str) -> Option<ResolvedAlias> {
    if !version_spec.starts_with("npm:") {
        return None;
    }
    
    let spec = &version_spec[4..]; // Remove "npm:" prefix
    
    // Handle scoped packages (@scope/name@version)
    if spec.starts_with('@') {
        // Find the second @ which separates name from version
        if let Some(at_pos) = spec[1..].find('@') {
            let actual_at_pos = at_pos + 1;
            return Some(ResolvedAlias {
                actual_name: spec[..actual_at_pos].to_string(),
                version_range: spec[actual_at_pos + 1..].to_string(),
            });
        }
        // No version specified, use latest
        return Some(ResolvedAlias {
            actual_name: spec.to_string(),
            version_range: "latest".to_string(),
        });
    }
    
    // Handle non-scoped packages (name@version)
    if let Some(at_pos) = spec.find('@') {
        return Some(ResolvedAlias {
            actual_name: spec[..at_pos].to_string(),
            version_range: spec[at_pos + 1..].to_string(),
        });
    }
    
    // No version specified
    Some(ResolvedAlias {
        actual_name: spec.to_string(),
        version_range: "latest".to_string(),
    })
}

#[derive(Clone)]
pub struct Registry {
    client: Client,
    base_url: String,
    cache: Arc<DashMap<String, RegistryPackage>>,
}

impl Registry {
    pub fn new() -> Self {
        // Configure client with connection pooling and keep-alive
        let client = Client::builder()
            .pool_max_idle_per_host(20)
            .pool_idle_timeout(Duration::from_secs(30))
            .tcp_keepalive(Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            base_url: "https://registry.npmjs.org".to_string(),
            cache: Arc::new(DashMap::new()),
        }
    }

    pub async fn get_package(&self, name: &str) -> Result<RegistryPackage> {
        // Check in-memory cache first
        if let Some(cached) = self.cache.get(name) {
            return Ok(cached.value().clone());
        }

        let url = format!("{}/{}", self.base_url, name);
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                return Err(RpmError::NetworkError {
                    name: name.to_string(),
                    status: None,
                    message: e.to_string(),
                }
                .into());
            }
        };

        let status = resp.status();
        if !status.is_success() {
            // Generate suggestions for 404 errors
            let suggestions = if status.as_u16() == 404 {
                self.generate_package_suggestions(name)
            } else {
                vec![]
            };

            return Err(if status.as_u16() == 404 {
                RpmError::PackageNotFound {
                    name: name.to_string(),
                    suggestions,
                }
            } else {
                RpmError::NetworkError {
                    name: name.to_string(),
                    status: Some(status.as_u16()),
                    message: status.to_string(),
                }
            }
            .into());
        }

        let package: RegistryPackage = match resp.json().await {
            Ok(p) => p,
            Err(e) => {
                return Err(RpmError::ParseError {
                    name: name.to_string(),
                    message: e.to_string(),
                }
                .into());
            }
        };

        // Store in cache
        self.cache.insert(name.to_string(), package.clone());

        Ok(package)
    }

    /// Generate package name suggestions for typos
    fn generate_package_suggestions(&self, name: &str) -> Vec<String> {
        // Common npm package prefixes/suffixes that users might forget
        let mut suggestions = Vec::new();

        // If it doesn't start with @, maybe it's a scoped package
        if !name.starts_with('@') {
            // Common scopes
            for scope in ["@types", "@babel", "@vue", "@angular", "@react-native"] {
                suggestions.push(format!("{}/{}", scope, name));
            }
        }

        // Common typos: missing -js, -node suffixes
        if !name.ends_with("-js") && !name.ends_with("js") {
            suggestions.push(format!("{}-js", name));
            suggestions.push(format!("{}js", name));
        }

        // If name has dashes, maybe wrong dash placement
        if name.contains('-') {
            let no_dash = name.replace('-', "");
            suggestions.push(no_dash);
        }

        // Limit suggestions
        suggestions.truncate(5);
        suggestions
    }

    pub fn resolve_version<'a>(
        &self,
        package: &'a RegistryPackage,
        range: &str,
    ) -> Result<&'a RegistryVersion> {
        // Try dist-tags first (e.g., "latest", "next", "beta")
        if let Some(tag_version) = package.dist_tags.get(range) {
            return package
                .versions
                .get(tag_version)
                .ok_or_else(|| RpmError::VersionNotFound {
                    name: package._name.clone(),
                    requested: range.to_string(),
                    available: self.get_available_versions(package),
                })
                .map_err(|e| e.into());
        }

        let req = VersionReq::parse(range).unwrap_or_else(|_| VersionReq::parse("*").unwrap());

        let mut valid_versions: Vec<&RegistryVersion> = package
            .versions
            .values()
            .filter(|v| {
                Version::parse(&v.version)
                    .map(|parsed| req.matches(&parsed))
                    .unwrap_or(false)
            })
            .collect();

        valid_versions.sort_by(|a, b| {
            let va = Version::parse(&a.version).unwrap();
            let vb = Version::parse(&b.version).unwrap();
            vb.cmp(&va)
        });

        valid_versions.first().cloned().ok_or_else(|| {
            RpmError::VersionNotFound {
                name: package._name.clone(),
                requested: range.to_string(),
                available: self.get_available_versions(package),
            }
            .into()
        })
    }

    /// Get a sorted list of available versions for error messages
    fn get_available_versions(&self, package: &RegistryPackage) -> Vec<String> {
        let mut versions: Vec<&str> = package.versions.keys().map(|s| s.as_str()).collect();

        // Sort by semver (descending)
        versions.sort_by(|a, b| {
            let va = Version::parse(a).ok();
            let vb = Version::parse(b).ok();
            match (va, vb) {
                (Some(va), Some(vb)) => vb.cmp(&va),
                _ => b.cmp(a),
            }
        });

        versions.into_iter().map(|s| s.to_string()).collect()
    }
}

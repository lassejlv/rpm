use crate::types::{RegistryPackage, RegistryVersion};
use anyhow::{Context, Result};
use dashmap::DashMap;
use reqwest::Client;
use semver::{Version, VersionReq};
use std::sync::Arc;
use std::time::Duration;

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
        let resp = self.client.get(&url).send().await?;
        
        if !resp.status().is_success() {
            anyhow::bail!("Failed to fetch package {}: {}", name, resp.status());
        }

        let package: RegistryPackage = resp.json().await.context("Failed to parse registry response")?;
        
        // Store in cache
        self.cache.insert(name.to_string(), package.clone());
        
        Ok(package)
    }

    pub fn resolve_version<'a>(
        &self,
        package: &'a RegistryPackage,
        range: &str,
    ) -> Result<&'a RegistryVersion> {
        if let Some(tag_version) = package.dist_tags.get(range) {
            return package.versions.get(tag_version)
                .context("Version from dist-tags not found in versions");
        }

        let req = VersionReq::parse(range).unwrap_or_else(|_| VersionReq::parse("*").unwrap());
        
        let mut valid_versions: Vec<&RegistryVersion> = package.versions.values()
            .filter(|v| {
                Version::parse(&v.version).map(|parsed| req.matches(&parsed)).unwrap_or(false)
            })
            .collect();

        valid_versions.sort_by(|a, b| {
            let va = Version::parse(&a.version).unwrap();
            let vb = Version::parse(&b.version).unwrap();
            vb.cmp(&va)
        });

        valid_versions.first().cloned().context("No matching version found")
    }
}

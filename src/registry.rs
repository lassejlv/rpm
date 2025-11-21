use crate::types::{RegistryPackage, RegistryVersion};
use anyhow::{Context, Result};
use reqwest::Client;
use semver::{Version, VersionReq};

#[derive(Clone)]
pub struct Registry {
    client: Client,
    base_url: String,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: "https://registry.npmjs.org".to_string(),
        }
    }

    pub async fn get_package(&self, name: &str) -> Result<RegistryPackage> {
        let url = format!("{}/{}", self.base_url, name);
        let resp = self.client.get(&url).send().await?;
        
        if !resp.status().is_success() {
            anyhow::bail!("Failed to fetch package {}: {}", name, resp.status());
        }

        resp.json::<RegistryPackage>().await.context("Failed to parse registry response")
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

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

fn default_version() -> String {
    "0.0.0".to_string()
}

fn deserialize_version<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_else(default_version))
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PackageJson {
    pub name: String,
    #[serde(default = "default_version", deserialize_with = "deserialize_version")]
    pub version: String,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    #[serde(default)]
    pub dev_dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "peerDependencies")]
    pub peer_dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "optionalDependencies")]
    pub optional_dependencies: BTreeMap<String, String>,
    #[serde(default)]
    pub scripts: HashMap<String, String>,
    #[serde(default)]
    pub bin: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct RegistryPackage {
    #[serde(rename = "name")]
    pub _name: String,
    #[serde(rename = "dist-tags")]
    pub dist_tags: HashMap<String, String>,
    pub versions: HashMap<String, RegistryVersion>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RegistryVersion {
    #[serde(rename = "name")]
    pub _name: String,
    pub version: String,
    pub dist: RegistryDist,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "peerDependencies")]
    pub peer_dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "optionalDependencies")]
    pub optional_dependencies: BTreeMap<String, String>,
    #[serde(default)]
    pub scripts: HashMap<String, String>,
    #[serde(default)]
    pub bin: Option<Value>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RegistryDist {
    pub tarball: String,
    #[allow(dead_code)]
    pub integrity: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LockFile {
    pub name: String,
    pub version: String,
    pub lockfile_version: u32,
    #[serde(default)]
    pub packages: BTreeMap<String, LockPackage>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LockPackage {
    pub version: String,
    pub resolved: String,
    pub integrity: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, String>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        rename = "peerDependencies"
    )]
    pub peer_dependencies: BTreeMap<String, String>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        rename = "optionalDependencies"
    )]
    pub optional_dependencies: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postinstall: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<Value>,
}

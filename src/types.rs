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

/// Deserialize a BTreeMap that might be null in JSON
fn deserialize_null_default_btreemap<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<BTreeMap<String, String>> = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

/// Deserialize a HashMap that might be null in JSON
fn deserialize_null_default_hashmap<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<HashMap<String, String>> = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PackageJson {
    pub name: String,
    #[serde(default = "default_version", deserialize_with = "deserialize_version")]
    pub version: String,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    #[serde(default, alias = "devDependencies", alias = "dev_dependencies")]
    pub dev_dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "peerDependencies")]
    pub peer_dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "optionalDependencies")]
    pub optional_dependencies: BTreeMap<String, String>,
    #[serde(default)]
    pub scripts: HashMap<String, String>,
    #[serde(default)]
    pub bin: Option<Value>,
    /// Workspace glob patterns (e.g., ["packages/*", "apps/*"])
    #[serde(default)]
    pub workspaces: Vec<String>,
}

/// Represents a workspace member with its path and package.json
#[derive(Debug, Clone)]
pub struct WorkspaceMember {
    pub name: String,
    pub path: std::path::PathBuf,
    pub package_json: PackageJson,
}

#[derive(Debug, Deserialize, Clone)]
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
    #[serde(default, deserialize_with = "deserialize_null_default_btreemap")]
    pub dependencies: BTreeMap<String, String>,
    #[serde(
        default,
        rename = "peerDependencies",
        deserialize_with = "deserialize_null_default_btreemap"
    )]
    pub peer_dependencies: BTreeMap<String, String>,
    #[serde(
        default,
        rename = "optionalDependencies",
        deserialize_with = "deserialize_null_default_btreemap"
    )]
    pub optional_dependencies: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "deserialize_null_default_hashmap")]
    pub scripts: HashMap<String, String>,
    #[serde(default)]
    pub bin: Option<Value>,
    /// Platform restrictions - list of supported operating systems
    #[serde(default)]
    pub os: Vec<String>,
    /// Platform restrictions - list of supported CPU architectures
    #[serde(default)]
    pub cpu: Vec<String>,
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

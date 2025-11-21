use std::collections::{BTreeMap, HashMap};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize, Serialize)]
pub struct PackageJson {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    #[serde(default)]
    pub dev_dependencies: BTreeMap<String, String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postinstall: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<Value>,
}

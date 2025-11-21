use anyhow::{Result};
use flate2::read::GzDecoder;
use reqwest::Client;
use std::path::{Path, PathBuf};
use tar::Archive;
use tokio::fs;

#[derive(Clone)]
pub struct Installer {
    client: Client,
    pub cache_dir: PathBuf,
    force_no_cache: bool,
}

impl Installer {
    pub fn new(force_no_cache: bool) -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .expect("Could not determine home directory");
        let cache_dir = PathBuf::from(home).join(".rpm").join("store");
        
        Self {
            client: Client::new(),
            cache_dir,
            force_no_cache,
        }
    }

    fn get_cache_path(&self, name: &str, version: &str) -> PathBuf {
        let safe_name = name.replace('/', "+");
        self.cache_dir.join(format!("{}@{}", safe_name, version))
    }

    async fn ensure_cache_entry(&self, name: &str, version: &str, tarball_url: &str) -> Result<PathBuf> {
        let cache_path = self.get_cache_path(name, version);
        
        if !self.force_no_cache && cache_path.exists() {
            return Ok(cache_path);
        }

        if self.force_no_cache && cache_path.exists() {
             fs::remove_dir_all(&cache_path).await?;
        }

        // Download
        let resp = self.client.get(tarball_url).send().await?;
        let bytes = resp.bytes().await?;

        let temp_dir = self.cache_dir.join("tmp").join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&temp_dir).await?;

        let temp_dir_clone = temp_dir.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let tar = GzDecoder::new(&bytes[..]);
            let mut archive = Archive::new(tar);

            archive.entries()?.filter_map(|e| e.ok()).for_each(|mut entry| {
                let path = entry.path().unwrap();
                let path_str = path.to_string_lossy();
                
                // npm packages are usually inside "package/" folder in tarball
                let dest_path = if path_str.starts_with("package/") {
                    temp_dir_clone.join(path_str.trim_start_matches("package/"))
                } else {
                    temp_dir_clone.join(path)
                };

                if let Some(parent) = dest_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = entry.unpack(&dest_path);
            });
            Ok(())
        }).await??;

        // Move to final cache location
        // Create parent dir if needed
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        
        // Rename can fail if cross-device, but here we are usually in same home drive
        match fs::rename(&temp_dir, &cache_path).await {
            Ok(_) => Ok(cache_path),
            Err(_) => {
                // Fallback for cross-device move if tmp and cache are different mounts (unlikely for default ~/.rpm)
                // But simple rename is best effort
                // If rename fails (e.g. target exists race condition), we can just return target
                if cache_path.exists() {
                     let _ = fs::remove_dir_all(&temp_dir).await;
                     Ok(cache_path)
                } else {
                    anyhow::bail!("Failed to move cache entry")
                }
            }
        }
    }

    pub async fn install_package(&self, name: &str, version: &str, tarball_url: &str, target_dir: &Path) -> Result<()> {
        let cache_path = self.ensure_cache_entry(name, version, tarball_url).await?;
        let install_path = target_dir.join("node_modules").join(name);

        if install_path.exists() {
            fs::remove_dir_all(&install_path).await?;
        }
        if let Some(parent) = install_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // Recursive copy from cache to install_path
        copy_dir_recursive(&cache_path, &install_path).await?;

        Ok(())
    }
}

// Recursive copy helper
#[async_recursion::async_recursion]
async fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).await?;
    let mut entries = fs::read_dir(src).await?;

    while let Some(entry) = entries.next_entry().await? {
        let file_type = entry.file_type().await?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path).await?;
        } else {
            fs::copy(&src_path, &dst_path).await?;
        }
    }
    Ok(())
}

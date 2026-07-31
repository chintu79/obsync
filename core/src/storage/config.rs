use std::collections::HashMap;
use std::path::PathBuf;

pub struct ConfigStore {
    path: PathBuf,
    data: std::sync::Mutex<HashMap<String, String>>,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        let data = if path.exists() {
            std::fs::read(&path)
                .ok()
                .and_then(|content| bincode::deserialize(&content).ok())
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        Self {
            path,
            data: std::sync::Mutex::new(data),
        }
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, anyhow::Error> {
        let data = self.data.lock().unwrap();
        Ok(data.get(key).cloned())
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), anyhow::Error> {
        let mut data = self.data.lock().unwrap();
        data.insert(key.to_string(), value.to_string());
        let bytes = bincode::serialize(&*data)?;
        std::fs::write(&self.path, bytes)?;
        Ok(())
    }

    pub fn remove(&self, key: &str) -> Result<(), anyhow::Error> {
        let mut data = self.data.lock().unwrap();
        data.remove(key);
        let bytes = bincode::serialize(&*data)?;
        std::fs::write(&self.path, bytes)?;
        Ok(())
    }

    pub fn contains(&self, key: &str) -> bool {
        let data = self.data.lock().unwrap();
        data.contains_key(key)
    }

    pub fn keys(&self) -> Vec<String> {
        let data = self.data.lock().unwrap();
        data.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_config_set_get() {
        let dir = TempDir::new().unwrap();
        let store = ConfigStore::new(dir.path().join("config.bin"));
        store.set("vault_path", "/home/user/vault").unwrap();
        assert_eq!(
            store.get("vault_path").unwrap(),
            Some("/home/user/vault".into())
        );
    }

    #[test]
    fn test_config_persistence() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.bin");
        {
            let store = ConfigStore::new(path.clone());
            store.set("key1", "value1").unwrap();
        }
        {
            let store = ConfigStore::new(path);
            assert_eq!(store.get("key1").unwrap(), Some("value1".into()));
        }
    }

    #[test]
    fn test_config_remove() {
        let dir = TempDir::new().unwrap();
        let store = ConfigStore::new(dir.path().join("config.bin"));
        store.set("temp", "value").unwrap();
        assert!(store.contains("temp"));
        store.remove("temp").unwrap();
        assert!(!store.contains("temp"));
    }
}

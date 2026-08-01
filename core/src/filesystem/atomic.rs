use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub struct AtomicWriter {
    temp_path: PathBuf,
    final_path: PathBuf,
    file: Option<fs::File>,
}

impl AtomicWriter {
    pub fn new(final_path: PathBuf) -> io::Result<Self> {
        let temp_name = format!(
            ".{}.sync-temp",
            final_path.file_name().unwrap_or_default().to_string_lossy()
        );
        let temp_path = final_path.with_file_name(temp_name);

        let file = fs::File::create(&temp_path)?;

        Ok(Self {
            temp_path,
            final_path,
            file: Some(file),
        })
    }

    pub fn writer(&mut self) -> &mut fs::File {
        self.file.as_mut().unwrap()
    }

    pub fn commit(&mut self) -> io::Result<()> {
        if let Some(file) = self.file.take() {
            file.sync_all()?;
            drop(file);
            fs::rename(&self.temp_path, &self.final_path)?;
        }
        Ok(())
    }

    pub fn abort(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.temp_path);
    }
}

impl Drop for AtomicWriter {
    fn drop(&mut self) {
        if self.file.is_some() {
            self.abort();
        }
    }
}

pub fn cleanup_stale_temps(vault_path: &Path) -> io::Result<()> {
    if vault_path.is_dir() {
        for entry in fs::read_dir(vault_path)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".sync-temp") {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_atomic_write_commit() {
        let dir = TempDir::new().unwrap();
        let final_path = dir.path().join("test.md");

        {
            let mut writer = AtomicWriter::new(final_path.clone()).unwrap();
            writer.writer().write_all(b"hello world").unwrap();
            writer.commit().unwrap();
        }

        assert!(final_path.exists());
        let content = fs::read_to_string(&final_path).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_atomic_write_abort() {
        let dir = TempDir::new().unwrap();
        let final_path = dir.path().join("test.md");

        {
            let mut writer = AtomicWriter::new(final_path.clone()).unwrap();
            writer.writer().write_all(b"should not appear").unwrap();
            writer.abort();
        }

        assert!(!final_path.exists());
    }

    #[test]
    fn test_atomic_write_no_stale_temp_on_commit() {
        let dir = TempDir::new().unwrap();
        let final_path = dir.path().join("test.md");

        {
            let mut writer = AtomicWriter::new(final_path.clone()).unwrap();
            writer.writer().write_all(b"data").unwrap();
            writer.commit().unwrap();
        }

        let has_temp = std::fs::read_dir(dir.path()).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".sync-temp")
        });
        assert!(!has_temp);
    }

    #[test]
    fn test_cleanup_stale_temps() {
        let dir = TempDir::new().unwrap();
        let temp_path = dir.path().join(".test.md.sync-temp");
        fs::write(&temp_path, b"stale").unwrap();
        assert!(temp_path.exists());

        cleanup_stale_temps(dir.path()).unwrap();
        assert!(!temp_path.exists());
    }
}

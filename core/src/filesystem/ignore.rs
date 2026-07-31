use std::path::Path;

const IGNORED_PREFIXES: &[&str] = &[
    ".~",
    "~$",
    ".",
];

const IGNORED_SUFFIXES: &[&str] = &[
    ".swp",
    ".swx",
    ".tmp",
    ".temp",
    ".bak",
    ".sync-temp",
    "~",
];

const IGNORED_NAMES: &[&str] = &[
    ".DS_Store",
    "Thumbs.db",
    "thumbs.db",
    ".directory",
];

pub fn should_ignore(path: &Path) -> bool {
    let name = match path.file_name() {
        Some(n) => n.to_string_lossy(),
        None => return true,
    };

    if IGNORED_NAMES.iter().any(|n| *n == name.as_ref()) {
        return true;
    }

    if IGNORED_SUFFIXES.iter().any(|s| name.ends_with(s)) {
        return true;
    }

    if IGNORED_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return true;
    }

    if name.starts_with('.') && name.len() > 1 {
        // Ignore dotfiles except .obsidian/
        let is_obsidian = path.components().any(|c| {
            c.as_os_str().to_string_lossy().as_ref() == ".obsidian"
        });
        if !is_obsidian {
            // Still allow specific hidden files in .obsidian
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_ignore_hidden_dotfiles() {
        assert!(should_ignore(&PathBuf::from(".hidden.md")));
    }

    #[test]
    fn test_not_ignore_obsidian_dir() {
        assert!(!should_ignore(&PathBuf::from(".obsidian/config")));
        assert!(!should_ignore(&PathBuf::from(".obsidian/plugins/plugin/main.js")));
    }

    #[test]
    fn test_ignore_editor_swp() {
        assert!(should_ignore(&PathBuf::from("notes.md.swp")));
        assert!(should_ignore(&PathBuf::from("notes.md.swx")));
    }

    #[test]
    fn test_ignore_temp_files() {
        assert!(should_ignore(&PathBuf::from("notes.md.tmp")));
        assert!(should_ignore(&PathBuf::from("notes.md.bak")));
        assert!(should_ignore(&PathBuf::from("notes.md~")));
    }

    #[test]
    fn test_ignore_ds_store() {
        assert!(should_ignore(&PathBuf::from(".DS_Store")));
        assert!(should_ignore(&PathBuf::from("thumbs.db")));
    }

    #[test]
    fn test_not_ignore_normal_files() {
        assert!(!should_ignore(&PathBuf::from("notes.md")));
        assert!(!should_ignore(&PathBuf::from("project.md")));
        assert!(!should_ignore(&PathBuf::from("image.png")));
        assert!(!should_ignore(&PathBuf::from("notes/ideas.md")));
    }

    #[test]
    fn test_ignore_sync_temp() {
        assert!(should_ignore(&PathBuf::from(".notes.md.sync-temp")));
        assert!(should_ignore(&PathBuf::from("notes.md.sync-temp")));
    }
}

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Recursively discover `.txt` wordlists, mapping each path to its name relative to `root`.
pub fn discover_wordlists(root: &Path) -> Result<BTreeMap<PathBuf, PathBuf>> {
    let mut files = BTreeMap::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Could not list {}", directory.display()));
            }
        };
        for entry in entries {
            let entry = entry.with_context(|| format!("Could not list {}", directory.display()))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("Could not inspect {}", path.display()))?;
            if file_type.is_dir() {
                directories.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
                && (file_type.is_file() || file_type.is_symlink() && path.is_file())
            {
                let name = path.strip_prefix(root)?.to_path_buf();
                files.insert(path, name);
            }
        }
    }
    Ok(files)
}

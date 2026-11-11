use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use furigana4epub_core::default_wordlists_directory;

#[derive(Default)]
pub struct Wordlists {
    entries: BTreeMap<PathBuf, Wordlist>,
}

#[derive(Default)]
pub struct Wordlist {
    shared_name: Option<PathBuf>,
    pub ignored: bool,
}

impl Wordlist {
    pub fn label(&self, path: &Path) -> String {
        let name = self.shared_name.as_deref().unwrap_or(path);
        let stem = name
            .file_stem()
            .unwrap_or(name.as_os_str())
            .to_string_lossy();
        let Some(category) = self
            .shared_name
            .as_deref()
            .and_then(Path::parent)
            .filter(|parent| !parent.as_os_str().is_empty())
        else {
            return stem.into_owned();
        };
        if category.as_os_str().eq_ignore_ascii_case("jlpt") {
            return format!("JLPT ({})", stem.to_ascii_uppercase());
        }
        format!("{} ({stem})", category.display())
    }
}

impl Wordlists {
    pub fn replace_shared(&mut self, shared: BTreeMap<PathBuf, PathBuf>) {
        self.entries
            .retain(|path, wordlist| wordlist.shared_name.is_none() || shared.contains_key(path));
        for (path, name) in shared {
            self.entries.entry(path).or_default().shared_name = Some(name);
        }
    }

    pub fn add(&mut self, path: PathBuf) -> bool {
        let wordlist = self.entries.entry(path).or_default();
        let changed = !wordlist.ignored;
        wordlist.ignored = true;
        changed
    }

    pub fn set_ignored(&mut self, path: &Path, ignored: bool) {
        if let Some(wordlist) = self.entries.get_mut(path) {
            wordlist.ignored = ignored;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &Wordlist)> {
        self.entries.iter()
    }

    pub fn ignored_paths(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .filter(|(_, wordlist)| wordlist.ignored)
            .map(|(path, _)| path.clone())
            .collect()
    }
}

pub fn read_shared() -> Result<BTreeMap<PathBuf, PathBuf>> {
    let root = default_wordlists_directory()?;
    let mut files = BTreeMap::new();
    let mut directories = vec![root.clone()];
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
                let name = path.strip_prefix(&root)?.to_path_buf();
                files.insert(path, name);
            }
        }
    }
    Ok(files)
}

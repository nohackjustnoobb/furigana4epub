use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use furigana4epub_core::discover_wordlists;

pub struct Wordlists(BTreeMap<String, PathBuf>);

impl Wordlists {
    pub fn read(directory: &Path) -> Result<Self> {
        let entries = discover_wordlists(directory)?
            .into_iter()
            .filter_map(|(path, name)| {
                let components = name
                    .iter()
                    .map(|part| part.to_str())
                    .collect::<Option<Vec<_>>>()?;
                Some((components.join("/"), path))
            })
            .collect();
        Ok(Self(entries))
    }

    pub fn names(self) -> Vec<String> {
        self.0.into_keys().collect()
    }

    pub fn find(&self, name: &str) -> Option<&Path> {
        self.0.get(name).map(PathBuf::as_path)
    }
}

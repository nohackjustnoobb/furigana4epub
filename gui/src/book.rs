use std::fs;
use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use furigana4epub_core::{Furigana, Options};
use tempfile::NamedTempFile;

pub fn convert(path: &Path, options: Options<'_>) -> Result<Vec<u8>> {
    let epub = fs::read(path).with_context(|| format!("Could not read {}", path.display()))?;
    let converter = Furigana::from_default_dictionary()
        .context("Could not load the dictionary. Click Update Dictionary to install it.")?;
    converter.convert_epub(&epub, options)
}

pub fn is_epub(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("epub"))
}

pub fn save(path: &Path, epub: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = NamedTempFile::new_in(directory)
        .with_context(|| format!("Could not create a file in {}", directory.display()))?;
    file.write_all(epub)
        .with_context(|| format!("Could not write {}", path.display()))?;
    match file.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(anyhow!("That file already exists. Choose a new filename."))
        }
        Err(error) => {
            Err(error.error).with_context(|| format!("Could not save {}", path.display()))
        }
    }
}

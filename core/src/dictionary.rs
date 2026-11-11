use std::fs;
use std::io::{self, ErrorKind, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tempfile::NamedTempFile;
use zip::ZipArchive;

use crate::Furigana;
use crate::jlpt::remove_jlpt_wordlists;

const DICTIONARY_NAME: &str = "system_full.dic";
const DICTIONARY_URL: &str =
    "https://d2ej7fkh96fzlu.cloudfront.net/sudachidict/v1/sudachi-dictionary-latest-full.zip";

fn dictionary_directory() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("finding the user data directory")?
        .join("furigana4epub"))
}

/// Return the path to the shared dictionary in the local user data directory.
pub fn default_dictionary_path() -> Result<PathBuf> {
    Ok(dictionary_directory()?.join(DICTIONARY_NAME))
}

/// The result of removing the default dictionary and its JLPT wordlists.
pub struct DictionaryRemoval {
    pub dictionary_path: PathBuf,
    pub dictionary_removed: bool,
    pub wordlists_removed: bool,
}

/// Remove the default dictionary and the installed JLPT wordlists.
///
/// Missing files are treated as already removed.
pub fn remove_dictionary() -> Result<DictionaryRemoval> {
    remove_dictionary_at(default_dictionary_path()?)
}

fn remove_dictionary_at(path: PathBuf) -> Result<DictionaryRemoval> {
    let dictionary_removed = match fs::remove_file(&path) {
        Ok(()) => true,
        Err(error) if error.kind() == ErrorKind::NotFound => false,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("removing dictionary at {}", path.display()));
        }
    };
    let wordlists_removed = remove_jlpt_wordlists(&path).with_context(|| {
        let status = if dictionary_removed {
            "Removed dictionary at"
        } else {
            "No dictionary installed at"
        };
        format!(
            "{status} {}. Could not remove the JLPT wordlists",
            path.display()
        )
    })?;
    Ok(DictionaryRemoval {
        dictionary_path: path,
        dictionary_removed,
        wordlists_removed,
    })
}

/// Download and install the latest Sudachi Full V1 dictionary in the user data directory.
///
/// This blocks until the download finishes and returns the installed file path.
/// A failed update leaves any previously installed dictionary in place.
pub fn download_dictionary() -> Result<PathBuf> {
    let directory = dictionary_directory()?;
    fs::create_dir_all(&directory).with_context(|| format!("creating {}", directory.display()))?;
    let mut archive = tempfile::tempfile_in(&directory)
        .with_context(|| format!("creating a temporary download in {}", directory.display()))?;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .timeout_global(Some(Duration::from_secs(1800)))
        .build()
        .into();
    let mut response = agent
        .get(DICTIONARY_URL)
        .call()
        .context("requesting the Sudachi Full V1 dictionary")?;
    io::copy(&mut response.body_mut().as_reader(), &mut archive)
        .context("downloading the Sudachi Full V1 dictionary")?;

    install_dictionary(archive, &directory)
}

fn install_dictionary(archive: impl Read + Seek, directory: &Path) -> Result<PathBuf> {
    let mut archive = ZipArchive::new(archive).context("opening the downloaded dictionary ZIP")?;
    let entry_index = archive
        .file_names_raw()
        .position(|name| {
            name.rsplit(|byte| *byte == b'/').next() == Some(DICTIONARY_NAME.as_bytes())
        })
        .context("system_full.dic is missing from the downloaded ZIP")?;
    let mut entry = archive
        .by_index(entry_index)
        .context("opening system_full.dic in the downloaded ZIP")?;

    let mut dictionary = NamedTempFile::new_in(directory)
        .with_context(|| format!("creating a temporary dictionary in {}", directory.display()))?;
    io::copy(&mut entry, &mut dictionary).context("extracting system_full.dic")?;
    Furigana::new(dictionary.path()).context("validating the downloaded dictionary")?;
    dictionary
        .as_file()
        .sync_all()
        .context("saving system_full.dic")?;
    let path = directory.join(DICTIONARY_NAME);
    dictionary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("installing the dictionary at {}", path.display()))?;
    Ok(path)
}

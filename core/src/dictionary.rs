use std::fs;
use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tempfile::NamedTempFile;
use zip::ZipArchive;

use crate::Furigana;

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

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use super::*;

    #[test]
    fn failed_extraction_preserves_the_installed_dictionary() -> Result<()> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(
            "sudachi-dictionary/system_full.dic",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )?;
        zip.write_all(b"dictionary contents")?;
        let mut archive = zip.finish()?.into_inner();
        let directory = tempfile::tempdir()?;
        let path = directory.path().join(DICTIONARY_NAME);
        let original = b"previous dictionary";
        fs::write(&path, original)?;

        let data_start = ZipArchive::new(Cursor::new(&archive))?
            .by_index(0)?
            .data_start()
            .context("finding the test dictionary in the ZIP")?;
        archive[data_start as usize] ^= 1;
        let error = install_dictionary(Cursor::new(&archive), directory.path()).unwrap_err();
        assert_eq!(error.to_string(), "extracting system_full.dic");
        assert_eq!(fs::read(&path)?, original);
        Ok(())
    }
}

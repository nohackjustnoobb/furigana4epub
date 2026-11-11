use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use tempfile::NamedTempFile;

use crate::{default_dictionary_path, is_kanji};

const LEVELS: [usize; 5] = [1, 2, 3, 4, 5];
const SOURCE: &str = "https://raw.githubusercontent.com/jamsinclair/open-anki-jlpt-decks/main/src";

fn wordlists_directory(dictionary_path: &Path) -> PathBuf {
    dictionary_path.with_file_name("wordlists")
}

/// Return the path to the shared wordlists in the local user data directory.
pub fn default_wordlists_directory() -> Result<PathBuf> {
    Ok(wordlists_directory(&default_dictionary_path()?))
}

/// Download the JLPT vocabulary and return the installed wordlist directory.
///
/// Writes N1.txt through N5.txt in the wordlists/jlpt subdirectory alongside
/// the default dictionary. Each file lists kanji-containing spellings, one per line.
/// Each spelling appears at the easiest level assigned by its tags.
pub fn download_jlpt_wordlists() -> Result<PathBuf> {
    let directory = default_wordlists_directory()?.join("jlpt");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();

    let mut words = BTreeMap::new();
    for level in LEVELS {
        let mut response = agent
            .get(format!("{SOURCE}/n{level}.csv"))
            .call()
            .with_context(|| format!("requesting the JLPT N{level} CSV"))?;
        read_words(response.body_mut().as_reader(), &mut words)
            .with_context(|| format!("reading the JLPT N{level} CSV"))?;
    }

    install_wordlists(words, &directory)?;
    Ok(directory)
}

/// Remove the JLPT wordlists from the wordlists/jlpt subdirectory.
///
/// Returns whether any wordlist files were removed.
pub fn remove_jlpt_wordlists(dictionary_path: &Path) -> Result<bool> {
    let directory = wordlists_directory(dictionary_path).join("jlpt");
    if !directory
        .try_exists()
        .with_context(|| format!("checking {}", directory.display()))?
    {
        return Ok(false);
    }
    let _lock = lock_wordlists(&directory)?;
    let mut removed = false;
    for level in LEVELS {
        let path = directory.join(format!("N{level}.txt"));
        match fs::remove_file(&path) {
            Ok(()) => removed = true,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("removing {}", path.display()));
            }
        }
    }
    match fs::remove_dir(&directory) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(error) => {
            return Err(error).with_context(|| format!("removing {}", directory.display()));
        }
    }
    Ok(removed)
}

fn read_words(source: impl Read, words: &mut BTreeMap<String, usize>) -> Result<()> {
    let mut csv = csv::Reader::from_reader(source);
    let headers = csv.headers()?;
    let expression = headers
        .iter()
        .position(|header| header == "expression")
        .context("missing expression column")?;
    let tags = headers
        .iter()
        .position(|header| header == "tags")
        .context("missing tags column")?;

    for row in csv.records() {
        let row = row?;
        let Some(level) = easiest_level(&row[tags]) else {
            continue;
        };
        let expression = strip_annotations(&row[expression]);
        for word in expression
            .split([';', '；', ',', '、', '\n', '\r'])
            .map(str::trim)
        {
            if word.chars().any(is_kanji) {
                words
                    .entry(word.to_owned())
                    .and_modify(|previous| *previous = (*previous).max(level))
                    .or_insert(level);
            }
        }
    }
    Ok(())
}

fn easiest_level(tags: &str) -> Option<usize> {
    tags.split_whitespace()
        .filter_map(|tag| {
            let level = tag.strip_prefix("JLPT_")?;
            match level.strip_prefix('N').unwrap_or(level).as_bytes() {
                [digit @ b'1'..=b'5'] => Some(usize::from(*digit - b'0')),
                _ => None,
            }
        })
        .max()
}

fn strip_annotations(expression: &str) -> String {
    let mut depth = 0usize;
    expression
        .chars()
        .filter(|ch| match ch {
            '(' | '（' => {
                depth += 1;
                false
            }
            ')' | '）' => {
                depth = depth.saturating_sub(1);
                false
            }
            '～' | '〜' => false,
            _ => depth == 0,
        })
        .collect()
}

fn install_wordlists(words: BTreeMap<String, usize>, directory: &Path) -> Result<()> {
    let mut lists: [String; LEVELS.len()] = Default::default();
    for (word, level) in words {
        lists[level - 1].push_str(&word);
        lists[level - 1].push('\n');
    }

    let parent = directory
        .parent()
        .context("finding the wordlists directory")?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let _lock = lock_wordlists(directory)?;
    fs::create_dir_all(directory).with_context(|| format!("creating {}", directory.display()))?;
    let mut files = Vec::new();
    for (level, list) in LEVELS.into_iter().zip(lists) {
        let path = directory.join(format!("N{level}.txt"));
        let mut file = NamedTempFile::new_in(directory)
            .with_context(|| format!("creating a temporary wordlist for {}", path.display()))?;
        file.write_all(list.as_bytes())
            .and_then(|()| file.as_file().sync_all())
            .with_context(|| format!("saving the JLPT N{level} wordlist"))?;
        files.push((path, file));
    }

    for (path, file) in files {
        file.persist(&path)
            .map_err(|error| error.error)
            .with_context(|| format!("installing the JLPT wordlist at {}", path.display()))?;
    }
    Ok(())
}

fn lock_wordlists(directory: &Path) -> Result<File> {
    let lock_path = directory.with_file_name(".jlpt.lock");
    let lock = File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    lock.lock().context("locking the JLPT wordlists")?;
    Ok(lock)
}

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use furigana4epub_core::{
    Furigana, Options, default_wordlists_directory, download_dictionary, download_jlpt_wordlists,
    remove_dictionary,
};

const UPDATE_DICTIONARY_HELP: &str =
    "Install or update the Sudachi Full V1 dictionary and JLPT wordlists.";

const HELP: &str = "Add furigana to a Japanese EPUB 3 book.

Usage: furigana4epub [OPTIONS] <INPUT.epub> [OUTPUT.epub]
       furigana4epub update-dict
       furigana4epub remove-dict

Commands:
  update-dict              Install or update the Sudachi Full V1 dictionary and JLPT wordlists.
  remove-dict              Remove the shared dictionary and JLPT wordlists.

Options:
  --dictionary <PATH>      Use a local Sudachi V1 dictionary instead of the default.
  --ignore-wordlist <LIST> Skip words from a shared list (e.g. jlpt/n1) or custom file. Repeatable.
  --override-existing      Replace existing ruby when Sudachi can read its entire base.
  --highlight-injected     Color the furigana added by the converter.
  --highlight-color <HEX>  Highlight generated furigana with a CSS hex color such as #2563eb.
  --new-identifier         Generate a new primary EPUB identifier.
  --identifier <VALUE>     Set the primary EPUB identifier and take precedence over --new-identifier.
  -h, --help               Show this help.

If omitted, the output for book.epub is book.furigana.epub in the same directory.
The output must be a new file. Use -- before paths that start with a dash.
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("furigana4epub: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = env::args_os().skip(1).peekable();
    if let Some(command) = args.next_if(|arg| arg == "update-dict" || arg == "remove-dict") {
        return run_dictionary_command(&command, args);
    }

    let mut positional_only = false;
    let mut paths = Vec::new();
    let mut dictionary = None;
    let mut wordlists = Vec::new();
    let mut identifier = None;
    let mut highlight_color = None;
    let mut options = Options::default();

    while let Some(arg) = args.next() {
        if positional_only || !arg.as_encoded_bytes().starts_with(b"-") {
            paths.push(arg);
            continue;
        }

        match arg.to_str() {
            Some("-h" | "--help") => {
                print!("{HELP}");
                return Ok(());
            }
            Some("--") => positional_only = true,
            Some("--dictionary") => dictionary = Some(option_value(&mut args, "--dictionary")?),
            Some("--ignore-wordlist") => {
                wordlists.push(option_value(&mut args, "--ignore-wordlist")?)
            }
            Some("--override-existing") => options.override_existing = true,
            Some("--highlight-injected") => options.highlight_injected = true,
            Some("--highlight-color") => {
                highlight_color = Some(
                    option_value(&mut args, "--highlight-color")?
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("--highlight-color requires a UTF-8 value"))?,
                );
            }
            Some("--new-identifier") => options.new_identifier = true,
            Some("--identifier") => {
                identifier = Some(
                    option_value(&mut args, "--identifier")?
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("--identifier requires a UTF-8 value"))?,
                );
            }
            _ => bail!("unknown option {arg:?}. See --help."),
        }
    }

    let (input, output) = match paths.as_slice() {
        [input] => {
            let input = Path::new(input);
            (input, input.with_extension("furigana.epub"))
        }
        [input, output] => (Path::new(input), PathBuf::from(output)),
        _ => bail!("expected an input EPUB path and an optional output path. See --help."),
    };
    let wordlists = wordlists
        .into_iter()
        .map(resolve_wordlist)
        .collect::<Result<Vec<_>>>()?;
    options.ignore_wordlists = &wordlists;
    options.identifier = identifier.as_deref();
    options.highlight_color = highlight_color.as_deref();

    let started_at = Instant::now();
    let epub = fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    let converter = match dictionary.as_deref() {
        Some(path) => Furigana::new(path)?,
        None => Furigana::from_default_dictionary().context(
            "loading the default dictionary (run furigana4epub update-dict to install it)",
        )?,
    };
    let converted = converter
        .convert_epub(&epub, options)
        .with_context(|| format!("converting {}", input.display()))?;

    let mut file =
        File::create_new(&output).with_context(|| format!("creating {}", output.display()))?;
    if let Err(error) = file.write_all(&converted) {
        drop(file);
        fs::remove_file(&output).with_context(|| {
            format!(
                "removing incomplete output {} after write failed: {error}",
                output.display()
            )
        })?;
        return Err(error).with_context(|| format!("writing {}", output.display()));
    }
    drop(file);
    let elapsed = started_at.elapsed();

    let dictionary = dictionary.as_deref().map_or_else(
        || "system_full.dic (default)".into(),
        OsStr::to_string_lossy,
    );
    let existing_ruby = if options.override_existing {
        "Replace when readable"
    } else {
        "Preserve"
    };
    let highlighting = match options.highlight_color {
        Some(color) => format!("On ({color})"),
        None if options.highlight_injected => "On".to_owned(),
        None => "Off".to_owned(),
    };
    let identifier = match options.identifier {
        Some(value) => format!("Set to {value:?}"),
        None if options.new_identifier => "New UUID".to_owned(),
        None => "Preserve".to_owned(),
    };
    let wordlists = if wordlists.is_empty() {
        "None".to_owned()
    } else {
        wordlists
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!(
        "EPUB conversion complete
  Output         {output}
  Elapsed        {elapsed:.2?}

Options used
  Dictionary     {dictionary}
  Existing ruby  {existing_ruby}
  Highlighting   {highlighting}
  Identifier     {identifier}
  Ignored lists  {wordlists}",
        output = output.display(),
    );
    Ok(())
}

fn resolve_wordlist(value: OsString) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    let Some(filename) = path.file_name() else {
        return Ok(path);
    };
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Ok(path);
    }
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => return Ok(path),
        Ok(_) => {}
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {}
        Err(error) => return Err(error).with_context(|| format!("checking {}", path.display())),
    }

    let mut filename = filename.to_os_string();
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
    {
        filename.push(".txt");
    }
    let mut installed = default_wordlists_directory()?;
    for part in path.with_file_name(filename).iter() {
        let Some(entry) = find_wordlist_entry(&installed, part)? else {
            return Ok(path);
        };
        installed = entry;
    }
    Ok(if installed.is_file() { installed } else { path })
}

fn find_wordlist_entry(directory: &Path, name: &OsStr) -> Result<Option<PathBuf>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            return Ok(None);
        }
        Err(error) => {
            return Err(error).with_context(|| format!("listing {}", directory.display()));
        }
    };
    let mut matches = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("listing {}", directory.display()))?;
        let filename = entry.file_name();
        if filename == name {
            return Ok(Some(entry.path()));
        }
        if filename.eq_ignore_ascii_case(name) {
            matches.push(entry.path());
        }
    }
    match matches.len() {
        0 | 1 => Ok(matches.pop()),
        _ => bail!(
            "multiple wordlist matches for {}",
            directory.join(name).display()
        ),
    }
}

fn run_dictionary_command(command: &OsStr, args: impl Iterator<Item = OsString>) -> Result<()> {
    let (description, action): (_, fn() -> Result<()>) = if command == "update-dict" {
        (UPDATE_DICTIONARY_HELP, update_dictionary)
    } else {
        (
            "Remove the shared dictionary and JLPT wordlists.",
            remove_dictionary_command,
        )
    };
    let usage = format!("Usage: furigana4epub {}", command.to_string_lossy());
    let mut args = args.peekable();
    let help = args.next_if(|arg| arg == "-h" || arg == "--help").is_some();
    if let Some(arg) = args.next() {
        bail!("unexpected argument {arg:?}. {usage}");
    }
    if help {
        println!("{usage}\n\n{description}");
        return Ok(());
    }
    action()
}

fn update_dictionary() -> Result<()> {
    println!("Downloading the latest Sudachi Full V1 dictionary…");
    let path = download_dictionary()?;
    println!("Installed dictionary at {}", path.display());
    println!("Downloading the JLPT wordlists…");
    let path = download_jlpt_wordlists()?;
    println!("Installed JLPT wordlists at {}", path.display());
    Ok(())
}

fn remove_dictionary_command() -> Result<()> {
    let removed = remove_dictionary()?;
    let path = removed.dictionary_path.display();
    if removed.dictionary_removed {
        println!("Removed dictionary at {path}");
    } else {
        println!("No dictionary installed at {path}");
    }
    if removed.wordlists_removed {
        println!("Removed JLPT wordlists.");
    } else {
        println!("No JLPT wordlists installed.");
    }
    Ok(())
}

fn option_value(args: &mut impl Iterator<Item = OsString>, option: &str) -> Result<OsString> {
    args.next()
        .with_context(|| format!("{option} requires a value. See --help."))
}

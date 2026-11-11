use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use furigana4epub_core::{Furigana, Options, default_dictionary_path, download_dictionary};

const HELP: &str = "Add furigana to a Japanese EPUB 3 book.

Usage: furigana4epub [OPTIONS] <INPUT.epub> [OUTPUT.epub]
       furigana4epub update-dict
       furigana4epub remove-dict

Commands:
  update-dict              Install or update the default Sudachi Full V1 dictionary.
  remove-dict              Remove the default dictionary shared by the CLI and GUI.

Options:
  --dictionary <PATH>      Use a local Sudachi V1 dictionary instead of the default.
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
    println!(
        "EPUB conversion complete
  Output         {output}
  Elapsed        {elapsed:.2?}

Options used
  Dictionary     {dictionary}
  Existing ruby  {existing_ruby}
  Highlighting   {highlighting}
  Identifier     {identifier}",
        output = output.display(),
    );
    Ok(())
}

fn run_dictionary_command(command: &OsStr, args: impl Iterator<Item = OsString>) -> Result<()> {
    let (description, action): (_, fn() -> Result<()>) = if command == "update-dict" {
        (
            "Install or update the default Sudachi Full V1 dictionary.",
            update_dictionary,
        )
    } else {
        (
            "Remove the default dictionary shared by the CLI and GUI.",
            remove_dictionary,
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
    Ok(())
}

fn remove_dictionary() -> Result<()> {
    let path = default_dictionary_path()?;
    match fs::remove_file(&path) {
        Ok(()) => println!("Removed dictionary at {}", path.display()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            println!("No dictionary installed at {}", path.display());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("removing dictionary at {}", path.display()));
        }
    }
    Ok(())
}

fn option_value(args: &mut impl Iterator<Item = OsString>, option: &str) -> Result<OsString> {
    args.next()
        .with_context(|| format!("{option} requires a value. See --help."))
}

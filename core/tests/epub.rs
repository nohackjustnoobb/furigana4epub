use std::io::{Cursor, Read};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use furigana4epub_core::{Furigana, Options};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

const FIXTURE: &[u8] = include_bytes!("fixtures/japanese.epub");
const FIXED_IDENTIFIER: &str = "https://example.com/epub?id=fixture&edition=1";

// The expected hashes were verified with SudachiDict Full V1 20261015.
#[test]
fn converts_with_default_options() -> Result<()> {
    check_hash(
        Options::default(),
        "4d65edb91eae628e1cada24f7333c4ef9311b0a18c072dd29b096993cc897930",
    )
}

#[test]
fn overrides_existing_ruby() -> Result<()> {
    check_hash(
        Options {
            override_existing: true,
            ..Options::default()
        },
        "f32d9efa645d5d91b3f74c16c67de6348c8222a16b8ce9624a9cf67103ecc49a",
    )
}

#[test]
fn highlights_injected_ruby() -> Result<()> {
    check_hash(
        Options {
            highlight_injected: true,
            ..Options::default()
        },
        "a82d6b6c1ed7fd89ed04ad5a3c1d775f3440a1e3fe118952ca2bf51029a6c87b",
    )
}

#[test]
fn changes_primary_identifier() -> Result<()> {
    check_hash(
        Options {
            identifier: Some(FIXED_IDENTIFIER),
            ..Options::default()
        },
        "c1370343e4b335bfafe95d2fa74acb6d60a3e034cef755b474aaa1a811f0fe59",
    )
}

#[test]
fn converts_with_all_options() -> Result<()> {
    check_hash(
        Options {
            override_existing: true,
            highlight_injected: true,
            highlight_color: Some("#2563eb"),
            new_identifier: true,
            identifier: Some(FIXED_IDENTIFIER),
            ..Options::default()
        },
        "2d858bf774564ef45ddf534d865dcc48f62ab2ea472232dab90cc80a6898dd69",
    )
}

#[test]
fn skips_words_from_multiple_wordlists() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    std::fs::write(&first, "\u{feff} 寿司 \r\n\r\n東京\r\n")?;
    std::fs::write(&second, "食べる\n")?;

    let converted = converter()?.convert_epub(
        FIXTURE,
        Options {
            ignore_wordlists: &[first, second],
            override_existing: true,
            ..Options::default()
        },
    )?;
    let mut archive = ZipArchive::new(Cursor::new(converted))?;
    let mut chapter = String::new();
    archive
        .by_name("EPUB/Text/第1章.xhtml")?
        .read_to_string(&mut chapter)?;

    assert!(chapter.contains("<p>寿司を食べた。"));
    assert!(chapter.contains("<ruby id=\"existing\">東京<rp>(</rp><rt>とうけい</rt>"));
    assert!(chapter.contains("<ruby class=\"furigana4epub\">漢字<rt>かんじ</rt></ruby>"));
    Ok(())
}

fn check_hash(options: Options<'_>, expected: &str) -> Result<()> {
    let converted = converter()?.convert_epub(FIXTURE, options)?;
    let actual: String = Sha256::digest(converted)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(actual, expected);
    Ok(())
}

fn converter() -> Result<&'static Furigana> {
    static CONVERTER: OnceLock<Result<Furigana>> = OnceLock::new();
    CONVERTER
        .get_or_init(|| match std::env::var_os("SUDACHI_DICT_PATH") {
            Some(path) => Furigana::new(path),
            None => Furigana::from_default_dictionary().context(
                "run furigana4epub update-dict or set SUDACHI_DICT_PATH to a local dictionary",
            ),
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error:#}"))
}

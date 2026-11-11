use std::ops::Range;

use anyhow::{Context, Result, ensure};
use roxmltree::Node;
use sudachi::analysis::stateless_tokenizer::StatelessTokenizer;
use sudachi::analysis::{Mode, Tokenize};
use sudachi::dic::dictionary::JapaneseDictionary;
use unicode_script::{Script, UnicodeScript};

use crate::{Edit, Options, apply_edits, closing_tag, parse_xml};

const XHTML: &str = "http://www.w3.org/1999/xhtml";
const RUBY_CLASS: &str = "furigana4epub";
const DEFAULT_HIGHLIGHT_COLOR: &str = "#c2410c";
const HIGHLIGHT_PREFIX: &str = "ruby.furigana4epub > rt { color: ";
const HIGHLIGHT_SUFFIX: &str = "; }";

type Tokenizer<'a> = StatelessTokenizer<&'a JapaneseDictionary>;

pub(super) fn highlight_rule(options: Options<'_>) -> Result<Option<String>> {
    let color = options.highlight_color.or(options
        .highlight_injected
        .then_some(DEFAULT_HIGHLIGHT_COLOR));
    color
        .map(|color| {
            ensure!(
                is_hex_color(color),
                "expected a CSS hex highlight color such as #2563eb"
            );
            Ok(format!("{HIGHLIGHT_PREFIX}{color}{HIGHLIGHT_SUFFIX}"))
        })
        .transpose()
}

fn is_hex_color(color: &str) -> bool {
    color.strip_prefix('#').is_some_and(|digits| {
        matches!(digits.len(), 3 | 4 | 6 | 8)
            && digits.bytes().all(|digit| digit.is_ascii_hexdigit())
    })
}

pub(super) fn annotate(
    source: &str,
    tokenizer: &Tokenizer<'_>,
    override_existing: bool,
    highlight: Option<&str>,
) -> Result<Option<String>> {
    let doc = parse_xml(source)?;
    let html = doc.root_element();
    ensure!(
        html.has_tag_name((XHTML, "html")),
        "expected an XHTML document"
    );
    let body = html
        .children()
        .find(|node| node.has_tag_name((XHTML, "body")))
        .context("XHTML document has no body")?;

    let chapter = Chapter {
        source,
        tokenizer,
        override_existing,
    };
    let mut edits = Vec::new();
    let annotated = chapter
        .annotate_children(body, false, &mut edits)?
        .annotated;
    if let Some(highlight) = highlight
        && (annotated || body.descendants().any(is_generated_ruby))
    {
        let head = html
            .children()
            .find(|node| node.has_tag_name((XHTML, "head")))
            .context("XHTML document has no head for the highlight stylesheet")?;
        let existing = head
            .children()
            .filter(|node| node.has_tag_name((XHTML, "style")))
            .find_map(|style| {
                let text = style.first_child()?;
                if !text.is_text() || text.next_sibling().is_some() {
                    return None;
                }
                let color = source
                    .get(text.range())?
                    .strip_prefix(HIGHLIGHT_PREFIX)?
                    .strip_suffix(HIGHLIGHT_SUFFIX)?;
                is_hex_color(color).then_some(text)
            });
        match existing {
            Some(text) if text.text() != Some(highlight) => {
                edits.push((text.range(), highlight.to_owned()));
            }
            Some(_) => {}
            None => {
                let start = closing_tag(source, head)?.start;
                let namespace = namespace_declaration(head);
                edits.push((
                    start..start,
                    format!("<style{namespace}>{highlight}</style>"),
                ));
            }
        }
    }

    if edits.is_empty() {
        return Ok(None);
    }
    Ok(Some(apply_edits(source, edits)?))
}

struct Chapter<'a> {
    source: &'a str,
    tokenizer: &'a Tokenizer<'a>,
    override_existing: bool,
}

#[derive(Default)]
struct Coverage {
    annotated: bool,
    unread_kanji: bool,
}

impl Chapter<'_> {
    fn annotate_children(
        &self,
        parent: Node<'_, '_>,
        replacing_ruby: bool,
        edits: &mut Vec<Edit>,
    ) -> Result<Coverage> {
        let mut coverage = Coverage::default();
        for node in parent.children() {
            if node.is_text() {
                let (ruby, unread_kanji) = self.annotate_text(
                    node.text().unwrap_or_default(),
                    namespace_declaration(parent),
                )?;
                coverage.unread_kanji |= unread_kanji;
                if let Some(ruby) = ruby {
                    // Adjacent text and CDATA can share a node with only the first source range.
                    let start = node.range().start;
                    let end = match node.next_sibling() {
                        Some(sibling) => sibling.range().start,
                        None => closing_tag(self.source, parent)?.start,
                    };
                    ensure!(
                        start >= parent.range().start
                            && end >= node.range().end
                            && end <= parent.range().end,
                        "cannot annotate XML text expanded from an entity"
                    );
                    edits.push((start..end, ruby));
                    coverage.annotated = true;
                }
                continue;
            }

            if node.tag_name().namespace() != Some(XHTML) {
                continue;
            }
            match node.tag_name().name() {
                "ruby" => {
                    if replacing_ruby {
                        coverage.unread_kanji = true;
                    } else if self.override_existing && !is_generated_ruby(node) {
                        let mut replacement = Vec::new();
                        let base = self.annotate_children(node, true, &mut replacement)?;
                        if base.annotated && !base.unread_kanji {
                            self.rename_to_span(node, &mut replacement)?;
                            edits.extend(replacement);
                            coverage.annotated = true;
                        }
                    }
                }
                "rt" | "rp" | "rtc" => {
                    if replacing_ruby {
                        edits.push((node.range(), String::new()));
                    }
                }
                "script" | "style" | "textarea" | "title" | "select" => {}
                name => {
                    if name == "rb" && replacing_ruby {
                        self.rename_to_span(node, edits)?;
                    }
                    let children = self.annotate_children(node, replacing_ruby, edits)?;
                    coverage.annotated |= children.annotated;
                    coverage.unread_kanji |= children.unread_kanji;
                }
            }
        }
        Ok(coverage)
    }

    fn rename_to_span(&self, node: Node<'_, '_>, edits: &mut Vec<Edit>) -> Result<()> {
        // Retain attributes (especially IDs) and any namespace declarations on the old base.
        let range = node.range();
        let name = self.source[range.start + 1..range.end]
            .split([' ', '\n', '\t', '\r', '/', '>'])
            .next()
            .unwrap_or_default();
        let end = range.start + 1 + name.len();
        let local_len = node.tag_name().name().len();
        edits.push((end - local_len..end, String::from("span")));
        if !self.source[range].ends_with("/>") {
            let end = closing_tag(self.source, node)?.start + 2 + name.len();
            edits.push((end - local_len..end, String::from("span")));
        }
        Ok(())
    }

    fn annotate_text(&self, mut text: &str, namespace: &str) -> Result<(Option<String>, bool)> {
        let total_kanji = text.chars().filter(|&ch| is_kanji(ch)).count();
        if total_kanji == 0 {
            return Ok((None, false));
        }

        let mut output = String::new();
        let mut annotated = 0;
        while !text.is_empty() {
            // Sudachi has a byte limit per input. Prefer sentence boundaries on large text nodes.
            let mut end = text.floor_char_boundary(16 * 1024);
            if end < text.len() {
                end = text[..end]
                    .rmatch_indices(['。', '！', '？', '\n'])
                    .next()
                    .map_or(end, |(index, mark)| index + mark.len());
            }
            let (chunk, rest) = text.split_at(end);
            text = rest;
            let morphemes = self.tokenizer.tokenize(chunk, Mode::C, false)?;
            let mut cursor = 0;
            for morpheme in morphemes.iter() {
                let start = morpheme.begin();
                let end = morpheme.end();
                if start < cursor || start >= end || morpheme.is_oov() {
                    continue;
                }
                let reading = hiragana(morpheme.reading_form());
                if let Some(parts) = kanji_readings(&chunk[start..end], &reading) {
                    for (range, reading) in parts {
                        let base = &chunk[start + range.start..start + range.end];
                        escape_text(&chunk[cursor..start + range.start], &mut output);
                        output.push_str(&format!("<ruby{namespace} class=\"{RUBY_CLASS}\">"));
                        escape_text(base, &mut output);
                        output.push_str("<rt>");
                        escape_text(reading, &mut output);
                        output.push_str("</rt></ruby>");
                        cursor = start + range.end;
                        annotated += base.chars().count();
                    }
                }
            }
            escape_text(&chunk[cursor..], &mut output);
        }

        Ok(((annotated > 0).then_some(output), annotated < total_kanji))
    }
}

fn namespace_declaration(parent: Node<'_, '_>) -> &'static str {
    if parent.lookup_namespace_uri(None) == Some(XHTML) {
        ""
    } else {
        " xmlns=\"http://www.w3.org/1999/xhtml\""
    }
}

fn is_generated_ruby(node: Node<'_, '_>) -> bool {
    node.has_tag_name((XHTML, "ruby"))
        && node.attribute("class").is_some_and(|classes| {
            classes
                .split_ascii_whitespace()
                .any(|class| class == RUBY_CLASS)
        })
}

fn is_kanji(ch: char) -> bool {
    ch.script() == Script::Han
}

fn hiragana(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            'ァ'..='ヶ' | 'ヽ'..='ヾ' => char::from_u32(ch as u32 - 0x60).unwrap(),
            _ => ch,
        })
        .collect()
}

fn kanji_readings<'a>(surface: &str, reading: &'a str) -> Option<Vec<(Range<usize>, &'a str)>> {
    let mut runs = Vec::new();
    let mut start = None;
    for (index, ch) in surface.char_indices() {
        if is_kanji(ch) {
            start.get_or_insert(index);
        } else if let Some(start) = start.take() {
            runs.push(start..index);
        }
    }
    if let Some(start) = start {
        runs.push(start..surface.len());
    }

    let first = runs.first()?;
    let last = runs.last()?;
    if !reading
        .chars()
        .all(|ch| matches!(ch, 'ぁ'..='ゖ' | 'ゝ' | 'ゞ' | 'ー'))
    {
        return None;
    }
    let mut rest = reading
        .strip_prefix(hiragana(&surface[..first.start]).as_str())?
        .strip_suffix(hiragana(&surface[last.end..]).as_str())?;

    let mut readings = Vec::new();
    for pair in runs.windows(2) {
        let anchor = hiragana(&surface[pair[0].end..pair[1].start]);
        // Infer internal okurigana boundaries only when the kana match is unambiguous.
        let mut matches = rest
            .match_indices(&anchor)
            .filter(|(index, _)| *index > 0 && index + anchor.len() < rest.len());
        let (index, _) = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        readings.push((pair[0].clone(), &rest[..index]));
        rest = &rest[index + anchor.len()..];
    }
    if rest.is_empty() {
        return None;
    }
    readings.push((last.clone(), rest));
    Some(readings)
}

fn escape_text(text: &str, output: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\r' => output.push_str("&#13;"),
            _ => output.push(ch),
        }
    }
}

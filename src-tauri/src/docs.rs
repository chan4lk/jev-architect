//! Document parsing and sectioning (FR-6, FR-7).
//!
//! Converts an uploaded `.pdf` / `.docx` / `.md` / `.txt` file into an ordered
//! list of [`Section`]s with stable ids (`S1..Sn`), using headings where the
//! format has them (Markdown ATX headings, DOCX `Heading*` paragraph styles)
//! and greedily-packed blank-line-delimited blocks otherwise (plain text, and
//! text extracted from a PDF). Any section that ends up longer than
//! [`TARGET_SECTION_TOKENS`] is split further, first at paragraph boundaries
//! and then at sentence boundaries, so the pipeline's budget fitting (FR-7)
//! never has to reason about an oversized section.
//!
//! See `.specclaw/changes/001-bistec-architect-agent/design.md` for the
//! `docs.rs` role in the architecture.

use std::fs;
use std::io::{Cursor, Read};
use std::panic;
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The target size (in estimated tokens, see [`estimate_tokens`]) for a
/// section produced by the small-path evidence-size cutoff and by the
/// oversized-section splitter (A4).
pub const TARGET_SECTION_TOKENS: usize = 1500;

/// A contiguous piece of a parsed document, in document order.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Section {
    /// Stable id, `"S1".."Sn"`, assigned in document order after splitting.
    pub id: String,
    /// The heading that introduced this section, if the source format has
    /// headings and one was found (Markdown ATX heading, DOCX `Heading*`
    /// paragraph style).
    pub heading: Option<String>,
    /// The section's text content.
    pub text: String,
    /// `estimate_tokens(text)`.
    pub tokens: usize,
}

/// Errors from [`parse_document`] / [`parse_bytes`].
#[derive(Error, Debug)]
pub enum DocError {
    /// The file extension isn't one of `pdf`, `docx`, `md`/`markdown`, `txt`.
    #[error("unsupported document type: {0}")]
    Unsupported(String),
    /// A PDF was parsed but contained no extractable text (e.g. a scanned,
    /// image-only PDF). OCR is out of scope for v1.
    #[error("This PDF has no text layer (OCR is not supported in v1)")]
    NoTextLayer,
    /// The document produced no non-blank sections at all.
    #[error("document has no extractable content")]
    Empty,
    /// Reading the file from disk failed.
    #[error("io error: {0}")]
    Io(String),
    /// The document's content could not be parsed (corrupt DOCX/zip/XML, or
    /// a PDF parser failure).
    #[error("failed to parse document: {0}")]
    Parse(String),
}

/// Estimates a token count from a character count: `ceil(chars / 4)` (A4).
pub fn estimate_tokens(s: &str) -> usize {
    let chars = s.chars().count();
    chars.div_ceil(4)
}

/// Sums [`Section::tokens`] across `sections`.
pub fn total_tokens(sections: &[Section]) -> usize {
    sections.iter().map(|s| s.tokens).sum()
}

/// Reads `path` and dispatches to [`parse_bytes`] by its extension
/// (case-insensitive): `pdf`, `docx`, `md`/`markdown`, `txt`.
pub fn parse_document(path: &Path) -> Result<Vec<Section>, DocError> {
    let bytes = fs::read(path).map_err(|e| DocError::Io(e.to_string()))?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    parse_bytes(file_name, &bytes)
}

/// Parses `bytes` as a document, dispatching by `file_name`'s extension
/// (case-insensitive): `pdf`, `docx`, `md`/`markdown`, `txt`. Returns
/// [`DocError::Unsupported`] for any other extension.
pub fn parse_bytes(file_name: &str, bytes: &[u8]) -> Result<Vec<Section>, DocError> {
    let ext = Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();

    let raw = match ext.as_str() {
        "pdf" => parse_pdf(bytes)?,
        "docx" => parse_docx(bytes)?,
        "md" | "markdown" => parse_markdown(bytes),
        "txt" => parse_txt(bytes),
        other => return Err(DocError::Unsupported(other.to_string())),
    };

    finalize_sections(raw)
}

// ---------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------

fn parse_pdf(bytes: &[u8]) -> Result<Vec<(Option<String>, String)>, DocError> {
    let owned = bytes.to_vec();
    let extracted = panic::catch_unwind(move || pdf_extract::extract_text_from_mem(&owned));

    let text = match extracted {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => return Err(DocError::Parse(format!("PDF extraction failed: {e}"))),
        Err(_) => {
            return Err(DocError::Parse(
                "PDF extraction panicked while reading the file".to_string(),
            ))
        }
    };

    if text.trim().is_empty() {
        return Err(DocError::NoTextLayer);
    }

    Ok(pack_blocks(&text))
}

// ---------------------------------------------------------------------
// DOCX
// ---------------------------------------------------------------------

fn parse_docx(bytes: &[u8]) -> Result<Vec<(Option<String>, String)>, DocError> {
    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| DocError::Parse(format!("invalid DOCX (not a zip): {e}")))?;

    let mut xml = String::new();
    {
        let mut file = archive
            .by_name("word/document.xml")
            .map_err(|e| DocError::Parse(format!("DOCX missing word/document.xml: {e}")))?;
        file.read_to_string(&mut xml)
            .map_err(|e| DocError::Io(e.to_string()))?;
    }

    parse_docx_xml(&xml)
}

/// Reads `word/document.xml`: paragraphs are `w:p`, paragraph text is the
/// concatenation of its `w:t` runs, and a paragraph whose `w:pPr/w:pStyle`
/// `w:val` starts with `"Heading"` (case-insensitive) starts a new section
/// whose heading is that paragraph's own text.
fn parse_docx_xml(xml: &str) -> Result<Vec<(Option<String>, String)>, DocError> {
    let mut reader = Reader::from_str(xml);

    let mut sections: Vec<(Option<String>, String)> = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut body_paragraphs: Vec<String> = Vec::new();

    let mut para_style: Option<String> = None;
    let mut para_text = String::new();
    let mut in_text = false;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| DocError::Parse(format!("invalid DOCX XML: {e}")))?;

        match event {
            Event::Eof => break,
            Event::Start(e) => match e.local_name().as_ref() {
                b"p" => {
                    para_style = None;
                    para_text.clear();
                }
                b"pStyle" => {
                    para_style = attr_local_value(&e, b"val");
                }
                b"t" => {
                    in_text = true;
                }
                _ => {}
            },
            Event::Empty(e) => {
                if e.local_name().as_ref() == b"pStyle" {
                    para_style = attr_local_value(&e, b"val");
                }
            }
            Event::Text(t) => {
                if in_text {
                    para_text.push_str(&t.unescape().unwrap_or_default());
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"t" => {
                    in_text = false;
                }
                b"p" => {
                    let text = para_text.trim().to_string();
                    let is_heading = para_style
                        .as_deref()
                        .map(|s| s.trim().to_lowercase().starts_with("heading"))
                        .unwrap_or(false);

                    if is_heading {
                        sections.push((current_heading.take(), body_paragraphs.join("\n\n")));
                        body_paragraphs.clear();
                        current_heading = Some(text);
                    } else if !text.is_empty() {
                        body_paragraphs.push(text);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    sections.push((current_heading, body_paragraphs.join("\n\n")));
    Ok(sections)
}

/// Looks up an attribute by its local name (ignoring any namespace prefix),
/// e.g. `w:val` matches `local == b"val"`.
fn attr_local_value(e: &BytesStart<'_>, local: &[u8]) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key.local_name().as_ref() == local {
            a.unescape_value().ok().map(|v| v.into_owned())
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------

/// Splits at ATX headings (`#`..`######`). The heading text becomes the
/// section's `heading`; content before the first heading is a section with
/// `heading: None`.
fn parse_markdown(bytes: &[u8]) -> Vec<(Option<String>, String)> {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let heading_re = Regex::new(r"^(#{1,6})\s+(.*)$").expect("valid regex");

    let mut sections: Vec<(Option<String>, String)> = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut buffer: Vec<&str> = Vec::new();

    for line in text.lines() {
        if let Some(caps) = heading_re.captures(line) {
            sections.push((current_heading.take(), buffer.join("\n")));
            buffer.clear();
            let heading_text = caps
                .get(2)
                .map(|m| m.as_str())
                .unwrap_or_default()
                .trim_end_matches('#')
                .trim()
                .to_string();
            current_heading = Some(heading_text);
        } else {
            buffer.push(line);
        }
    }
    sections.push((current_heading, buffer.join("\n")));

    sections
}

// ---------------------------------------------------------------------
// Plain text (and PDF-extracted text)
// ---------------------------------------------------------------------

/// Blocks separated by blank lines, greedily packed into sections up to
/// [`TARGET_SECTION_TOKENS`].
fn parse_txt(bytes: &[u8]) -> Vec<(Option<String>, String)> {
    let text = String::from_utf8_lossy(bytes).into_owned();
    pack_blocks(&text)
}

fn pack_blocks(text: &str) -> Vec<(Option<String>, String)> {
    pack_into_chunks(&split_blocks(text), "\n\n")
        .into_iter()
        .map(|chunk| (None, chunk))
        .collect()
}

/// Splits `text` into blocks of consecutive non-blank lines, dropping blank
/// (whitespace-only) separator lines.
fn split_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }

    blocks
}

/// Greedily packs `items` (already-trimmed, non-blank strings) into chunks
/// joined by `sep`, never adding an item that would push a non-empty chunk
/// over [`TARGET_SECTION_TOKENS`]. A single item that is itself oversized is
/// still emitted as its own (oversized) chunk — the caller is expected to
/// split it further if needed.
fn pack_into_chunks(items: &[String], sep: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for item in items {
        let candidate = if current.is_empty() {
            item.clone()
        } else {
            format!("{current}{sep}{item}")
        };

        if !current.is_empty() && estimate_tokens(&candidate) > TARGET_SECTION_TOKENS {
            chunks.push(current);
            current = item.clone();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

// ---------------------------------------------------------------------
// Oversized-section splitting and final assembly
// ---------------------------------------------------------------------

/// If `text` is within budget, returns it unchanged. Otherwise splits it
/// further, first at paragraph (blank-line) boundaries, then at sentence
/// boundaries if a resulting chunk is still too long. The heading is kept on
/// the first chunk; later chunks get `"<heading> (cont.)"` (or stay
/// headingless if the section had no heading).
fn split_oversized(heading: Option<String>, text: String) -> Vec<(Option<String>, String)> {
    if estimate_tokens(&text) <= TARGET_SECTION_TOKENS {
        return vec![(heading, text)];
    }

    let paragraphs = split_blocks(&text);
    let paragraph_chunks = if paragraphs.len() > 1 {
        pack_into_chunks(&paragraphs, "\n\n")
    } else {
        vec![text]
    };

    let mut chunks: Vec<String> = Vec::new();
    for chunk in paragraph_chunks {
        if estimate_tokens(&chunk) > TARGET_SECTION_TOKENS {
            let sentences = split_sentences(&chunk);
            if sentences.len() > 1 {
                chunks.extend(pack_into_chunks(&sentences, " "));
            } else {
                chunks.push(chunk);
            }
        } else {
            chunks.push(chunk);
        }
    }

    chunks
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let h = match (&heading, i) {
                (Some(h), 0) => Some(h.clone()),
                (Some(h), _) => Some(format!("{h} (cont.)")),
                (None, _) => None,
            };
            (h, t)
        })
        .collect()
}

/// Splits `text` into sentences at `.`/`!`/`?` followed by whitespace (or end
/// of text). Best-effort: a chunk with no sentence-ending punctuation comes
/// back as a single "sentence".
fn split_sentences(text: &str) -> Vec<String> {
    let re = Regex::new(r"(?s)(.*?[.!?])(?:\s+|$)").expect("valid regex");

    let mut sentences = Vec::new();
    let mut last_end = 0;
    for caps in re.captures_iter(text) {
        let m = caps.get(1).expect("group 1 always matches");
        sentences.push(m.as_str().to_string());
        last_end = caps.get(0).expect("whole match").end();
    }

    let rest = text[last_end..].trim();
    if !rest.is_empty() {
        sentences.push(rest.to_string());
    }

    sentences
}

/// Applies oversized-section splitting to every raw `(heading, text)` pair,
/// drops any resulting section whose text is blank, assigns ids `S1..Sn` in
/// document order, and computes `tokens`. Returns [`DocError::Empty`] if
/// nothing is left.
fn finalize_sections(raw: Vec<(Option<String>, String)>) -> Result<Vec<Section>, DocError> {
    let mut sections: Vec<(Option<String>, String)> = Vec::new();

    for (heading, text) in raw {
        let trimmed = text.trim().to_string();
        for (h, t) in split_oversized(heading, trimmed) {
            let t = t.trim().to_string();
            if !t.is_empty() {
                sections.push((h, t));
            }
        }
    }

    if sections.is_empty() {
        return Err(DocError::Empty);
    }

    Ok(sections
        .into_iter()
        .enumerate()
        .map(|(i, (heading, text))| {
            let tokens = estimate_tokens(&text);
            Section {
                id: format!("S{}", i + 1),
                heading,
                text,
                tokens,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_rounds_up() {
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
    }

    #[test]
    fn unsupported_extension_is_rejected() {
        let err = parse_bytes("sample.rtf", b"hello").unwrap_err();
        assert!(matches!(err, DocError::Unsupported(ext) if ext == "rtf"));
    }

    #[test]
    fn empty_document_is_rejected() {
        let err = parse_bytes("sample.txt", b"   \n\n  ").unwrap_err();
        assert!(matches!(err, DocError::Empty));
    }

    #[test]
    fn markdown_splits_at_headings() {
        let md = b"# Title\n\nIntro text.\n\n## Section Two\n\nMore text.\n";
        let sections = parse_bytes("sample.md", md).unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].id, "S1");
        assert_eq!(sections[0].heading.as_deref(), Some("Title"));
        assert_eq!(sections[1].heading.as_deref(), Some("Section Two"));
    }
}

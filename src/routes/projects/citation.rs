use serde_json::Value;

/// A citation format served as its own HTTP response, in a content type the Zotero
/// Connector imports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bibtex,
    Ris,
}

impl Format {
    fn content_type(self) -> &'static str {
        match self {
            Format::Bibtex => "application/x-bibtex",
            Format::Ris => "application/x-research-info-systems",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Format::Bibtex => "bib",
            Format::Ris => "ris",
        }
    }
}

/// A citation as a downloadable file.
pub struct CitationFile {
    pub content_type: &'static str,
    pub filename: String,
    pub body: String,
}

/// The citation of a project as a file, or `None` when the project carries no BibTeX.
pub fn citation_file(citation: Option<&Value>, slug: &str, format: Format) -> Option<CitationFile> {
    let bibtex = citation?.get("bibtex")?.as_str()?.trim();
    if bibtex.is_empty() {
        return None;
    }

    let body = match format {
        Format::Bibtex => bibtex.to_string(),
        Format::Ris => bibtex_to_ris(bibtex),
    };

    Some(CitationFile {
        content_type: format.content_type(),
        filename: format!("{}.{}", slug, format.extension()),
        body,
    })
}

/// The RIS tag each BibTeX entry type maps to.
fn ris_type(entry_type: &str) -> &'static str {
    match entry_type {
        "article" => "JOUR",
        "inproceedings" | "conference" => "CPAPER",
        "book" => "BOOK",
        "incollection" => "CHAP",
        "phdthesis" | "mastersthesis" => "THES",
        "techreport" => "RPRT",
        _ => "GEN",
    }
}

/// The fields carried over, in the order they are written.
const FIELDS: [(&str, &str); 11] = [
    ("title", "TI"),
    ("journal", "JO"),
    ("booktitle", "T2"),
    ("year", "PY"),
    ("volume", "VL"),
    ("number", "IS"),
    ("pages", "SP"),
    ("doi", "DO"),
    ("url", "UR"),
    ("publisher", "PB"),
    ("abstract", "AB"),
];

fn bibtex_to_ris(bibtex: &str) -> String {
    let mut lines = vec![format!("TY  - {}", ris_type(&entry_type(bibtex)))];

    if let Some(authors) = field(bibtex, "author") {
        for author in split_authors(&authors) {
            lines.push(format!("AU  - {}", author));
        }
    }

    for (name, tag) in FIELDS {
        if let Some(value) = field(bibtex, name) {
            lines.push(format!("{}  - {}", tag, value));
        }
    }

    lines.push("ER  - ".to_string());
    lines.join("\n")
}

/// The entry type of `@article{key, ...}`, lowercased.
fn entry_type(bibtex: &str) -> String {
    let after_at = match bibtex.find('@') {
        Some(at) => &bibtex[at + 1..],
        None => return String::new(),
    };
    after_at
        .split('{')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// BibTeX joins authors with a bare `and`.
fn split_authors(authors: &str) -> Vec<String> {
    let normalized = authors.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = Vec::new();
    for part in normalized.split(" and ") {
        let name = part.trim();
        if !name.is_empty() {
            out.push(name.to_string());
        }
    }
    out
}

/// The value of `name = {...}` or `name = "..."`, taken to the first closing delimiter.
/// The name must stand on its own, so `url` is not read out of `howpublished`.
fn field(bibtex: &str, name: &str) -> Option<String> {
    let lowered = bibtex.to_ascii_lowercase();
    let bytes = lowered.as_bytes();
    let mut from = 0;
    while let Some(offset) = lowered[from..].find(name) {
        let start = from + offset;
        let end = start + name.len();
        from = end;

        let preceded_by_name = start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
        if preceded_by_name {
            continue;
        }

        let mut rest = bibtex[end..].trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        rest = rest[1..].trim_start();

        let closing = match rest.chars().next() {
            Some('{') => '}',
            Some('"') => '"',
            _ => continue,
        };
        let value = &rest[1..];
        let Some(close_at) = value.find(closing) else {
            continue;
        };
        return Some(value[..close_at].trim().to_string());
    }
    None
}

#[cfg(test)]
#[path = "tests/citation.rs"]
mod tests;

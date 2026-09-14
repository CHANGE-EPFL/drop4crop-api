use super::*;
use serde_json::json;

const BIBTEX: &str = r#"@article{thomas2026drop4crop,
  author = {Thomas, Evan and Doe, Jane},
  title = {Crop water use over the twenty-first century},
  journal = {Nature Water},
  year = {2026},
  volume = {4},
  doi = {10.1038/s44221-026-00001-2},
  url = {https://doi.org/10.1038/s44221-026-00001-2}
}"#;

fn citation() -> serde_json::Value {
    json!({ "text": "Thomas et al. (2026)", "bibtex": BIBTEX })
}

#[test]
fn test_ris_is_served_as_the_content_type_the_connector_imports() {
    let file = citation_file(Some(&citation()), "crop-water-use", Format::Ris).unwrap();

    assert_eq!(file.content_type, "application/x-research-info-systems");
    assert_eq!(file.filename, "crop-water-use.ris");
}

#[test]
fn test_bibtex_is_served_as_the_content_type_the_connector_imports() {
    let file = citation_file(Some(&citation()), "crop-water-use", Format::Bibtex).unwrap();

    assert_eq!(file.content_type, "application/x-bibtex");
    assert_eq!(file.filename, "crop-water-use.bib");
    assert_eq!(file.body, BIBTEX);
}

#[test]
fn test_ris_carries_every_field_the_bibtex_entry_declares() {
    let file = citation_file(Some(&citation()), "crop-water-use", Format::Ris).unwrap();
    let lines: Vec<&str> = file.body.lines().collect();

    assert_eq!(lines[0], "TY  - JOUR");
    assert!(lines.contains(&"AU  - Thomas, Evan"));
    assert!(lines.contains(&"AU  - Doe, Jane"));
    assert!(lines.contains(&"TI  - Crop water use over the twenty-first century"));
    assert!(lines.contains(&"JO  - Nature Water"));
    assert!(lines.contains(&"PY  - 2026"));
    assert!(lines.contains(&"VL  - 4"));
    assert!(lines.contains(&"DO  - 10.1038/s44221-026-00001-2"));
    assert!(lines.contains(&"UR  - https://doi.org/10.1038/s44221-026-00001-2"));
    assert_eq!(*lines.last().unwrap(), "ER  - ");
}

#[test]
fn test_an_unknown_entry_type_falls_back_to_a_generic_record() {
    let citation = json!({ "bibtex": "@dataset{d, title = {A raster}}" });
    let file = citation_file(Some(&citation), "india", Format::Ris).unwrap();

    assert_eq!(file.body.lines().next().unwrap(), "TY  - GEN");
}

#[test]
fn test_a_field_is_not_read_out_of_a_longer_field_name() {
    let citation = json!({ "bibtex": "@misc{m, howpublished = {Online}, url = {https://example.org}}" });
    let file = citation_file(Some(&citation), "india", Format::Ris).unwrap();

    assert!(file.body.lines().any(|line| line == "UR  - https://example.org"));
}

#[test]
fn test_a_project_without_bibtex_has_no_citation_file() {
    assert!(citation_file(None, "india", Format::Ris).is_none());
    assert!(citation_file(Some(&json!({ "text": "Thomas et al." })), "india", Format::Ris).is_none());
    assert!(citation_file(Some(&json!({ "bibtex": "  " })), "india", Format::Bibtex).is_none());
}

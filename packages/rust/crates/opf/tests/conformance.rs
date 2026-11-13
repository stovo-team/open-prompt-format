use open_prompt_format::{load, load_by_id, load_collection, parse, RenderedMessage, Role};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize)]
struct ConformanceSuite {
    valid: Vec<ValidCase>,
    invalid: Vec<String>,
}

#[derive(Deserialize)]
struct ValidCase {
    file: String,
    inputs: HashMap<String, String>,
    expected: Vec<RenderedMessage>,
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("..")
        .join("fixtures")
}

#[test]
fn conformance_valid_and_invalid() {
    let fdir = fixtures_dir();
    let raw = fs::read_to_string(fdir.join("conformance.json")).unwrap();
    let suite: ConformanceSuite = serde_json::from_str(&raw).unwrap();

    for tc in suite.valid {
        let p = load(fdir.join(&tc.file)).expect(&tc.file);
        let got = p.render(&tc.inputs).expect(&tc.file);
        assert_eq!(got, tc.expected, "mismatch for {}", tc.file);
        assert!(p.source_digest.starts_with("sha256:"));
        assert_eq!(p.source_digest.len(), 7 + 64);
    }

    for rel in suite.invalid {
        let res = load(fdir.join(&rel));
        assert!(res.is_err(), "expected error for invalid fixture {rel}");
    }
}

#[test]
fn newline_normalization_digest() {
    let lf = "---\nformat: opf/0.2\nid: norm.test\n---\n\n## user\nHello\n";
    let crlf = lf.replace('\n', "\r\n");
    let cr = lf.replace('\n', "\r");

    let p_lf = parse(lf, None).unwrap();
    let p_crlf = parse(&crlf, None).unwrap();
    let p_cr = parse(&cr, None).unwrap();

    assert_eq!(p_lf.source_digest, p_crlf.source_digest);
    assert_eq!(p_lf.source_digest, p_cr.source_digest);
}

#[test]
fn collection_lookup_and_duplicate_rejection() {
    let tmp = std::env::temp_dir().join(format!("opf-rust-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();

    let a = "---\nformat: opf/0.2\nid: alpha.one\n---\n\n## user\nA\n";
    let b = "---\nformat: opf/0.2\nid: beta.two\n---\n\n## user\nB\n";
    fs::write(tmp.join("a.md"), a).unwrap();
    fs::write(tmp.join("b.md"), b).unwrap();
    fs::write(tmp.join("notes.md"), "# Not OPF\n").unwrap();

    let found = load_by_id("beta.two", &tmp).unwrap();
    assert_eq!(found.metadata.id, "beta.two");
    let rendered = found.render(&HashMap::new()).unwrap();
    assert_eq!(
        rendered,
        vec![RenderedMessage {
            role: Role::User,
            content: "B".to_string()
        }]
    );

    fs::write(tmp.join("dup.md"), a).unwrap();
    assert!(load_collection(&tmp).is_err());

    let _ = fs::remove_dir_all(&tmp);
}

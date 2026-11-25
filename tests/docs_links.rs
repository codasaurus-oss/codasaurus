use std::fs;
use std::path::Path;

fn manifest_dir() -> &'static str {
    env!("CARGO_MANIFEST_DIR")
}

fn read(rel: &str) -> String {
    fs::read_to_string(Path::new(manifest_dir()).join(rel))
        .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

const TOKEN_DELIMS: &[u8] = b" \t\n\r\"'<>()[]`";

fn relative_docs_refs(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut refs = Vec::new();
    let mut from = 0;
    while let Some(hit) = text[from..].find("docs/") {
        let pos = from + hit;
        let mut lo = pos;
        while lo > 0 && !TOKEN_DELIMS.contains(&bytes[lo - 1]) {
            lo -= 1;
        }
        let mut hi = pos + 5;
        while hi < bytes.len() && !TOKEN_DELIMS.contains(&bytes[hi]) {
            hi += 1;
        }
        let token = &text[lo..hi];
        if !token.starts_with("http") {
            refs.push(token.to_owned());
        }
        from = pos + 5;
    }
    refs
}

const DOCS_BASE: &str = "https://codasaurus-oss.github.io/";
const RETIRED_DOCS_BASE: &str = "https://codasaurus-oss.github.io/docs/";

#[test]
fn dead_docs_links_resolve() {
    for file in ["README.md", "src/main.rs"] {
        let text = read(file);
        let refs = relative_docs_refs(&text);
        assert!(
            refs.is_empty(),
            "{file} still has relative docs/... links: {refs:?}"
        );
        assert!(
            text.contains(DOCS_BASE),
            "{file} must link the published docs base {DOCS_BASE}"
        );
        assert!(
            !text.contains(RETIRED_DOCS_BASE),
            "{file} still links the retired {RETIRED_DOCS_BASE} URLs"
        );
    }

    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("cargo clippy --locked --all-targets -- -D warnings"),
        "ci.yml must run `cargo clippy --locked --all-targets -- -D warnings`"
    );

    let cargo_toml = read("Cargo.toml");
    assert!(
        cargo_toml.contains("[lints.rust]"),
        "Cargo.toml must declare [lints.rust]"
    );
    assert!(
        cargo_toml.contains("warnings = \"deny\""),
        "Cargo.toml must set warnings = \"deny\" under [lints.rust]"
    );
}

//! The public guest invite page (`GET /api/rooms/guest`).
//!
//! `ui/guest.html` is the markup, `ui/src/guest/guest-page.js` the page script
//! and `ui/src/guest/guest-catalog.generated.json` its translations (written by
//! `ui/scripts/sync-guest-catalog.mjs`). The page picks its language from the
//! browser's `navigator.languages`, so the backend serves one document: the
//! template with the catalog and the script inlined, exactly as
//! `ui/scripts/guest-page.mjs` assembles it.

use std::sync::LazyLock;

const TEMPLATE: &str = include_str!("../../ui/guest.html");
const SCRIPT: &str = include_str!("../../ui/src/guest/guest-page.js");
const CATALOG: &str = include_str!("../../ui/src/guest/guest-catalog.generated.json");

const CATALOG_TAG: &str = r#"<script type="application/json" id="rooms-guest-catalog"></script>"#;
const SCRIPT_TAG: &str = r#"<script src="src/guest/guest-page.js"></script>"#;

/// The assembled guest page.
pub static GUEST_HTML: LazyLock<String> = LazyLock::new(|| assemble(TEMPLATE, SCRIPT, CATALOG));

fn assemble(template: &str, script: &str, catalog: &str) -> String {
    assert!(
        template.contains(CATALOG_TAG),
        "guest.html lost its catalog tag"
    );
    assert!(
        template.contains(SCRIPT_TAG),
        "guest.html lost its script tag"
    );
    // `<` cannot occur outside JSON strings, where its JSON unicode escape is equivalent; it
    // keeps a translation from closing the `<script>` element.
    let catalog = catalog.trim().replace('<', "\\u003c");
    template
        .replacen(
            CATALOG_TAG,
            &format!(
                r#"<script type="application/json" id="rooms-guest-catalog">{catalog}</script>"#
            ),
            1,
        )
        .replacen(SCRIPT_TAG, &format!("<script>\n{script}</script>"), 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inlines_the_script_and_catalog() {
        let html = GUEST_HTML.as_str();
        assert!(!html.contains(SCRIPT_TAG));
        assert!(!html.contains(CATALOG_TAG));
        assert!(html.contains(r#"id="rooms-guest-catalog">{"#));
        assert!(html.contains("navigator.languages"));
        let catalog: serde_json::Value = serde_json::from_str(CATALOG).expect("catalog JSON");
        assert!(catalog.is_object());
    }

    #[test]
    fn script_and_catalog_cannot_close_their_elements() {
        assert!(!SCRIPT.to_ascii_lowercase().contains("</script"));
        assert!(!SCRIPT.contains("<!--"));
        let html = assemble(TEMPLATE, "", r#"{"ar":{"a":"</script><b>"}}"#);
        let lt = concat!("\\", "u003c");
        assert!(html.contains(&format!(r#"{{"ar":{{"a":"{lt}/script>{lt}b>"}}}}"#)));
        assert!(!html.contains("</script><b>"));
    }
}

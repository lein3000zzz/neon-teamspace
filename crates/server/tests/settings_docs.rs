//! The committed settings reference must match the page the registry generates, so the
//! docs list exactly the settings the server reads.

use std::path::Path;

use neon_app::settings::registry;

#[test]
fn committed_settings_reference_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/src/content/docs/reference/settings.md");
    let committed = std::fs::read_to_string(&path).expect("the settings reference is readable");

    assert!(
        committed == registry().reference_markdown(),
        "docs/src/content/docs/reference/settings.md is stale. Regenerate it with:\n\n    \
         make docs-gen\n"
    );
}

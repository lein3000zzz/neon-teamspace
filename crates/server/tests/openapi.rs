//! The committed description must match the one the routes generate, so clients built
//! from `api/openapi.json` never drift from the server.

use std::path::Path;

#[test]
fn committed_description_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../api/openapi.json");
    let committed = std::fs::read_to_string(&path).expect("api/openapi.json is readable");

    assert!(
        committed == neon_server::http::spec_json(),
        "api/openapi.json is stale. Regenerate it with:\n\n    \
         cargo run -p neon-server -- openapi --out api/openapi.json\n"
    );
}

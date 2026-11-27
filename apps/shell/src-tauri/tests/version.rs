//! The apps carry the server's version, as a release builds both from one tag.

#[test]
fn the_apps_version_is_the_servers() {
    let root = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Cargo.toml"))
        .expect("the root Cargo.toml");
    let version = root
        .lines()
        .skip_while(|line| line.trim() != "[workspace.package]")
        .find_map(|line| line.strip_prefix("version = "))
        .map(|version| version.trim_matches('"'));
    assert_eq!(version, Some(env!("CARGO_PKG_VERSION")));
}

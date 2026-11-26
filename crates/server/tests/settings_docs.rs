//! The committed settings reference must match the page the registry generates, so the
//! docs list exactly the settings the server reads.

use std::path::Path;

use neon_app::settings::registry;
use neon_domain::settings::Value;

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

/// The settings `neon.example.yaml` shows under `settings:`, commented out or not, by key,
/// with the value each shows: a line `key: value`, or `# key: value`, nested by the column
/// its key, or its `#`, starts at.
fn example_settings(example: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut path: Vec<(usize, String)> = Vec::new();
    let mut inside = false;
    for line in example.lines() {
        if line == "settings:" {
            inside = true;
            continue;
        }
        if !inside || line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('#') {
            break;
        }
        let column = line.len() - line.trim_start().len();
        let text = line.trim_start();
        let text = text.strip_prefix("# ").unwrap_or(text);
        let Some((key, value)) = text.split_once(':') else {
            continue;
        };
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            continue;
        }
        if !(value.is_empty() || value.starts_with(' ')) {
            continue;
        }
        path.retain(|(at, _)| *at < column);
        let value = value.trim();
        let mut key_path: Vec<&str> = path.iter().map(|(_, key)| key.as_str()).collect();
        key_path.push(key);
        found.push((key_path.join("."), value.to_owned()));
        if value.is_empty() {
            path.push((column, key.to_owned()));
        }
    }
    found
}

#[test]
fn the_example_config_shows_every_setting_with_its_default() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../neon.example.yaml");
    let example = std::fs::read_to_string(&path).expect("the example config is readable");
    let shown = example_settings(&example);

    for def in registry().definitions() {
        let Some((_, value)) = shown.iter().find(|(key, _)| key == def.key()) else {
            panic!("neon.example.yaml lacks settings.{}", def.key());
        };
        let value = value.trim_start_matches('[').trim_end_matches(']');
        // A secret, and a string empty by default, show no value.
        let default = match def.default_value() {
            Value::Secret(_) => String::new(),
            other => other.to_string(),
        };
        assert_eq!(
            value,
            default,
            "neon.example.yaml shows settings.{} as {value:?}, but its default is {default:?}",
            def.key()
        );
    }
}

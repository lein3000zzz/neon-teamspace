//! The YAML config file as a configuration layer.
//!
//! figment's own YAML support is built on `serde_yaml`, which is no longer maintained, so
//! the file is parsed with `serde-saphyr` and handed to figment here.

use std::path::PathBuf;

use figment::value::{Dict, Map, Value};
use figment::{Error, Metadata, Profile, Provider};

pub(super) struct YamlFile {
    path: PathBuf,
}

impl YamlFile {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Provider for YamlFile {
    fn metadata(&self) -> Metadata {
        // Errors name keys as they are written in the file.
        Metadata::from("YAML file", self.path.as_path())
            .interpolater(|_: &Profile, keys: &[&str]| keys.join("."))
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        // figment adds the file's path to these errors.
        let text = std::fs::read_to_string(&self.path)
            .map_err(|error| Error::from(format!("cannot read it: {error}")))?;
        let dict = parse(&text).map_err(|error| Error::from(error.to_string()))?;
        Ok(Profile::Default.collect(dict))
    }
}

/// A file that is empty or holds only comments is an empty configuration. Errors give
/// the line and column; figment prints them on one line, so without a source snippet.
///
/// Booleans are YAML 1.2's: `true` and `false`, also written `True`, `TRUE`, `False` or
/// `FALSE`. The YAML 1.1 spellings, such as `on`, `off`, `yes` and `no`, are strings, so
/// `mode: off` names the mode `off`.
fn parse(text: &str) -> Result<Dict, serde_saphyr::Error> {
    let options = serde_saphyr::options! { with_snippet: false, strict_booleans: true };
    let dict: Option<Dict> = serde_saphyr::from_str_with_options(text, options)?;
    Ok(drop_unset(dict.unwrap_or_default()))
}

/// Drops keys without a value, so `url:` with nothing after it, or a section whose keys
/// are all commented out, counts as unset instead of as a value of the wrong type.
fn drop_unset(dict: Dict) -> Dict {
    dict.into_iter()
        .filter_map(|(key, value)| match value {
            Value::Empty(..) => None,
            Value::Dict(tag, inner) => Some((key, Value::Dict(tag, drop_unset(inner)))),
            other => Some((key, other)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_maps_become_dicts() {
        let dict = parse("server:\n  listen: 127.0.0.1:9000\nflag: true\n").expect("valid YAML");

        let server = dict["server"].as_dict().expect("a mapping");
        assert_eq!(server["listen"].as_str(), Some("127.0.0.1:9000"));
        assert_eq!(dict["flag"].to_bool(), Some(true));
    }

    #[test]
    fn only_yaml_1_2_spellings_are_booleans() {
        let dict = parse("on: true\noff: false\nup: True\ndown: FALSE\nmode: off\nanswer: yes\n")
            .expect("valid YAML");

        assert_eq!(dict["on"].to_bool(), Some(true));
        assert_eq!(dict["off"].to_bool(), Some(false));
        assert_eq!(dict["up"].to_bool(), Some(true));
        assert_eq!(dict["down"].to_bool(), Some(false));
        assert_eq!(dict["mode"].as_str(), Some("off"));
        assert_eq!(dict["answer"].as_str(), Some("yes"));
    }

    #[test]
    fn empty_files_are_empty_configurations() {
        assert_eq!(parse("").expect("empty"), Dict::new());
        assert_eq!(parse("# only a comment\n").expect("comments"), Dict::new());
    }

    #[test]
    fn keys_without_values_are_unset() {
        let dict = parse("server:\n  listen: ':80'\n  public_url:\nsettings:\n  instance:\n")
            .expect("valid YAML");

        let server = dict["server"].as_dict().expect("a mapping");
        assert_eq!(server.keys().collect::<Vec<_>>(), ["listen"]);
        assert_eq!(dict["settings"].as_dict().map(Dict::len), Some(0));
    }

    #[test]
    fn syntax_errors_are_reported() {
        assert!(parse("server: [unclosed\n").is_err());
        assert!(parse("- a list\n").is_err());
    }
}

//! The Android project's settings that keep the app's secrets on the device and its build
//! inputs pinned, read from the files in gen/android.

use std::path::Path;

use neon_shell::servers::FILE_NAME;

fn read(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("gen/android")
        .join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The `<include>` and `<exclude>` rules of a backup rules file, as `(element, domain, path)`.
fn rules(file: &str) -> Vec<(String, String, String)> {
    let text = read(file);
    text.split('<')
        .filter(|tag| tag.starts_with("include ") || tag.starts_with("exclude "))
        .map(|tag| {
            let attribute = |name: &str| {
                let start = tag
                    .find(&format!("{name}=\""))
                    .unwrap_or_else(|| panic!("{file}: a rule with no {name}: <{tag}"))
                    + name.len()
                    + 2;
                tag[start..]
                    .split('"')
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            };
            let element = tag.split(' ').next().unwrap_or_default().to_owned();
            (element, attribute("domain"), attribute("path"))
        })
        .collect()
}

#[test]
fn the_app_makes_no_backups_and_moves_only_the_server_list() {
    let manifest = read("app/src/main/AndroidManifest.xml");
    for attribute in [
        r#"android:allowBackup="false""#,
        r#"android:dataExtractionRules="@xml/data_extraction_rules""#,
        r#"android:fullBackupContent="@xml/backup_rules""#,
    ] {
        assert!(
            manifest.contains(attribute),
            "the manifest lacks {attribute}"
        );
    }
    // The server list is in the app's data directory, the root of its private storage on
    // Android; an include rule leaves everything else out, the web view's data included.
    let only_the_list = vec![(
        "include".to_owned(),
        "root".to_owned(),
        FILE_NAME.to_owned(),
    )];
    assert_eq!(
        rules("app/src/main/res/xml/backup_rules.xml"),
        only_the_list
    );
    let extraction = read("app/src/main/res/xml/data_extraction_rules.xml");
    for section in ["cloud-backup", "device-transfer"] {
        assert!(
            extraction.contains(&format!("<{section}>")),
            "data_extraction_rules.xml has no {section} rules, which would take everything"
        );
    }
    assert_eq!(
        rules("app/src/main/res/xml/data_extraction_rules.xml"),
        [only_the_list.clone(), only_the_list].concat()
    );
}

#[test]
fn the_gradle_distribution_is_checked_against_its_checksum() {
    let wrapper = read("gradle/wrapper/gradle-wrapper.properties");
    let checksum = wrapper
        .lines()
        .find_map(|line| line.strip_prefix("distributionSha256Sum="))
        .expect("a distributionSha256Sum");
    assert_eq!(checksum.len(), 64);
    assert!(checksum.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

#[test]
fn the_build_signs_nothing_itself() {
    // The release workflow signs the APK in a job of its own, where no build code runs.
    let build = read("app/build.gradle.kts");
    for word in ["signingConfig", "keystore", "storePassword"] {
        assert!(!build.contains(word), "app/build.gradle.kts names {word}");
    }
}

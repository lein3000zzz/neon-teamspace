//! The Android project's settings that keep the app's secrets on the device and its build
//! inputs pinned, and what its activity does beside Tauri's, read from the files in
//! gen/android.

use std::path::Path;

use neon_shell::servers::FILE_NAME;

/// The app's activity.
const MAIN_ACTIVITY: &str = "app/src/main/java/io/github/lein3000zzz/neonteamspace/MainActivity.kt";

fn read(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("gen/android")
        .join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The body of the Kotlin function `name` in `source`: from the brace that opens it to the one
/// that closes it.
fn kotlin_function<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source
        .find(&format!("fun {name}("))
        .unwrap_or_else(|| panic!("no function {name}"));
    let open = start + source[start..].find('{').expect("a body");
    let mut depth = 0;
    for (at, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open..=open + at];
                }
            }
            _ => {}
        }
    }
    panic!("the body of {name} does not close");
}

/// The value of the colour resource `name` in the resource file `file`.
fn color(file: &str, name: &str) -> String {
    let text = read(&format!("app/src/main/res/{file}"));
    let start = text
        .find(&format!(r#"<color name="{name}">"#))
        .unwrap_or_else(|| panic!("{file} has no colour {name}"))
        + name.len()
        + 15;
    text[start..]
        .split('<')
        .next()
        .unwrap_or_default()
        .to_owned()
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

#[test]
fn the_window_follows_a_switch_of_light_or_dark_mode_while_the_app_runs() {
    // The switch does not recreate the activity, which would load the page again...
    let manifest = read("app/src/main/AndroidManifest.xml");
    let changes = manifest
        .split(r#"android:configChanges=""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the activity's configChanges");
    assert!(
        changes.split('|').any(|change| change == "uiMode"),
        "{changes}"
    );
    // ...so the activity applies the new mode's window colour, and the bars' icons, itself.
    let activity = read(MAIN_ACTIVITY);
    let changed = kotlin_function(&activity, "onConfigurationChanged");
    for call in [
        "super.onConfigurationChanged(newConfig)",
        "window.setBackgroundDrawableResource(R.color.window_background)",
        "enableEdgeToEdge()",
    ] {
        assert!(
            changed.contains(call),
            "onConfigurationChanged lacks {call}"
        );
    }
    // A colour for each mode, the Nothing palette's grounds, which the theme starts with.
    assert_eq!(color("values/colors.xml", "window_background"), "#FFFFFFFF");
    assert_eq!(
        color("values-night/colors.xml", "window_background"),
        "#FF000000"
    );
    let theme = read("app/src/main/res/values/themes.xml");
    assert!(
        theme.contains(r#"<item name="android:windowBackground">@color/window_background</item>"#),
        "{theme}"
    );
    let night_theme = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("gen/android/app/src/main/res/values-night/themes.xml");
    assert!(
        !night_theme.exists(),
        "a night theme of its own would set the window apart from the colour"
    );
}

#[test]
fn back_goes_through_the_webviews_history_before_it_leaves_the_app() {
    let activity = read(MAIN_ACTIVITY);
    assert!(
        kotlin_function(&activity, "onWebViewCreate").contains("goBackThroughHistory(webView)"),
        "the back handling is not set up with the webview"
    );
    let back = kotlin_function(&activity, "handleOnBackPressed");
    let at = |code: &str| {
        back.find(code)
            .unwrap_or_else(|| panic!("handleOnBackPressed lacks {code}"))
    };
    // Back in the history, which holds the entries open panels add, while there is any; out
    // of the app only once there is none.
    assert!(at("if (webView.canGoBack())") < at("webView.goBack()"));
    assert!(at("webView.goBack()") < at("} else {"));
    assert!(at("} else {") < at("onBackPressedDispatcher.onBackPressed()"));
}

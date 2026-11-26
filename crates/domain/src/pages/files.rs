//! Pages as files: the names an export gives pages, the titles an import takes from names,
//! and the paths of an archive's entries, which an import must keep inside the archive.
//!
//! An export writes each page as a markdown file named after its title, and the pages under
//! it in a folder of the same name beside the file, with the files its blocks show. File
//! systems refuse some characters and names, and some do not tell upper from lower case, so
//! names are cleaned and kept apart among siblings ignoring case. An import reads the same
//! layout back: a folder is a page, a markdown file with the folder's name beside it is that
//! page's content, and the other files are those its pages show.

use std::collections::HashSet;

use super::MAX_TITLE;

/// The longest name an export gives a page, in bytes, before a ` (2)` that keeps it apart
/// from a sibling's and its extension: well within the 255 bytes file systems allow.
pub const MAX_NAME: usize = 100;

/// What a page without a title is named.
pub const UNTITLED: &str = "Untitled";

/// Characters Windows, macOS or Linux refuse in a file name, besides control characters.
const REFUSED: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Names Windows keeps for devices, with or without an extension.
const DEVICES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The name of a page's file and folder in an export, without the extension: its title,
/// with each refused character and run of white space as one space, without dots or
/// spaces at the ends, cut to [`MAX_NAME`] bytes. A title with nothing left is
/// [`UNTITLED`], and a device name of Windows gets a `_` in front.
pub fn file_name(title: &str) -> String {
    let mut name = String::with_capacity(title.len());
    for c in title.chars() {
        let c = if c.is_control() || REFUSED.contains(&c) {
            ' '
        } else {
            c
        };
        if c.is_whitespace() {
            if !name.is_empty() && !name.ends_with(' ') {
                name.push(' ');
            }
        } else {
            name.push(c);
        }
    }
    let mut end = name.len().min(MAX_NAME);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name.truncate(end);
    let name = name.trim_matches(|c: char| c == '.' || c == ' ');
    if name.is_empty() {
        return UNTITLED.to_owned();
    }
    let device = name.split('.').next().unwrap_or(name);
    if DEVICES
        .iter()
        .any(|d| d.eq_ignore_ascii_case(device.trim_end()))
    {
        format!("_{name}")
    } else {
        name.to_owned()
    }
}

/// The extension of the markdown files an export writes.
pub const EXTENSION: &str = "md";

/// The names given to one set of siblings in an export. Each is a folder's name and, with
/// [`EXTENSION`], a file's, and no two of all those are the same, ignoring case: a page
/// titled `Notes.md` beside one titled `Notes` must not take the name of the other's file.
#[derive(Debug, Clone, Default)]
pub struct Names {
    taken: HashSet<String>,
}

impl Names {
    /// `name`, or the first of `name (2)`, `name (3)`, ... whose folder and file are both
    /// free, which are then taken.
    pub fn claim(&mut self, name: &str) -> String {
        let mut claimed = name.to_owned();
        let mut n = 1;
        loop {
            let folder = claimed.to_lowercase();
            let file = format!("{folder}.{EXTENSION}");
            if !self.taken.contains(&folder) && !self.taken.contains(&file) {
                self.taken.insert(folder);
                self.taken.insert(file);
                return claimed;
            }
            n += 1;
            claimed = format!("{name} ({n})");
        }
    }
}

impl Names {
    /// A free name for an attachment in the folder, from the file's own: `name`, or
    /// `stem (2).ext`, `stem (3).ext`, ..., which is then taken, so neither another file nor
    /// a page's folder or file takes it. A name that would read as markdown gets `.txt`
    /// after it, as an import makes a page of a markdown file.
    pub fn claim_file(&mut self, name: &str) -> String {
        let cleaned = file_name(name);
        let markdown = cleaned.rsplit_once('.').is_some_and(|(_, ext)| {
            ["md", "markdown"]
                .iter()
                .any(|md| ext.eq_ignore_ascii_case(md))
        });
        let name = if markdown {
            format!("{cleaned}.txt")
        } else {
            cleaned
        };
        let (stem, ext) = match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => (stem.to_owned(), format!(".{ext}")),
            _ => (name.clone(), String::new()),
        };
        let mut claimed = name;
        let mut n = 1;
        while self.taken.contains(&claimed.to_lowercase()) {
            n += 1;
            claimed = format!("{stem} ({n}){ext}");
        }
        self.taken.insert(claimed.to_lowercase());
        claimed
    }
}

/// A title from a text, such as a heading's or the name of a file without its extension:
/// line breaks and other control characters as spaces, trimmed, cut to [`MAX_TITLE`]
/// characters. What it gives passes [`check_title`](super::check_title).
pub fn clean_title(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    spaced
        .trim()
        .chars()
        .take(MAX_TITLE)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// An archive's entry whose path leads outside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UnsafePath {
    #[error("it starts at the root of a disk")]
    Absolute,
    #[error("it climbs out of its folder with ..")]
    Parent,
    #[error("it holds a NUL character")]
    Nul,
}

/// The folders and the name of an archive's entry, from its path, which must stay inside
/// the archive: no `/` or drive letter in front, no `..`, no NUL. Backslashes count as
/// slashes, as some archivers on Windows write them; empty parts and `.` are dropped. An
/// entry for a folder, its path ending with `/`, gives the folder's parts.
pub fn entry_path(raw: &str) -> Result<Vec<&str>, UnsafePath> {
    if raw.contains('\0') {
        return Err(UnsafePath::Nul);
    }
    let bytes = raw.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if raw.starts_with(['/', '\\']) || drive {
        return Err(UnsafePath::Absolute);
    }
    let mut parts = Vec::new();
    for part in raw.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return Err(UnsafePath::Parent),
            part => parts.push(part),
        }
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_become_names_file_systems_take() {
        assert_eq!(file_name("Roadmap"), "Roadmap");
        assert_eq!(file_name("Q1: Goals / Risks"), "Q1 Goals Risks");
        assert_eq!(file_name("  a\t\tb\nc  "), "a b c");
        assert_eq!(file_name("why? <maybe> \"so\" | *"), "why maybe so");
        assert_eq!(file_name("...hidden."), "hidden");
        assert_eq!(file_name(".env"), "env");
        assert_eq!(file_name(""), UNTITLED);
        assert_eq!(file_name(" / : "), UNTITLED);
        assert_eq!(file_name("Notes 🗺"), "Notes 🗺");
        assert_eq!(file_name("con"), "_con");
        assert_eq!(file_name("LPT1.notes"), "_LPT1.notes");
        assert_eq!(file_name("Console"), "Console");
    }

    #[test]
    fn names_are_cut_at_a_character() {
        let long = file_name(&"é".repeat(MAX_NAME));
        assert_eq!(long, "é".repeat(MAX_NAME / 2));
        let odd = file_name(&format!("a{}", "é".repeat(MAX_NAME)));
        assert_eq!(odd.len(), MAX_NAME - 1);
        assert_eq!(
            file_name(&format!("{} tail", "x".repeat(MAX_NAME - 1))),
            "x".repeat(MAX_NAME - 1)
        );
    }

    #[test]
    fn siblings_get_names_of_their_own_ignoring_case() {
        let mut names = Names::default();
        assert_eq!(names.claim("Notes"), "Notes");
        assert_eq!(names.claim("notes"), "notes (2)");
        assert_eq!(names.claim("Notes"), "Notes (3)");
        assert_eq!(names.claim("Notes (2)"), "Notes (2) (2)");
        assert_eq!(names.claim("Other"), "Other");
        assert_eq!(names.claim("other.MD"), "other.MD (2)");
        assert_eq!(names.claim("Plan.md"), "Plan.md");
        assert_eq!(names.claim("Plan"), "Plan (2)");
    }

    #[test]
    fn attachments_get_names_of_their_own_beside_pages() {
        let mut names = Names::default();
        assert_eq!(names.claim_file("diagram.png"), "diagram.png");
        assert_eq!(names.claim_file("Diagram.PNG"), "Diagram (2).PNG");
        assert_eq!(names.claim("diagram.png"), "diagram.png (2)");
        assert_eq!(names.claim("Notes"), "Notes");
        assert_eq!(names.claim_file("notes"), "notes (2)");
        assert_eq!(names.claim_file("notes.md"), "notes.md.txt");
        assert_eq!(names.claim_file("README.markdown"), "README.markdown.txt");
        assert_eq!(names.claim_file("a/b: c?.pdf"), "a b c .pdf");
        assert_eq!(names.claim_file(".env"), "env");
        assert_eq!(names.claim_file(""), UNTITLED);
        assert_eq!(names.claim_file("con.txt"), "_con.txt");
    }

    #[test]
    fn names_become_titles() {
        assert_eq!(clean_title(" Meeting notes "), "Meeting notes");
        assert_eq!(clean_title("two\nlines"), "two lines");
        assert_eq!(
            clean_title("x".repeat(MAX_TITLE + 5).as_str()).len(),
            MAX_TITLE
        );
        let cut = clean_title(&format!("{}  y", "x".repeat(MAX_TITLE - 1)));
        assert_eq!(cut, "x".repeat(MAX_TITLE - 1));
        assert!(super::super::check_title(&clean_title("a\u{7}b\r\n")).is_ok());
    }

    #[test]
    fn entry_paths_stay_inside_the_archive() {
        assert_eq!(
            entry_path("notes/q1/goals.md"),
            Ok(vec!["notes", "q1", "goals.md"])
        );
        assert_eq!(
            entry_path("notes\\q1\\goals.md"),
            Ok(vec!["notes", "q1", "goals.md"])
        );
        assert_eq!(entry_path("./notes//a.md"), Ok(vec!["notes", "a.md"]));
        assert_eq!(entry_path("notes/"), Ok(vec!["notes"]));
        assert_eq!(entry_path("a..b/c.md"), Ok(vec!["a..b", "c.md"]));
        for absolute in [
            "/etc/passwd",
            "\\\\server\\share\\a.md",
            "C:\\a.md",
            "c:a.md",
        ] {
            assert_eq!(
                entry_path(absolute),
                Err(UnsafePath::Absolute),
                "{absolute}"
            );
        }
        for climbing in ["../a.md", "notes/../../a.md", "notes\\..\\a.md", ".."] {
            assert_eq!(entry_path(climbing), Err(UnsafePath::Parent), "{climbing}");
        }
        assert_eq!(entry_path("a\0.md"), Err(UnsafePath::Nul));
    }
}

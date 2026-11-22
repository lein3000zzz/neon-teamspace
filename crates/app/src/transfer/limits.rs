use std::time::Duration;

const MIB: usize = 1024 * 1024;

/// Bounds on imports and exports, which keep what one of them takes of the server's memory
/// and time small and known. The defaults suit an instance of a few teams; tests shorten
/// them.
#[derive(Debug, Clone)]
pub struct Limits {
    /// The largest file an import takes, as uploaded: a markdown file, or a zip of them.
    pub max_upload: usize,
    /// The largest markdown file, alone or unpacked from a zip.
    pub max_file: usize,
    /// The most a zip's markdown files unpack to, together. Files left out are not
    /// unpacked and do not count.
    pub max_unpacked: usize,
    /// The most entries a zip lists: files and folders, those left out included.
    pub max_entries: usize,
    /// The most folders deep an entry of a zip is.
    pub max_depth: usize,
    /// The most pages one import makes, folders included.
    pub max_pages: usize,
    /// The most pages one export holds, rows of databases aside.
    pub max_export_pages: usize,
    /// The most rows of databases one export holds besides its pages: by default as many
    /// as a database holds out of the trash.
    pub max_export_rows: usize,
    /// The most bytes of markdown one export holds, before it is zipped.
    pub max_export_bytes: usize,
    /// The longest an import or export may take, once its file is uploaded.
    pub time_limit: Duration,
    /// How many imports and exports run at once on the server, once their file is
    /// uploaded; one more is refused until one ends.
    pub parallel: usize,
    /// How many imports and exports one person runs at once, an import's upload included;
    /// one more is refused until one ends.
    pub per_user: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_upload: 32 * MIB,
            max_file: 4 * MIB,
            max_unpacked: 64 * MIB,
            max_entries: 2000,
            max_depth: 32,
            max_pages: 1000,
            max_export_pages: 5000,
            max_export_rows: neon_domain::databases::MAX_ROWS,
            max_export_bytes: 128 * MIB,
            time_limit: Duration::from_secs(20),
            parallel: 4,
            per_user: 2,
        }
    }
}

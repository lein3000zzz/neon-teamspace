//! Attachments: files uploaded to a page, for its image, video, audio and file blocks and
//! its cover.
//!
//! What a file is comes from its bytes alone, never from the name or type its client sent:
//! [`sniff`] reads its first [`SNIFF_BYTES`] and names one of the [`MediaType`]s, and a
//! [`FileKind`], the block the file is for, takes only the types its allow list names. A
//! file is served as that type, shown in place only when it is an image, a video or audio
//! a browser draws without running anything, and saved as a download otherwise.
//!
//! A document refers to a file by its [`path`], `/api/v1/files/<id>`, on the server that
//! keeps it: the editor loads it from there, markdown carries it as a link, and the server
//! finds the files a document holds by it, with [`file_id`].
//!
//! Who may upload and read a file is the rules' to say, as for its page: a file belongs to
//! the page it was uploaded to.

use std::ops::Range;

use time::OffsetDateTime;
use uuid::Uuid;

/// How many of a file's first bytes [`sniff`] reads.
pub const SNIFF_BYTES: usize = 4096;

/// How many of an image's first bytes [`dimensions`] reads at most: enough for the size a
/// JPEG gives after its metadata.
pub const DIMENSION_BYTES: usize = 256 * 1024;

/// Where the server serves files, before the file's id.
pub const PATH: &str = "/api/v1/files/";

/// The longest name a file keeps, in bytes.
pub const MAX_NAME: usize = 255;

/// What a file without a name is called.
pub const UNNAMED: &str = "file";

text_enum! {
    /// What a file is uploaded for: the kind of block it backs, or a page's cover, which
    /// takes images.
    FileKind {
        Image => "image",
        Video => "video",
        Audio => "audio",
        /// Any file, shown as a link to download it.
        File => "file",
    }
}

impl FileKind {
    /// Whether a file of this type may back this kind of block: an image block takes
    /// images, a video block videos, an audio block audio and the containers recordings
    /// come in, and a file block anything.
    pub fn takes(self, media: MediaType) -> bool {
        use MediaType as M;
        match self {
            Self::Image => matches!(
                media,
                M::Png | M::Jpeg | M::Gif | M::Webp | M::Avif | M::Svg
            ),
            Self::Video => matches!(media, M::Mp4 | M::Webm | M::OggVideo | M::QuickTime),
            Self::Audio => matches!(
                media,
                M::Mp3 | M::Wav | M::Ogg | M::Flac | M::Aac | M::M4a | M::Webm | M::Mp4
            ),
            Self::File => true,
        }
    }

    /// The types this kind takes, as people read them, for the error that refuses one.
    pub fn allowed(self) -> &'static str {
        match self {
            Self::Image => "PNG, JPEG, GIF, WebP, AVIF or SVG",
            Self::Video => "MP4, WebM, Ogg or QuickTime",
            Self::Audio => "MP3, WAV, Ogg, FLAC, AAC, M4A, WebM or MP4",
            Self::File => "any file",
        }
    }
}

text_enum! {
    /// What a file's bytes are, as [`sniff`] reads them, kept and served as its MIME type.
    MediaType {
        Png => "image/png",
        Jpeg => "image/jpeg",
        Gif => "image/gif",
        Webp => "image/webp",
        Avif => "image/avif",
        /// Shown in `<img>`, where browsers run none of its script, and served as a
        /// download, never as a page of the app's origin.
        Svg => "image/svg+xml",
        Mp4 => "video/mp4",
        QuickTime => "video/quicktime",
        Webm => "video/webm",
        OggVideo => "video/ogg",
        Mp3 => "audio/mpeg",
        /// MPEG-4 audio, as `.m4a` files are.
        M4a => "audio/mp4",
        Ogg => "audio/ogg",
        Wav => "audio/wav",
        Flac => "audio/flac",
        Aac => "audio/aac",
        Pdf => "application/pdf",
        Zip => "application/zip",
        Gzip => "application/gzip",
        Html => "text/html",
        Xml => "application/xml",
        Text => "text/plain",
        /// Anything else.
        Unknown => "application/octet-stream",
    }
}

impl MediaType {
    /// Whether a browser may show it in place, as an image, a video or audio, which run no
    /// script. Everything else is served as a download.
    pub fn is_inline(self) -> bool {
        use MediaType as M;
        matches!(
            self,
            M::Png
                | M::Jpeg
                | M::Gif
                | M::Webp
                | M::Avif
                | M::Mp4
                | M::QuickTime
                | M::Webm
                | M::OggVideo
                | M::Mp3
                | M::M4a
                | M::Ogg
                | M::Wav
                | M::Flac
                | M::Aac
        )
    }

    /// Whether [`dimensions`] reads its width and height.
    pub fn has_dimensions(self) -> bool {
        matches!(
            self,
            Self::Png | Self::Jpeg | Self::Gif | Self::Webp | Self::Avif
        )
    }

    /// The `Content-Type` it is served with: text in UTF-8, as [`sniff`] found it.
    pub fn header_value(self) -> &'static str {
        match self {
            Self::Html => "text/html; charset=utf-8",
            Self::Text => "text/plain; charset=utf-8",
            other => other.as_str(),
        }
    }

    /// The extension a file of this type usually has, for the name of a file that has
    /// none.
    pub fn extension(self) -> Option<&'static str> {
        use MediaType as M;
        Some(match self {
            M::Png => "png",
            M::Jpeg => "jpg",
            M::Gif => "gif",
            M::Webp => "webp",
            M::Avif => "avif",
            M::Svg => "svg",
            M::Mp4 => "mp4",
            M::QuickTime => "mov",
            M::Webm => "webm",
            M::OggVideo => "ogv",
            M::Mp3 => "mp3",
            M::M4a => "m4a",
            M::Ogg => "ogg",
            M::Wav => "wav",
            M::Flac => "flac",
            M::Aac => "aac",
            M::Pdf => "pdf",
            M::Zip => "zip",
            M::Gzip => "gz",
            M::Html => "html",
            M::Xml => "xml",
            M::Text => "txt",
            M::Unknown => return None,
        })
    }
}

/// What a file is, from its first bytes: up to [`SNIFF_BYTES`] of them, all of it when
/// `whole`. Signatures of binary formats come first; then text in UTF-8 is markup or plain
/// text, by how it starts. Anything else is [`MediaType::Unknown`].
pub fn sniff(head: &[u8], whole: bool) -> MediaType {
    let head = &head[..head.len().min(SNIFF_BYTES)];
    binary(head)
        .or_else(|| text(head, whole))
        .unwrap_or(MediaType::Unknown)
}

fn binary(head: &[u8]) -> Option<MediaType> {
    use MediaType as M;
    let at = |offset: usize, signature: &[u8]| {
        head.get(offset..offset + signature.len()) == Some(signature)
    };
    let holds = |needle: &[u8]| head.windows(needle.len()).any(|window| window == needle);
    if at(0, b"\x89PNG\r\n\x1a\n") {
        return Some(M::Png);
    }
    if at(0, b"\xFF\xD8\xFF") {
        return Some(M::Jpeg);
    }
    if at(0, b"GIF87a") || at(0, b"GIF89a") {
        return Some(M::Gif);
    }
    if at(0, b"RIFF") && at(8, b"WEBP") {
        return Some(M::Webp);
    }
    if at(0, b"RIFF") && at(8, b"WAVE") {
        return Some(M::Wav);
    }
    if at(4, b"ftyp") {
        return Some(iso_media(head));
    }
    if at(0, b"\x1A\x45\xDF\xA3") {
        // Matroska, of which browsers play the WebM kind only.
        return Some(if holds(b"webm") { M::Webm } else { M::Unknown });
    }
    if at(0, b"OggS") {
        return Some(if holds(b"\x80theora") {
            M::OggVideo
        } else {
            M::Ogg
        });
    }
    if at(0, b"fLaC") {
        return Some(M::Flac);
    }
    if at(0, b"ID3") {
        return Some(M::Mp3);
    }
    if let [0xFF, second, third, ..] = *head {
        // An audio frame's sync, then: layer 00 and a sampling rate of the 13 is AAC in
        // ADTS; layer III of a known version, with a bitrate and a sampling rate, is MP3.
        if second & 0xF6 == 0xF0 && (third >> 2) & 0x0F <= 12 {
            return Some(M::Aac);
        }
        let version = (second >> 3) & 0x03;
        let bitrate = third >> 4;
        if second & 0xE0 == 0xE0
            && second & 0x06 == 0x02
            && version != 0x01
            && !matches!(bitrate, 0x00 | 0x0F)
            && (third >> 2) & 0x03 != 0x03
        {
            return Some(M::Mp3);
        }
    }
    if at(0, b"%PDF-") {
        return Some(M::Pdf);
    }
    if at(0, b"PK\x03\x04") || at(0, b"PK\x05\x06") {
        return Some(M::Zip);
    }
    if at(0, b"\x1F\x8B") {
        return Some(M::Gzip);
    }
    None
}

/// An ISO base media file, by its brands: AVIF images, MPEG-4 audio, QuickTime, and
/// MPEG-4 video for the rest. HEIF images other than AVIF, which few browsers show, are
/// unknown.
fn iso_media(head: &[u8]) -> MediaType {
    use MediaType as M;
    let size = head
        .get(0..4)
        .and_then(|b| b.try_into().ok())
        .map_or(0, u32::from_be_bytes);
    let end = usize::try_from(size)
        .unwrap_or(usize::MAX)
        .clamp(16, head.len().max(16));
    let major = head.get(8..12).unwrap_or_default();
    let (compatible, _) = head
        .get(16..end.min(head.len()))
        .unwrap_or_default()
        .as_chunks::<4>();
    let brand = |name: &[u8; 4]| major == name || compatible.contains(name);
    if brand(b"avif") || brand(b"avis") {
        M::Avif
    } else if [
        b"heic", b"heix", b"hevc", b"heim", b"heis", b"mif1", b"msf1",
    ]
    .iter()
    .any(|name| major == *name)
    {
        M::Unknown
    } else if [b"M4A ", b"M4B ", b"M4P ", b"F4A "]
        .iter()
        .any(|name| major == *name)
    {
        M::M4a
    } else if major == b"qt  " {
        M::QuickTime
    } else {
        M::Mp4
    }
}

/// Text in UTF-8, as HTML, SVG, other XML or plain text. A character the end of a partial
/// head cuts in two still counts as text.
fn text(head: &[u8], whole: bool) -> Option<MediaType> {
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let text = match std::str::from_utf8(head) {
        Ok(text) => text,
        Err(error) if !whole && error.error_len().is_none() => {
            std::str::from_utf8(&head[..error.valid_up_to()]).ok()?
        }
        Err(_) => return None,
    };
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\x0C'))
    {
        return None;
    }
    Some(markup(text))
}

/// The tags whose opening says a text is HTML, as the WHATWG's sniffing rules list them.
const HTML_TAGS: &[&str] = &[
    "!doctype html",
    "html",
    "head",
    "script",
    "iframe",
    "h1",
    "div",
    "font",
    "table",
    "a",
    "style",
    "title",
    "b",
    "body",
    "br",
    "p",
    "!--",
];

fn markup(text: &str) -> MediaType {
    let start = text.trim_start();
    let lower: String = start
        .chars()
        .take(SNIFF_BYTES)
        .flat_map(char::to_lowercase)
        .collect();
    let opens = |tag: &str| {
        lower
            .strip_prefix('<')
            .and_then(|rest| rest.strip_prefix(tag))
            .is_some_and(|rest| {
                rest.is_empty() || rest.starts_with([' ', '>', '\t', '\n', '\r', '/'])
            })
    };
    if opens("svg") {
        return MediaType::Svg;
    }
    if lower.starts_with("<?xml") || opens("!--") || lower.starts_with("<!doctype svg") {
        if lower.contains("<svg") {
            return MediaType::Svg;
        }
        if lower.contains("<html") {
            return MediaType::Html;
        }
        if lower.starts_with("<?xml") || lower.starts_with("<!doctype svg") {
            return MediaType::Xml;
        }
    }
    if HTML_TAGS.iter().any(|tag| opens(tag)) {
        return MediaType::Html;
    }
    MediaType::Text
}

/// An image's width and height in pixels, from its first bytes, up to
/// [`DIMENSION_BYTES`]: `None` for a type without them, when what was read does not say,
/// or when either side is past `i32::MAX`, the most the database keeps, which no real image
/// is.
pub fn dimensions(media: MediaType, head: &[u8]) -> Option<(u32, u32)> {
    if !media.has_dimensions() {
        return None;
    }
    let size = imagesize::blob_size(&head[..head.len().min(DIMENSION_BYTES)]).ok()?;
    let side = |pixels: usize| {
        i32::try_from(pixels)
            .ok()
            .filter(|side| *side > 0)
            .map(i32::unsigned_abs)
    };
    Some((side(size.width)?, side(size.height)?))
}

/// A file as kept: what it is, whose page it belongs to, and where its bytes are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// The page it was uploaded to, whose access it follows.
    pub page_id: Uuid,
    pub uploaded_by: Option<Uuid>,
    pub kind: FileKind,
    /// Where its bytes are in the blob store: made for it alone.
    pub key: String,
    /// The name it was uploaded with, as [`clean_name`] keeps it.
    pub name: String,
    pub media: MediaType,
    pub size: u64,
    /// The SHA-256 of its bytes.
    pub sha256: [u8; 32],
    /// An image's width and height in pixels, when its header says.
    pub dimensions: Option<(u32, u32)>,
    pub created_at: OffsetDateTime,
}

impl File {
    /// The path documents refer to it by.
    pub fn path(&self) -> String {
        path(self.id)
    }
}

/// The path a document refers to a file by, on the server that keeps it.
pub fn path(id: Uuid) -> String {
    format!("{PATH}{}", id.hyphenated())
}

/// The id of the file a URL refers to, when it is a file's [`path`], exactly.
pub fn file_id(url: &str) -> Option<Uuid> {
    let id = url.strip_prefix(PATH)?;
    // Uuid::parse_str also takes braces, URNs and the simple form: only the hyphenated one
    // is a path.
    (id.len() == 36).then(|| Uuid::parse_str(id).ok()).flatten()
}

/// The name a file keeps: the last part of the name its client sent, without control
/// characters, trimmed, cut to [`MAX_NAME`] bytes at a character, and [`UNNAMED`] with
/// the type's extension when nothing is left.
pub fn clean_name(raw: &str, media: MediaType) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    let name = trimmed[..trimmed.floor_char_boundary(MAX_NAME)].trim_end();
    if name.is_empty() || name.chars().all(|c| c == '.') {
        return match media.extension() {
            Some(extension) => format!("{UNNAMED}.{extension}"),
            None => UNNAMED.to_owned(),
        };
    }
    name.to_owned()
}

/// A file name's extension, in lowercase, when it is a short run of ASCII letters and
/// digits: what logs say of a file's name, and no more.
pub fn extension(name: &str) -> Option<String> {
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty()
        && (1..=10).contains(&extension.len())
        && extension.bytes().all(|b| b.is_ascii_alphanumeric()))
    .then(|| extension.to_ascii_lowercase())
}

/// What a `Range` header asks of a file, by RFC 9110.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ranged {
    /// All of it: no range, one that does not parse, or several, which are answered whole.
    Whole,
    /// These bytes of it.
    Part(Range<u64>),
    /// None of its bytes: the range starts at or past its end.
    Unsatisfiable,
}

/// The bytes of a file of `size` bytes a `Range` header asks for: `bytes=first-last`, with
/// `last` cut to the file's end, `bytes=first-` to the end, or `bytes=-n`, the last `n`.
pub fn byte_range(header: &str, size: u64) -> Ranged {
    let Some((unit, set)) = header.trim().split_once('=') else {
        return Ranged::Whole;
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") || set.contains(',') {
        return Ranged::Whole;
    }
    let Some((first, last)) = set.trim().split_once('-') else {
        return Ranged::Whole;
    };
    let number = |text: &str| -> Option<u64> {
        let text = text.trim();
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse().ok())
            .flatten()
    };
    match (number(first), number(last), last.trim().is_empty()) {
        (None, Some(suffix), _) if first.trim().is_empty() => {
            if suffix == 0 || size == 0 {
                Ranged::Unsatisfiable
            } else {
                Ranged::Part(size.saturating_sub(suffix)..size)
            }
        }
        (Some(first), None, true) => {
            if first >= size {
                Ranged::Unsatisfiable
            } else {
                Ranged::Part(first..size)
            }
        }
        (Some(first), Some(last), _) if first <= last => {
            if first >= size {
                Ranged::Unsatisfiable
            } else {
                Ranged::Part(first..last.saturating_add(1).min(size))
            }
        }
        _ => Ranged::Whole,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MediaType as M;

    /// The smallest valid PNG header with the given size: the signature and an IHDR chunk.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
        let size = u32::try_from(16 + 4 * compatible.len()).expect("small");
        let mut bytes = size.to_be_bytes().to_vec();
        bytes.extend(b"ftyp");
        bytes.extend(major);
        bytes.extend([0, 0, 0, 0]);
        for brand in compatible {
            bytes.extend(*brand);
        }
        bytes.extend(b"\0\0\0\x08free");
        bytes
    }

    #[test]
    fn binary_formats_are_known_by_their_signatures() {
        let cases: Vec<(Vec<u8>, MediaType)> = vec![
            (png(1, 1), M::Png),
            (b"\xFF\xD8\xFF\xE0\0\x10JFIF".to_vec(), M::Jpeg),
            (b"GIF89a\x01\0\x01\0".to_vec(), M::Gif),
            (b"GIF87a\x01\0\x01\0".to_vec(), M::Gif),
            (b"RIFF\x24\0\0\0WEBPVP8 ".to_vec(), M::Webp),
            (b"RIFF\x24\0\0\0WAVEfmt ".to_vec(), M::Wav),
            (b"RIFF\x24\0\0\0AVI LIST".to_vec(), M::Unknown),
            (ftyp(b"avif", &[b"mif1", b"miaf"]), M::Avif),
            (ftyp(b"mif1", &[b"avif"]), M::Avif),
            (ftyp(b"heic", &[b"mif1"]), M::Unknown),
            (ftyp(b"mif1", &[b"heic"]), M::Unknown),
            (ftyp(b"isom", &[b"iso2", b"avc1", b"mp41"]), M::Mp4),
            (ftyp(b"mp42", &[b"isom"]), M::Mp4),
            (ftyp(b"M4A ", &[b"isom"]), M::M4a),
            (ftyp(b"qt  ", &[b"qt  "]), M::QuickTime),
            (
                b"\x1A\x45\xDF\xA3\x9F\x42\x86\x81\x01\x42\x82\x84webm".to_vec(),
                M::Webm,
            ),
            (
                b"\x1A\x45\xDF\xA3\x9F\x42\x86\x81\x01\x42\x82\x88matroska".to_vec(),
                M::Unknown,
            ),
            (b"OggS\0\x02\0\0\0\0\0\0\0\0\x01vorbis".to_vec(), M::Ogg),
            (b"OggS\0\x02\0\0\0\0\0\0\0\0OpusHead".to_vec(), M::Ogg),
            (
                b"OggS\0\x02\0\0\0\0\0\0\0\0\x80theora".to_vec(),
                M::OggVideo,
            ),
            (b"fLaC\0\0\0\x22".to_vec(), M::Flac),
            (b"ID3\x04\0\0\0\0\0\0".to_vec(), M::Mp3),
            (b"\xFF\xFB\x90\x64".to_vec(), M::Mp3),
            (b"\xFF\xF1\x50\x80".to_vec(), M::Aac),
            (b"\xFF\xF9\x50\x80".to_vec(), M::Aac),
            (b"\xFF\xFB\xF0\x64".to_vec(), M::Unknown),
            (b"\xFF\xF3\x9C\x64".to_vec(), M::Unknown),
            (b"%PDF-1.7\n%\xE2\xE3\xCF\xD3".to_vec(), M::Pdf),
            (b"PK\x03\x04\x14\0\0\0".to_vec(), M::Zip),
            (b"PK\x05\x06\0\0\0\0".to_vec(), M::Zip),
            (b"\x1F\x8B\x08\0".to_vec(), M::Gzip),
            (b"\x7FELF\x02\x01\x01\0".to_vec(), M::Unknown),
            (b"MZ\x90\0\x03\0\0\0".to_vec(), M::Unknown),
            (vec![0, 0, 0, 0], M::Unknown),
        ];
        for (bytes, expected) in cases {
            assert_eq!(sniff(&bytes, true), expected, "{bytes:?}");
        }
    }

    #[test]
    fn text_is_markup_or_plain() {
        let cases = [
            ("hello, world\n", M::Text),
            ("\u{feff}a text with a byte order mark", M::Text),
            ("Grüße, 日本語\r\n\ttabs", M::Text),
            ("", M::Text),
            ("<svg xmlns=\"http://www.w3.org/2000/svg\"/>", M::Svg),
            ("  \n<SVG viewBox=\"0 0 1 1\"></SVG>", M::Svg),
            (
                "<?xml version=\"1.0\"?>\n<!-- made by hand -->\n<svg><script>alert(1)</script></svg>",
                M::Svg,
            ),
            (
                "<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\"><svg/>",
                M::Svg,
            ),
            ("<!-- a comment --><svg/>", M::Svg),
            ("<?xml version=\"1.0\"?><feed/>", M::Xml),
            (
                "<?xml version=\"1.0\"?><html><script>x</script></html>",
                M::Html,
            ),
            ("<!doctype html><title>x</title>", M::Html),
            ("<HTML>", M::Html),
            ("<script>alert(1)</script>", M::Html),
            ("<iframe src=x>", M::Html),
            ("<body onload=x>", M::Html),
            ("<p>a paragraph", M::Html),
            ("<!-- a comment -->", M::Html),
            ("<a href=x>", M::Html),
            ("<abbr>not a tag the rules know</abbr>", M::Text),
            ("<svgx>", M::Text),
            ("a <script> later in the text", M::Text),
        ];
        for (text, expected) in cases {
            assert_eq!(sniff(text.as_bytes(), true), expected, "{text:?}");
        }
    }

    #[test]
    fn binary_is_not_text() {
        assert_eq!(sniff(b"text\0with a NUL", true), M::Unknown);
        assert_eq!(sniff(b"text with a bell \x07", true), M::Unknown);
        assert_eq!(sniff(b"\xC3\x28 not UTF-8", true), M::Unknown);
        // UTF-16, which carries NULs.
        assert_eq!(sniff(b"\xFF\xFEh\0i\0", true), M::Unknown);
    }

    #[test]
    fn a_character_cut_by_the_head_still_counts_as_text() {
        let cut = "ab\u{1F5FA}".as_bytes();
        let partial = &cut[..cut.len() - 1];
        assert_eq!(sniff(partial, false), M::Text);
        assert_eq!(
            sniff(partial, true),
            M::Unknown,
            "a whole file must be UTF-8"
        );
    }

    #[test]
    fn only_the_head_is_read() {
        let mut long = "a".repeat(SNIFF_BYTES).into_bytes();
        long.push(0);
        assert_eq!(sniff(&long, false), M::Text);
    }

    #[test]
    fn each_kind_takes_its_own_types() {
        let images = [M::Png, M::Jpeg, M::Gif, M::Webp, M::Avif, M::Svg];
        let videos = [M::Mp4, M::Webm, M::OggVideo, M::QuickTime];
        let audio = [
            M::Mp3,
            M::Wav,
            M::Ogg,
            M::Flac,
            M::Aac,
            M::M4a,
            M::Webm,
            M::Mp4,
        ];
        for media in MediaType::ALL.iter().copied() {
            assert_eq!(
                FileKind::Image.takes(media),
                images.contains(&media),
                "{media}"
            );
            assert_eq!(
                FileKind::Video.takes(media),
                videos.contains(&media),
                "{media}"
            );
            assert_eq!(
                FileKind::Audio.takes(media),
                audio.contains(&media),
                "{media}"
            );
            assert!(FileKind::File.takes(media), "{media}");
        }
        for kind in FileKind::ALL {
            assert!(!kind.allowed().is_empty());
            assert_eq!(FileKind::parse(kind.as_str()), Some(*kind));
        }
    }

    #[test]
    fn only_media_without_script_shows_in_place() {
        let inline: Vec<MediaType> = MediaType::ALL
            .iter()
            .copied()
            .filter(|media| media.is_inline())
            .collect();
        assert_eq!(
            inline,
            [
                M::Png,
                M::Jpeg,
                M::Gif,
                M::Webp,
                M::Avif,
                M::Mp4,
                M::QuickTime,
                M::Webm,
                M::OggVideo,
                M::Mp3,
                M::M4a,
                M::Ogg,
                M::Wav,
                M::Flac,
                M::Aac,
            ]
        );
        for downloaded in [M::Svg, M::Html, M::Xml, M::Pdf, M::Text, M::Zip, M::Unknown] {
            assert!(!downloaded.is_inline(), "{downloaded}");
        }
    }

    #[test]
    fn types_round_trip_through_their_text() {
        for media in MediaType::ALL {
            assert_eq!(MediaType::parse(media.as_str()), Some(*media));
            assert!(media.header_value().starts_with(media.as_str()));
        }
        assert_eq!(M::Text.header_value(), "text/plain; charset=utf-8");
        assert_eq!(M::Html.header_value(), "text/html; charset=utf-8");
        assert_eq!(M::Png.header_value(), "image/png");
        assert_eq!(M::Unknown.extension(), None);
        assert_eq!(M::Jpeg.extension(), Some("jpg"));
    }

    #[test]
    fn images_have_dimensions() {
        assert_eq!(dimensions(M::Png, &png(640, 480)), Some((640, 480)));
        assert_eq!(
            dimensions(M::Gif, b"GIF89a\x40\x01\xF0\x00\0\0\0"),
            Some((320, 240))
        );
        assert_eq!(dimensions(M::Png, &png(0, 480)), None, "no area");
        let widest = u32::try_from(i32::MAX).expect("positive");
        assert_eq!(dimensions(M::Png, &png(widest, 1)), Some((widest, 1)));
        assert_eq!(
            dimensions(M::Png, &png(u32::MAX, 1)),
            None,
            "past what is kept"
        );
        assert_eq!(
            dimensions(M::Png, &png(1, widest + 1)),
            None,
            "past what is kept"
        );
        assert_eq!(dimensions(M::Png, b"\x89PNG\r\n\x1a\n"), None, "cut short");
        assert_eq!(dimensions(M::Svg, b"<svg width=\"10\"/>"), None);
        assert_eq!(dimensions(M::Pdf, &png(1, 1)), None);
    }

    #[test]
    fn documents_name_files_by_their_path() {
        let id = Uuid::from_u128(0x0193_a5b2_0000_7000_8000_0000_0000_00f1);
        let path = path(id);
        assert_eq!(path, "/api/v1/files/0193a5b2-0000-7000-8000-0000000000f1");
        assert_eq!(file_id(&path), Some(id));
        assert_eq!(
            file_id(&path.to_uppercase().replace("/API/V1/FILES/", PATH)),
            Some(id)
        );
        for other in [
            "/api/v1/files/",
            "/api/v1/files/not-an-id",
            "/api/v1/files/0193a5b200007000800000000000000f1",
            "/api/v1/files/{0193a5b2-0000-7000-8000-0000000000f1}",
            "/api/v1/files/0193a5b2-0000-7000-8000-0000000000f1/x",
            "/api/v1/files/0193a5b2-0000-7000-8000-0000000000f1?download",
            "https://example.com/api/v1/files/0193a5b2-0000-7000-8000-0000000000f1",
            "api/v1/files/0193a5b2-0000-7000-8000-0000000000f1",
            "https://example.com/diagram.png",
        ] {
            assert_eq!(file_id(other), None, "{other}");
        }
    }

    #[test]
    fn ranges_ask_for_bytes_within_the_file() {
        let cases = [
            ("bytes=0-99", Ranged::Part(0..100)),
            ("bytes=100-", Ranged::Part(100..1000)),
            ("bytes=-100", Ranged::Part(900..1000)),
            ("bytes=-5000", Ranged::Part(0..1000)),
            ("bytes=990-2000", Ranged::Part(990..1000)),
            ("bytes=999-999", Ranged::Part(999..1000)),
            (" Bytes = 5 - 9 ", Ranged::Part(5..10)),
            ("bytes=1000-", Ranged::Unsatisfiable),
            ("bytes=1000-1001", Ranged::Unsatisfiable),
            ("bytes=-0", Ranged::Unsatisfiable),
            ("bytes=0-1,5-9", Ranged::Whole),
            ("bytes=9-5", Ranged::Whole),
            ("bytes=a-b", Ranged::Whole),
            ("bytes=-", Ranged::Whole),
            ("bytes=+1-2", Ranged::Whole),
            ("items=0-1", Ranged::Whole),
            ("0-1", Ranged::Whole),
            ("bytes=0-99999999999999999999999", Ranged::Whole),
        ];
        for (header, expected) in cases {
            assert_eq!(byte_range(header, 1000), expected, "{header}");
        }
        assert_eq!(byte_range("bytes=0-", 0), Ranged::Unsatisfiable);
        assert_eq!(byte_range("bytes=-1", 0), Ranged::Unsatisfiable);
    }

    #[test]
    fn names_keep_what_the_client_sent_within_bounds() {
        assert_eq!(clean_name("diagram.png", M::Png), "diagram.png");
        assert_eq!(
            clean_name("C:\\Users\\me\\report.pdf", M::Pdf),
            "report.pdf"
        );
        assert_eq!(clean_name("photos/2026/beach.jpg", M::Jpeg), "beach.jpg");
        assert_eq!(clean_name("  two\nlines.txt ", M::Text), "twolines.txt");
        assert_eq!(clean_name("", M::Png), "file.png");
        assert_eq!(clean_name("...", M::Unknown), "file");
        assert_eq!(clean_name("dir/", M::Pdf), "file.pdf");
        assert_eq!(
            clean_name(&"é".repeat(200), M::Text),
            "é".repeat(MAX_NAME / 2)
        );
    }

    #[test]
    fn logs_see_extensions_alone() {
        assert_eq!(extension("Quarterly report.PDF").as_deref(), Some("pdf"));
        assert_eq!(extension("archive.tar.gz").as_deref(), Some("gz"));
        assert_eq!(extension("no extension"), None);
        assert_eq!(extension(".env"), None);
        assert_eq!(extension("a.secret name"), None);
        assert_eq!(extension("a.waytoolongextension"), None);
        assert_eq!(extension("a.ünï"), None);
    }
}

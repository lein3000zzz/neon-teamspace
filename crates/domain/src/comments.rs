//! Comments: threads on a page, on the page as a whole or on a range of its text, with
//! replies, and what a comment and an anchor may hold.
//!
//! A comment is markdown limited to inline content: text with its styles, links, line
//! breaks and mentions, up to [`MAX_COMMENT`] characters. An inline thread is anchored to a
//! range of the page's text by two Yjs relative positions, as Yjs encodes them, which keep
//! to the same characters while the document changes around them, and keeps the text it
//! was made on, its quote. Who may comment is for the permission rules, see
//! [`authorize`](crate::workspaces::authorize).

/// The most characters a comment's markdown holds.
pub const MAX_COMMENT: usize = 10_000;

/// The most characters of an inline thread's quote, the text it was made on. A longer
/// selection keeps its start.
pub const MAX_QUOTE: usize = 1000;

/// The most bytes of one encoded relative position. Yjs writes one in under 30; the
/// bound leaves room and keeps what a client sends small.
pub const MAX_POSITION: usize = 256;

/// The most threads a page holds, resolved ones included.
pub const MAX_THREADS: usize = 1000;

/// The most comments one thread holds.
pub const MAX_REPLIES: usize = 500;

/// Where an inline thread is: the range of text between two Yjs relative positions, and
/// the text it held when the thread began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub start: Vec<u8>,
    pub end: Vec<u8>,
    pub quote: String,
}

/// A comment that cannot be kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidComment {
    #[error("a comment holds some text")]
    Empty,
    #[error("a comment holds at most {MAX_COMMENT} characters")]
    TooLong,
}

/// The markdown of a comment as kept: trimmed, not empty, at most [`MAX_COMMENT`]
/// characters. What of it is inline content is the converter's to say.
pub fn check_comment(raw: &str) -> Result<String, InvalidComment> {
    let body = raw.trim();
    if body.is_empty() {
        Err(InvalidComment::Empty)
    } else if body.chars().count() > MAX_COMMENT {
        Err(InvalidComment::TooLong)
    } else {
        Ok(body.to_owned())
    }
}

/// An anchor that cannot be kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidAnchor {
    #[error("an anchor's positions are each 1 to {MAX_POSITION} bytes")]
    Position,
    #[error("an anchor quotes the text it is on, which is not empty")]
    NoQuote,
}

/// An anchor as kept: positions of 1 to [`MAX_POSITION`] bytes, and its quote, without
/// control characters but line breaks, cut to [`MAX_QUOTE`] characters.
pub fn check_anchor(start: Vec<u8>, end: Vec<u8>, quote: &str) -> Result<Anchor, InvalidAnchor> {
    let fits = |position: &[u8]| (1..=MAX_POSITION).contains(&position.len());
    if !fits(&start) || !fits(&end) {
        return Err(InvalidAnchor::Position);
    }
    let quote: String = quote
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .take(MAX_QUOTE)
        .collect();
    if quote.trim().is_empty() {
        return Err(InvalidAnchor::NoQuote);
    }
    Ok(Anchor { start, end, quote })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_are_trimmed_bounded_and_never_empty() {
        assert_eq!(check_comment("  Looks good  "), Ok("Looks good".to_owned()));
        assert_eq!(check_comment(" \n "), Err(InvalidComment::Empty));
        assert_eq!(
            check_comment(&"é".repeat(MAX_COMMENT)).map(|body| body.chars().count()),
            Ok(MAX_COMMENT)
        );
        assert_eq!(
            check_comment(&"a".repeat(MAX_COMMENT + 1)),
            Err(InvalidComment::TooLong)
        );
    }

    #[test]
    fn anchors_keep_small_positions_and_a_quote() {
        let anchor = check_anchor(vec![0, 1, 2], vec![0, 1, 3], "the\tquoted\ntext").expect("kept");
        assert_eq!(anchor.quote, "the quoted\ntext");
        assert_eq!(
            check_anchor(Vec::new(), vec![1], "x"),
            Err(InvalidAnchor::Position)
        );
        assert_eq!(
            check_anchor(vec![1], vec![1; MAX_POSITION + 1], "x"),
            Err(InvalidAnchor::Position)
        );
        assert_eq!(
            check_anchor(vec![1], vec![1], " "),
            Err(InvalidAnchor::NoQuote)
        );
        let long = check_anchor(vec![1], vec![1], &"q".repeat(MAX_QUOTE * 2)).expect("kept");
        assert_eq!(long.quote.chars().count(), MAX_QUOTE);
    }
}

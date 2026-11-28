//! Where what an update prints goes: the end of it into the status, every line of it into the
//! updater's log, and, in the update's own container, its standard output.

use std::io::Write as _;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Takes a command's output as it comes, in whatever pieces it comes in.
pub trait Output: Send + Sync {
    /// Takes the next bytes.
    fn write(&self, bytes: &[u8]);

    /// Takes a line of the updater's own.
    fn line(&self, text: &str) {
        self.write(format!("{text}\n").as_bytes());
    }
}

/// The last bytes written, up to a bound.
#[derive(Debug)]
pub struct Tail {
    max: usize,
    bytes: Mutex<Vec<u8>>,
}

impl Tail {
    /// Keeps the last `max` bytes written.
    #[must_use]
    pub fn new(max: usize) -> Self {
        Self {
            max,
            bytes: Mutex::new(Vec::new()),
        }
    }

    /// What it keeps, as text: never half a character where the bound cut one, and without
    /// any bytes that are not UTF-8, so never longer than the bound.
    #[must_use]
    pub fn text(&self) -> String {
        let bytes = lock(&self.bytes);
        let start = bytes
            .iter()
            .position(|&b| !is_continuation(b))
            .unwrap_or(bytes.len());
        bytes[start..]
            .utf8_chunks()
            .map(|chunk| chunk.valid())
            .collect()
    }
}

impl Output for Tail {
    fn write(&self, bytes: &[u8]) {
        let mut kept = lock(&self.bytes);
        kept.extend_from_slice(bytes);
        if let Some(over) = kept.len().checked_sub(self.max) {
            kept.drain(..over);
        }
    }
}

/// A byte inside a UTF-8 character, never its first.
fn is_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

/// The longest line the updater's log takes whole: a longer one, such as a progress bar that
/// never prints a newline, is logged in pieces of this many bytes.
const MAX_LINE_BYTES: usize = 4096;

/// The end of an update's output for its status, and each line of it for the updater's log.
#[derive(Debug)]
pub struct Recorder {
    tail: Tail,
    /// What came after the last newline, until the line ends, at most [`MAX_LINE_BYTES`].
    partial: Mutex<Vec<u8>>,
}

impl Recorder {
    /// Keeps the last `max` bytes for the status.
    #[must_use]
    pub fn new(max: usize) -> Self {
        Self {
            tail: Tail::new(max),
            partial: Mutex::new(Vec::new()),
        }
    }

    /// The end of the output, as [`Tail::text`] has it.
    #[must_use]
    pub fn text(&self) -> String {
        self.tail.text()
    }

    /// Logs the last line, should it not have ended with a newline.
    pub fn finish(&self) {
        let rest = std::mem::take(&mut *lock(&self.partial));
        if !rest.is_empty() {
            log_line(&rest);
        }
    }
}

impl Output for Recorder {
    fn write(&self, bytes: &[u8]) {
        self.tail.write(bytes);
        let mut partial = lock(&self.partial);
        partial.extend_from_slice(bytes);
        while let Some(end) = partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = partial.drain(..=end).collect();
            log_line(&line[..end]);
        }
        while partial.len() >= MAX_LINE_BYTES {
            let piece: Vec<u8> = partial.drain(..MAX_LINE_BYTES).collect();
            log_line(&piece);
        }
    }
}

fn log_line(line: &[u8]) {
    let line = String::from_utf8_lossy(line);
    let line = line.trim_end_matches('\r');
    if !line.trim().is_empty() {
        tracing::info!(target: "neon_updater::update", output = %line, "update output");
    }
}

/// The process's standard output: the update's own container prints there, for the updater
/// that started it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stdout;

impl Output for Stdout {
    fn write(&self, bytes: &[u8]) {
        let mut stdout = std::io::stdout().lock();
        // Nothing can be done about a standard output that is gone.
        let _ = stdout.write_all(bytes);
        let _ = stdout.flush();
    }
}

/// A lock that a panic elsewhere left poisoned still guards bytes that are whole.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tail_keeps_the_end() {
        let tail = Tail::new(8);
        tail.write(b"0123");
        tail.write(b"456789");
        assert_eq!(tail.text(), "23456789");

        tail.write("abc€".as_bytes());
        assert_eq!(tail.text(), "89abc€");
    }

    #[test]
    fn a_tail_never_keeps_half_a_character() {
        let tail = Tail::new(8);
        tail.write("abc€".as_bytes());
        tail.write(b"1234567");
        // The bound cut into €, whose last byte is all that is left of it.
        assert_eq!(tail.text(), "1234567");
    }

    #[test]
    fn a_tail_holds_its_bound_of_text() {
        let tail = Tail::new(crate::protocol::MAX_LOG_BYTES);
        for _ in 0..5_000 {
            tail.write("Pulling server at 0.2.1 ✓\n".as_bytes());
        }
        let text = tail.text();
        assert!(text.len() <= crate::protocol::MAX_LOG_BYTES);
        assert!(text.len() > crate::protocol::MAX_LOG_BYTES - 4);
        assert!(text.ends_with("0.2.1 ✓\n"));
    }

    #[test]
    fn a_tail_drops_what_is_not_utf_8() {
        let tail = Tail::new(64);
        tail.write(b"ok \xff\xfe then ok");
        assert_eq!(tail.text(), "ok  then ok");
        assert!(tail.text().len() <= 64);
    }

    #[test]
    fn a_line_ends_with_a_newline() {
        let tail = Tail::new(64);
        tail.line("Updating server, web to 0.2.1");
        assert_eq!(tail.text(), "Updating server, web to 0.2.1\n");
    }

    #[test]
    fn a_recorder_keeps_the_end_of_lines_written_in_pieces() {
        let recorder = Recorder::new(16);
        recorder.write(b"first li");
        recorder.write(b"ne\nsecond line\nthe end");
        recorder.finish();
        assert_eq!(recorder.text(), "ond line\nthe end");
        assert!(lock(&recorder.partial).is_empty());
    }

    #[test]
    fn a_recorder_holds_no_more_than_a_line_of_a_line_that_never_ends() {
        let recorder = Recorder::new(16);
        for _ in 0..1000 {
            recorder.write(b"\r[=====>     ] 42%");
        }
        assert!(lock(&recorder.partial).len() < MAX_LINE_BYTES);
        assert_eq!(recorder.text(), "=====>     ] 42%");
    }
}

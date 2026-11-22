//! Where new blocks' ids come from.

/// Gives the id of each new block. The crate makes no ids itself, as it has no source of
/// randomness: the server passes one that makes random ids, and tests a [`SequentialIds`],
/// so their output stays the same from run to run.
///
/// Any `FnMut() -> String` is one.
pub trait IdSource {
    fn next_id(&mut self) -> String;
}

impl<F: FnMut() -> String> IdSource for F {
    fn next_id(&mut self) -> String {
        self()
    }
}

/// Ids made of a prefix and a counter: `block-1`, `block-2`, and so on.
#[derive(Debug, Clone)]
pub struct SequentialIds {
    prefix: String,
    next: u64,
}

impl SequentialIds {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            next: 1,
        }
    }
}

impl IdSource for SequentialIds {
    fn next_id(&mut self) -> String {
        let id = format!("{}-{}", self.prefix, self.next);
        self.next += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_ids_count_from_one() {
        let mut ids = SequentialIds::new("block");
        assert_eq!(ids.next_id(), "block-1");
        assert_eq!(ids.next_id(), "block-2");
    }

    #[test]
    fn closures_are_id_sources() {
        let mut n = 0;
        let mut ids = || {
            n += 1;
            format!("id{n}")
        };
        let source: &mut dyn IdSource = &mut ids;
        assert_eq!(source.next_id(), "id1");
        assert_eq!(source.next_id(), "id2");
    }
}

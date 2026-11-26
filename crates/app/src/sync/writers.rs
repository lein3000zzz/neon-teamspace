//! Who wrote what a live document gained since its latest snapshot, so a mention in its
//! text is credited to whoever wrote it rather than to whoever changed the page last.
//!
//! Each Yjs item has an id: the client that made it, and a clock that counts that client's
//! items. Every change the document takes, a client's update or an edit of the server's,
//! adds a range of each client's clock, from the state before it to the state after; the
//! change's author wrote what falls in it. A snapshot takes in what came before it, which
//! is credited since, so the ranges start again after each.

use std::collections::HashMap;

use uuid::Uuid;
use yrs::{ClientID, ID, StateVector};

/// The ranges of each client's clock that changes added since the latest snapshot, each
/// with its change's author, oldest first.
#[derive(Debug, Default)]
pub(super) struct Writers {
    ranges: HashMap<ClientID, Vec<Range>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Range {
    start: u32,
    end: u32,
    /// `None` for a logged change whose author's account is gone.
    author: Option<Uuid>,
}

impl Writers {
    /// Notes a change by `author` that took the document from `before` to `after`. A
    /// client's clock only grows, so its ranges stay in order; one that follows on from
    /// the last of the same author joins it, so typing adds one range, not one a key.
    pub(super) fn add(&mut self, before: &StateVector, after: &StateVector, author: Option<Uuid>) {
        for (client, &end) in after.iter() {
            let start = before.get(client);
            if end <= start {
                continue;
            }
            let ranges = self.ranges.entry(*client).or_default();
            match ranges.last_mut() {
                Some(last) if last.end == start && last.author == author => last.end = end,
                _ => ranges.push(Range { start, end, author }),
            }
        }
    }

    /// Who wrote the item of `id`, when a change since the snapshot added it and its author
    /// is known.
    pub(super) fn of(&self, id: &ID) -> Option<Uuid> {
        let ranges = self.ranges.get(&id.client)?;
        let at = ranges.partition_point(|range| range.end <= id.clock);
        ranges
            .get(at)
            .filter(|range| range.start <= id.clock)
            .and_then(|range| range.author)
    }

    /// A snapshot took everything in.
    pub(super) fn clear(&mut self) {
        self.ranges.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA: Uuid = Uuid::from_u128(1);
    const BEA: Uuid = Uuid::from_u128(2);

    fn state(clocks: &[(u64, u32)]) -> StateVector {
        let mut state = StateVector::default();
        for (client, clock) in clocks {
            state.set_max(ClientID::new(*client), *clock);
        }
        state
    }

    fn id(client: u64, clock: u32) -> ID {
        ID::new(ClientID::new(client), clock)
    }

    #[test]
    fn each_item_is_credited_to_the_change_that_added_it() {
        let mut writers = Writers::default();
        writers.add(&state(&[]), &state(&[(7, 4)]), Some(ADA));
        // Ada types on: her range grows.
        writers.add(&state(&[(7, 4)]), &state(&[(7, 9)]), Some(ADA));
        // Bea's change adds to her client and, relayed, to Ada's.
        writers.add(&state(&[(7, 9)]), &state(&[(7, 12), (8, 3)]), Some(BEA));
        // A change that adds nothing new changes nothing.
        writers.add(&state(&[(7, 12)]), &state(&[(7, 12)]), Some(ADA));
        assert_eq!(writers.ranges[&ClientID::new(7)].len(), 2);
        assert_eq!(writers.of(&id(7, 0)), Some(ADA));
        assert_eq!(writers.of(&id(7, 8)), Some(ADA));
        assert_eq!(writers.of(&id(7, 9)), Some(BEA));
        assert_eq!(writers.of(&id(7, 11)), Some(BEA));
        assert_eq!(writers.of(&id(8, 2)), Some(BEA));
        assert_eq!(writers.of(&id(7, 12)), None, "not added yet");
        assert_eq!(writers.of(&id(9, 0)), None, "a client never seen");

        writers.add(&state(&[(7, 12)]), &state(&[(7, 13)]), None);
        assert_eq!(
            writers.of(&id(7, 12)),
            None,
            "an author whose account is gone"
        );
        writers.clear();
        assert_eq!(writers.of(&id(7, 0)), None, "the snapshot took it in");
    }

    #[test]
    fn what_came_before_the_snapshot_is_not_credited() {
        let mut writers = Writers::default();
        // The document loaded with a snapshot up to clock 5 of client 7.
        writers.add(&state(&[(7, 5)]), &state(&[(7, 6)]), Some(BEA));
        assert_eq!(writers.of(&id(7, 4)), None);
        assert_eq!(writers.of(&id(7, 5)), Some(BEA));
    }
}

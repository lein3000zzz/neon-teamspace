//! Where an entry sits in an ordered list, such as a page among its siblings or a page among
//! an account's favorites: a key that sorts, byte by byte, between its neighbours' keys.
//!
//! A key is made of the 62 digits `0-9A-Za-z`, which ASCII orders as written, so the
//! database sorts keys with the `C` collation. It is an integer part, then a fraction:
//!
//! - The integer's first character tells its length. `a` to `z` start integers of 2 to 27
//!   characters, counting up from `a0`; `A` to `Z` start integers of 27 to 2 characters,
//!   counting down below it. Placing a key before the first or after the last entry steps
//!   the integer, so a list that grows at either end keeps keys of 2 or 3 characters for
//!   thousands of entries.
//! - The fraction holds what was placed between two keys whose integers follow each other:
//!   the midpoint of their fractions, which ends in any digit but `0`, so there is always
//!   room before it.
//!
//! A key is worked out from its neighbours alone, with nothing random, so the same intent
//! on the same list always gives the same key. Inserting at one spot again and again
//! lengthens keys by about one character for every six insertions; a list whose keys pass
//! [`MAX_LEN`] is given fresh ones with [`sequence`].

/// The digits, in the order their bytes sort.
const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = DIGITS.len();
const ZERO: u8 = DIGITS[0];
const HIGHEST: u8 = DIGITS[BASE - 1];

/// The key of an entry alone in its list.
pub const START: &str = "a0";

/// The longest key kept. A key longer than this is not saved: the list is given fresh keys
/// first.
pub const MAX_LEN: usize = 64;

/// The integer below which nothing fits: `A` then 26 zeros. It is no key, since nothing
/// could be placed before it.
const LOWEST_INTEGER: &[u8; 27] = b"A00000000000000000000000000";

/// Keys that cannot be ordered between.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidPosition {
    #[error("{0:?} is not a position key")]
    Malformed(String),
    #[error("{before:?} does not sort before {after:?}")]
    OutOfOrder { before: String, after: String },
    /// Past the last integer there is, or before the first: never reached in practice.
    #[error("no key is left beyond {0:?}")]
    Exhausted(String),
}

/// Checks that `key` is a key: digits only, an integer as long as its first character says,
/// and a fraction not ending in `0`.
pub fn check(key: &str) -> Result<(), InvalidPosition> {
    let malformed = || InvalidPosition::Malformed(key.to_owned());
    let bytes = key.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(|b| value(*b).is_some()) {
        return Err(malformed());
    }
    let integer = integer_len(bytes[0]).ok_or_else(malformed)?;
    if bytes.len() < integer || bytes == LOWEST_INTEGER || bytes[integer..].last() == Some(&ZERO) {
        return Err(malformed());
    }
    Ok(())
}

/// A key that sorts after `before` and before `after`. `None` is the end of the list on
/// that side, so `between(None, None)` is [`START`], `between(Some(last), None)` comes
/// after the last key and `between(None, Some(first))` before the first.
pub fn between(before: Option<&str>, after: Option<&str>) -> Result<String, InvalidPosition> {
    if let Some(key) = before {
        check(key)?;
    }
    if let Some(key) = after {
        check(key)?;
    }
    let key = match (before.map(str::as_bytes), after.map(str::as_bytes)) {
        (None, None) => START.as_bytes().to_vec(),
        (None, Some(after)) => before_key(after)?,
        (Some(before), None) => after_key(before),
        (Some(a), Some(b)) if a >= b => {
            return Err(InvalidPosition::OutOfOrder {
                before: text(a),
                after: text(b),
            });
        }
        (Some(before), Some(after)) => between_keys(before, after)?,
    };
    Ok(text(&key))
}

/// `count` keys in order, each the next integer after the one before, from [`START`]: fresh
/// keys for a list whose keys grew too long.
pub fn sequence(count: usize) -> Vec<String> {
    let mut keys: Vec<String> = Vec::with_capacity(count);
    for _ in 0..count {
        let next = match keys.last() {
            None => START.as_bytes().to_vec(),
            Some(last) => after_key(last.as_bytes()),
        };
        keys.push(text(&next));
    }
    keys
}

/// A key before `after`: the integer below its own, or its integer alone when it has a
/// fraction.
fn before_key(after: &[u8]) -> Result<Vec<u8>, InvalidPosition> {
    let (integer, fraction) = split(after);
    if integer == LOWEST_INTEGER {
        return Ok(join(integer, &midpoint(&[], Some(fraction))));
    }
    if !fraction.is_empty() {
        return Ok(integer.to_vec());
    }
    decrement(integer).ok_or_else(|| InvalidPosition::Exhausted(text(after)))
}

/// A key after `before`: the integer above its own, or past the last integer, a longer
/// fraction.
fn after_key(before: &[u8]) -> Vec<u8> {
    let (integer, fraction) = split(before);
    match increment(integer) {
        Some(next) => next,
        None => join(integer, &midpoint(fraction, None)),
    }
}

fn between_keys(before: &[u8], after: &[u8]) -> Result<Vec<u8>, InvalidPosition> {
    let (integer_a, fraction_a) = split(before);
    let (integer_b, fraction_b) = split(after);
    if integer_a == integer_b {
        return Ok(join(integer_a, &midpoint(fraction_a, Some(fraction_b))));
    }
    let next = increment(integer_a).ok_or_else(|| InvalidPosition::Exhausted(text(before)))?;
    if next.as_slice() < after {
        Ok(next)
    } else {
        Ok(join(integer_a, &midpoint(fraction_a, None)))
    }
}

/// A fraction strictly between `a` and `b`, where `a` sorts before `b`, or before the end
/// when `b` is `None`. Neither ends in `0`, and neither does the result; `a` may be empty,
/// the lowest fraction.
fn midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        // Keep what the two share, reading `a` as padded with zeros.
        let shared = b
            .iter()
            .zip(a.iter().copied().chain(std::iter::repeat(ZERO)))
            .take_while(|(digit_b, digit_a)| *digit_a == **digit_b)
            .count();
        if shared > 0 {
            let mut key = b[..shared].to_vec();
            key.extend(midpoint(
                a.get(shared..).unwrap_or_default(),
                Some(&b[shared..]),
            ));
            return key;
        }
    }
    let digit_a = a.first().map_or(0, |digit| digit_value(*digit));
    let digit_b = b
        .and_then(<[u8]>::first)
        .map_or(BASE, |digit| digit_value(*digit));
    if digit_b - digit_a > 1 {
        // A digit fits between them: the one halfway, rounded up.
        return vec![DIGITS[(digit_a + digit_b).div_ceil(2)]];
    }
    match b {
        // `b` goes on past its first digit, which alone is then between.
        Some(b) if b.len() > 1 => b[..1].to_vec(),
        // The digits are consecutive: keep `a`'s and go on after the rest of it.
        _ => {
            let mut key = vec![DIGITS[digit_a]];
            key.extend(midpoint(a.get(1..).unwrap_or_default(), None));
            key
        }
    }
}

/// The next integer, or `None` after the last one, `z` and 26 more `z`s.
fn increment(integer: &[u8]) -> Option<Vec<u8>> {
    let (head, digits) = integer.split_first()?;
    let mut digits = digits.to_vec();
    for digit in digits.iter_mut().rev() {
        if *digit == HIGHEST {
            *digit = ZERO;
        } else {
            *digit = DIGITS[digit_value(*digit) + 1];
            return Some(join(&[*head], &digits));
        }
    }
    // Every digit carried: the integer grows by one length class.
    match *head {
        b'Z' => Some(b"a0".to_vec()),
        b'z' => None,
        head => {
            let next = head + 1;
            if next > b'a' {
                digits.push(ZERO);
            } else {
                digits.pop();
            }
            Some(join(&[next], &digits))
        }
    }
}

/// The integer before, or `None` before the first one, [`LOWEST_INTEGER`].
fn decrement(integer: &[u8]) -> Option<Vec<u8>> {
    let (head, digits) = integer.split_first()?;
    let mut digits = digits.to_vec();
    for digit in digits.iter_mut().rev() {
        if *digit == ZERO {
            *digit = HIGHEST;
        } else {
            *digit = DIGITS[digit_value(*digit) - 1];
            return Some(join(&[*head], &digits));
        }
    }
    match *head {
        b'a' => Some(vec![b'Z', HIGHEST]),
        b'A' => None,
        head => {
            let next = head - 1;
            if next < b'Z' {
                digits.push(HIGHEST);
            } else {
                digits.pop();
            }
            Some(join(&[next], &digits))
        }
    }
}

/// How long an integer starting with `head` is.
fn integer_len(head: u8) -> Option<usize> {
    match head {
        b'a'..=b'z' => Some(usize::from(head - b'a') + 2),
        b'A'..=b'Z' => Some(usize::from(b'Z' - head) + 2),
        _ => None,
    }
}

/// A checked key's integer part and fraction.
fn split(key: &[u8]) -> (&[u8], &[u8]) {
    key.split_at(integer_len(key[0]).unwrap_or(key.len()).min(key.len()))
}

fn join(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut key = a.to_vec();
    key.extend_from_slice(b);
    key
}

fn value(digit: u8) -> Option<usize> {
    DIGITS.iter().position(|known| *known == digit)
}

/// The value of a digit of a checked key.
fn digit_value(digit: u8) -> usize {
    value(digit).unwrap_or(0)
}

/// Keys are ASCII digits, so their bytes are always text.
fn text(key: &[u8]) -> String {
    key.iter().map(|byte| char::from(*byte)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(before: Option<&str>, after: Option<&str>) -> String {
        between(before, after).unwrap_or_else(|error| panic!("{before:?}..{after:?}: {error}"))
    }

    #[test]
    fn keys_fall_where_the_reference_algorithm_puts_them() {
        for (before, after, want) in [
            (None, None, "a0"),
            (None, Some("a0"), "Zz"),
            (None, Some("Zz"), "Zy"),
            (Some("a0"), None, "a1"),
            (Some("a1"), None, "a2"),
            (Some("a0"), Some("a1"), "a0V"),
            (Some("a1"), Some("a2"), "a1V"),
            (Some("a0V"), Some("a1"), "a0l"),
            (Some("Zz"), Some("a0"), "ZzV"),
            (Some("Zz"), Some("a1"), "a0"),
            (None, Some("Y00"), "Xzzz"),
            (Some("bzz"), None, "c000"),
            (Some("a0"), Some("a0V"), "a0G"),
            (Some("a0"), Some("a0G"), "a08"),
            (Some("b125"), Some("b129"), "b127"),
            (Some("a0"), Some("a1V"), "a1"),
            (Some("Zz"), Some("a01"), "a0"),
            (None, Some("a0V"), "a0"),
            (None, Some("b999"), "b99"),
            (
                None,
                Some("A000000000000000000000000001"),
                "A000000000000000000000000000V",
            ),
            (
                Some("zzzzzzzzzzzzzzzzzzzzzzzzzzy"),
                None,
                "zzzzzzzzzzzzzzzzzzzzzzzzzzz",
            ),
            (
                Some("zzzzzzzzzzzzzzzzzzzzzzzzzzz"),
                None,
                "zzzzzzzzzzzzzzzzzzzzzzzzzzzV",
            ),
        ] {
            assert_eq!(key(before, after), want, "{before:?}..{after:?}");
        }
    }

    #[test]
    fn what_is_no_key_is_refused() {
        for bad in [
            "",
            "0",
            "a",
            "a00",
            "a0 ",
            "a-1",
            "é0",
            "A00000000000000000000000000",
        ] {
            assert_eq!(
                check(bad),
                Err(InvalidPosition::Malformed(bad.to_owned())),
                "{bad:?}"
            );
            assert!(between(Some(bad), None).is_err(), "{bad:?}");
            assert!(between(None, Some(bad)).is_err(), "{bad:?}");
        }
        assert!(between(Some("a00"), Some("a1")).is_err());
        assert_eq!(
            between(Some("a1"), Some("a0")),
            Err(InvalidPosition::OutOfOrder {
                before: "a1".to_owned(),
                after: "a0".to_owned()
            })
        );
        assert!(between(Some("a1"), Some("a1")).is_err(), "equal keys");
        for good in ["a0", "Zz", "a0V", "A000000000000000000000000000V", START] {
            assert_eq!(check(good), Ok(()), "{good:?}");
        }
    }

    #[test]
    fn the_ends_run_out_only_past_the_extreme_integers() {
        let last = "z".repeat(27);
        assert_eq!(key(Some(&last), None), format!("{last}V"));
        assert_eq!(
            key(Some(&format!("{last}V")), Some(&format!("{last}W"))),
            format!("{last}VV")
        );
        let first = format!("A{}1", "0".repeat(26));
        assert_eq!(key(None, Some(&first)), format!("A{}0V", "0".repeat(26)));
    }

    #[test]
    fn appending_and_prepending_keep_keys_short() {
        let mut keys = vec![START.to_owned()];
        for _ in 0..10_000 {
            let next = key(keys.last().map(String::as_str), None);
            keys.push(next);
        }
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(
            keys.iter().all(|key| key.len() <= 4),
            "appended keys stay short"
        );
        assert_eq!(keys[62], "b00");

        let mut first = START.to_owned();
        for _ in 0..10_000 {
            let before = key(None, Some(&first));
            assert!(before < first, "{before} sorts before {first}");
            first = before;
        }
        assert!(first.len() <= 4, "{first}");
    }

    #[test]
    fn inserting_between_neighbours_always_finds_room() {
        let mut low = START.to_owned();
        let high = key(Some(&low), None);
        // Always right after the same key: the worst case, which lengthens keys steadily.
        for round in 0..200 {
            let mid = key(Some(&low), Some(&high));
            assert!(
                low < mid && mid < high,
                "round {round}: {low} < {mid} < {high}"
            );
            assert_eq!(check(&mid), Ok(()));
            low = mid;
        }
        assert!(low.len() > 20, "{low}");

        let mut high = key(Some(START), None);
        for round in 0..200 {
            let mid = key(Some(START), Some(&high));
            assert!(
                START < mid.as_str() && mid < high,
                "round {round}: {mid} < {high}"
            );
            high = mid;
        }
    }

    #[test]
    fn a_list_built_by_random_looking_inserts_stays_ordered_and_reproducible() {
        let build = || {
            let mut list: Vec<String> = Vec::new();
            for step in 0_usize..500 {
                // A fixed scramble of where each entry goes, the same on every run.
                let at = if list.is_empty() {
                    0
                } else {
                    (step * 7919) % (list.len() + 1)
                };
                let before = at.checked_sub(1).map(|i| list[i].as_str());
                let after = list.get(at).map(String::as_str);
                let new = key(before, after);
                list.insert(at, new);
            }
            list
        };
        let list = build();

        assert!(list.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(list.iter().all(|key| check(key).is_ok()));
        assert_eq!(list, build(), "nothing random goes into a key");
    }

    #[test]
    fn a_sequence_is_fresh_short_keys_in_order() {
        assert_eq!(sequence(0), Vec::<String>::new());
        assert_eq!(sequence(3), ["a0", "a1", "a2"]);
        let keys = sequence(5_000);
        assert_eq!(keys.len(), 5_000);
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(keys.iter().all(|key| key.len() <= 4));
        assert!(key(Some(&keys[10]), Some(&keys[11])).len() <= MAX_LEN);
    }
}

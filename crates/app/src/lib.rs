//! Use cases.
//!
//! Use cases call the repositories directly and own their transactions. Traits exist only
//! for side effects that tests replace: mail, blob storage, realtime broadcast, rate limits.

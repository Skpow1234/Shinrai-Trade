//! Market-data journal, gap detection, and deterministic replay.
//!
//! On a sequence gap the feed is marked **degraded** and messages are not
//! applied until a snapshot restores continuity. Replay is deterministic:
//! the same journal yields the same consumer digest.

#![forbid(unsafe_code)]

mod checksum;
mod consumer;
mod error;
mod journal;
mod record;
mod replay;
mod synth;

pub use checksum::state_digest;
pub use consumer::{ApplyOutcome, FeedStatus, MdConsumerState};
pub use error::MdError;
pub use journal::MdJournal;
pub use record::{MdKind, MdRecord};
pub use replay::{replay, ReplayReport};
pub use synth::SyntheticFeed;

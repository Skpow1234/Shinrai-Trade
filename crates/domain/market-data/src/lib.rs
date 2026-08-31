//! Market-data journal, gap detection, deterministic replay, OHLCV bars, and L2 books.
//!
//! On a sequence gap the feed is marked **degraded** and messages are not
//! applied until a snapshot restores continuity. Replay is deterministic:
//! the same journal yields the same consumer digest.

#![forbid(unsafe_code)]

mod bar;
mod book;
mod checksum;
mod consumer;
mod error;
mod historical;
mod journal;
mod record;
mod replay;
mod synth;

pub use bar::{BarAggregator, BarInterval, BarStore, OhlcvBar};
pub use book::{
    BookApplyOutcome, BookChange, BookDelta, BookEngine, BookEvent, BookLevel, BookSide,
    BookSnapshot, BookStatus, L2Book,
};

pub use checksum::state_digest;
pub use consumer::{ApplyOutcome, FeedStatus, MdConsumerState};
pub use error::MdError;
pub use historical::{
    BarHistoryPage, BarHistoryQuery, HistoricalArchive, PageParams, TradeHistoryPage,
    TradeHistoryQuery, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT,
};
pub use journal::MdJournal;
pub use record::{MdKind, MdRecord};
pub use replay::{replay, ReplayReport};
pub use synth::SyntheticFeed;

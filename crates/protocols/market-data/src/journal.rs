//! Append-only raw vendor frames plus optional normalized records.

use shinrai_market_data::MdRecord;

use crate::vendor::VendorId;

/// One recorded inbound frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFrame {
    vendor: VendorId,
    received_logical: u64,
    payload: Vec<u8>,
    decoded: Option<MdRecord>,
}

impl RawFrame {
    /// Vendor that produced the frame.
    #[must_use]
    pub const fn vendor(&self) -> VendorId {
        self.vendor
    }

    /// Logical receive time supplied by the supervisor clock.
    #[must_use]
    pub const fn received_logical(&self) -> u64 {
        self.received_logical
    }

    /// Exact vendor bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Normalized record if decode succeeded.
    #[must_use]
    pub const fn decoded(&self) -> Option<MdRecord> {
        self.decoded
    }
}

/// Append-only raw journal (source for replay).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecordingJournal {
    frames: Vec<RawFrame>,
}

impl RecordingJournal {
    /// Empty journal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of frames.
    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Returns true if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Appends a frame and returns its index.
    pub fn append(
        &mut self,
        vendor: VendorId,
        received_logical: u64,
        payload: &[u8],
        decoded: Option<MdRecord>,
    ) -> usize {
        let index = self.frames.len();
        self.frames.push(RawFrame {
            vendor,
            received_logical,
            payload: payload.to_vec(),
            decoded,
        });
        index
    }

    /// Frames in insertion order.
    #[must_use]
    pub fn frames(&self) -> &[RawFrame] {
        &self.frames
    }
}

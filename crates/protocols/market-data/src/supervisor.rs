//! Connection supervisor: heartbeat, reconnect, gap → snapshot, stale clock.

use std::collections::HashMap;

use shinrai_instruments::{InstrumentId, InstrumentMaster};
use shinrai_market_data::{
    ApplyOutcome, BookApplyOutcome, BookEngine, BookEvent, MdConsumerState, MdJournal, MdRecord,
};

use crate::error::ProviderError;
use crate::journal::RecordingJournal;
use crate::vendor::{DecodedFrame, MarketDataVendor, SnapshotSpec, VendorId};

/// Commands the I/O layer must execute (no sockets in this crate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedCommand {
    /// Send this WebSocket subscribe payload.
    Subscribe {
        /// Vendor JSON bytes.
        payload: Vec<u8>,
    },
    /// Fetch a snapshot before applying further ticks.
    RequestSnapshot {
        /// Internal instrument.
        instrument_id: InstrumentId,
        /// Vendor product id.
        product_id: String,
        /// HTTP GET spec.
        spec: SnapshotSpec,
    },
    /// Reconnect after a disconnect.
    Reconnect {
        /// Attempt count starting at 1.
        attempt: u32,
        /// Logical delay before the next connect.
        delay_logical: u64,
    },
}

/// Events emitted while ingesting or ticking the clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorEvent {
    /// Sequenced record applied.
    Applied(MdRecord),
    /// Duplicate vendor sequence ignored.
    Duplicate {
        /// Instrument.
        instrument_id: InstrumentId,
    },
    /// Gap detected; ticks will not apply until snapshot.
    Gap {
        /// Instrument.
        instrument_id: InstrumentId,
        /// Expected sequence.
        expected: u64,
        /// Received sequence.
        got: u64,
    },
    /// Snapshot restored a healthy feed.
    SnapshotRecovered {
        /// Instrument.
        instrument_id: InstrumentId,
    },
    /// Tick ignored because the feed is degraded.
    IgnoredDegraded {
        /// Instrument.
        instrument_id: InstrumentId,
    },
    /// Heartbeat refreshed liveness.
    Heartbeat {
        /// Instrument.
        instrument_id: InstrumentId,
    },
    /// No message within the SLA.
    Stale {
        /// Instrument.
        instrument_id: InstrumentId,
        /// Logical time since last message.
        silent_for: u64,
    },
    /// Book rebuilt from snapshot.
    BookRebuilt {
        /// Instrument.
        instrument_id: InstrumentId,
        /// Book digest after rebuild.
        checksum: u64,
    },
    /// L2 delta applied.
    BookDeltaApplied {
        /// Instrument.
        instrument_id: InstrumentId,
        /// Book digest.
        checksum: u64,
    },
    /// Vendor control / unusable frame (still recorded raw).
    Skipped,
}

/// Result of ingesting one stream frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestOutcome {
    /// Domain / health events.
    pub events: Vec<SupervisorEvent>,
    /// I/O commands (snapshot on gap).
    pub commands: Vec<FeedCommand>,
}

/// Supervises one vendor session.
#[derive(Debug, Clone)]
pub struct FeedSupervisor {
    consumer: MdConsumerState,
    normalized: MdJournal,
    raw: RecordingJournal,
    subscriptions: Vec<(InstrumentId, String)>,
    last_seen: HashMap<InstrumentId, u64>,
    sla_logical: u64,
    reconnect_attempt: u32,
    connected: bool,
    books: BookEngine,
}

impl FeedSupervisor {
    /// Creates a supervisor with a logical silence SLA (e.g. 30).
    #[must_use]
    pub fn new(sla_logical: u64) -> Self {
        Self {
            consumer: MdConsumerState::new(),
            normalized: MdJournal::new(),
            raw: RecordingJournal::new(),
            subscriptions: Vec::new(),
            last_seen: HashMap::new(),
            sla_logical,
            reconnect_attempt: 0,
            connected: false,
            books: BookEngine::new(),
        }
    }

    /// Watches an instrument mapped to a vendor product id.
    pub fn watch(&mut self, instrument_id: InstrumentId, product_id: impl Into<String>) {
        let product_id = product_id.into();
        if self
            .subscriptions
            .iter()
            .any(|(id, _)| *id == instrument_id)
        {
            return;
        }
        self.subscriptions.push((instrument_id, product_id));
    }

    /// Consumer projection.
    #[must_use]
    pub const fn consumer(&self) -> &MdConsumerState {
        &self.consumer
    }

    /// Normalized sequenced journal.
    #[must_use]
    pub const fn normalized(&self) -> &MdJournal {
        &self.normalized
    }

    /// Raw vendor journal.
    #[must_use]
    pub const fn raw(&self) -> &RecordingJournal {
        &self.raw
    }

    /// Local L2 books.
    #[must_use]
    pub const fn books(&self) -> &BookEngine {
        &self.books
    }

    /// Session currently connected.
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        self.connected
    }

    /// Called after a successful transport connect.
    pub fn on_connected(&mut self, vendor: &impl MarketDataVendor, now: u64) -> Vec<FeedCommand> {
        self.connected = true;
        self.reconnect_attempt = 0;
        let mut commands = Vec::new();
        let product_ids: Vec<String> = self.subscriptions.iter().map(|(_, p)| p.clone()).collect();
        if !product_ids.is_empty() {
            commands.push(FeedCommand::Subscribe {
                payload: vendor.subscribe_message(&product_ids),
            });
        }
        for (instrument_id, product_id) in &self.subscriptions {
            self.consumer.mark_degraded(*instrument_id);
            self.books.invalidate(*instrument_id);
            self.last_seen.insert(*instrument_id, now);
            commands.push(FeedCommand::RequestSnapshot {
                instrument_id: *instrument_id,
                product_id: product_id.clone(),
                spec: vendor.snapshot_spec(product_id),
            });
        }
        commands
    }

    /// Called when the transport drops. Marks every watched instrument degraded.
    pub fn on_disconnect(&mut self) -> Vec<FeedCommand> {
        self.connected = false;
        for (instrument_id, _) in &self.subscriptions {
            self.consumer.mark_degraded(*instrument_id);
            self.books.invalidate(*instrument_id);
        }
        self.reconnect_attempt = self.reconnect_attempt.saturating_add(1);
        vec![FeedCommand::Reconnect {
            attempt: self.reconnect_attempt,
            delay_logical: reconnect_delay(self.reconnect_attempt),
        }]
    }

    /// Ingests one WebSocket (or recorded) frame.
    pub fn ingest(
        &mut self,
        vendor: &impl MarketDataVendor,
        master: &InstrumentMaster,
        now: u64,
        raw: &[u8],
    ) -> IngestOutcome {
        let Ok(decoded) = vendor.decode_stream(raw, now, master) else {
            self.raw.append(vendor.id(), now, raw, None);
            return IngestOutcome {
                events: vec![SupervisorEvent::Skipped],
                commands: Vec::new(),
            };
        };

        match decoded {
            DecodedFrame::Control => {
                self.raw.append(vendor.id(), now, raw, None);
                IngestOutcome {
                    events: vec![SupervisorEvent::Skipped],
                    commands: Vec::new(),
                }
            }
            DecodedFrame::Heartbeat { instrument_id } => {
                self.raw.append(vendor.id(), now, raw, None);
                self.last_seen.insert(instrument_id, now);
                IngestOutcome {
                    events: vec![SupervisorEvent::Heartbeat { instrument_id }],
                    commands: Vec::new(),
                }
            }
            DecodedFrame::Record(record) => self.apply_record(vendor, now, raw, record),
            DecodedFrame::Book(event) => self.apply_book(now, raw, vendor.id(), &event),
        }
    }

    /// Applies a snapshot HTTP body for a watched instrument.
    ///
    /// # Errors
    ///
    /// Returns decode or consumer validation errors.
    pub fn ingest_snapshot(
        &mut self,
        vendor: &impl MarketDataVendor,
        master: &InstrumentMaster,
        now: u64,
        product_id: &str,
        raw: &[u8],
    ) -> Result<IngestOutcome, ProviderError> {
        let record = vendor.decode_snapshot(raw, now, product_id, master)?;
        let mut outcome = self.apply_record(vendor, now, raw, record);
        if let Ok(event) = vendor.decode_book_snapshot(raw, now, product_id, master) {
            let book_events = self.apply_book_event(&event);
            outcome.events.extend(book_events);
        }
        Ok(outcome)
    }

    /// Emits stale alerts for silence beyond the SLA. Does not degrade sequence.
    #[must_use]
    pub fn on_clock(&self, now: u64) -> Vec<SupervisorEvent> {
        if !self.connected {
            return Vec::new();
        }
        let mut events = Vec::new();
        for (instrument_id, _) in &self.subscriptions {
            let last = self.last_seen.get(instrument_id).copied().unwrap_or(now);
            let silent_for = now.saturating_sub(last);
            if silent_for > self.sla_logical {
                events.push(SupervisorEvent::Stale {
                    instrument_id: *instrument_id,
                    silent_for,
                });
            }
        }
        events
    }

    fn apply_record(
        &mut self,
        vendor: &impl MarketDataVendor,
        now: u64,
        raw: &[u8],
        record: MdRecord,
    ) -> IngestOutcome {
        self.raw.append(vendor.id(), now, raw, Some(record));
        if self.normalized.append(record).is_err() {
            return IngestOutcome {
                events: vec![SupervisorEvent::Skipped],
                commands: Vec::new(),
            };
        }
        self.last_seen.insert(record.instrument_id(), now);

        let Ok(outcome) = self.consumer.apply(record) else {
            return IngestOutcome {
                events: vec![SupervisorEvent::Skipped],
                commands: Vec::new(),
            };
        };

        let instrument_id = record.instrument_id();
        match outcome {
            ApplyOutcome::Applied => IngestOutcome {
                events: vec![SupervisorEvent::Applied(record)],
                commands: Vec::new(),
            },
            ApplyOutcome::Duplicate => IngestOutcome {
                events: vec![SupervisorEvent::Duplicate { instrument_id }],
                commands: Vec::new(),
            },
            ApplyOutcome::GapDetected { expected, got } => {
                self.books.invalidate(instrument_id);
                let mut commands = Vec::new();
                if let Some(product_id) = self.product_id(instrument_id) {
                    let spec = vendor.snapshot_spec(&product_id);
                    commands.push(FeedCommand::RequestSnapshot {
                        instrument_id,
                        product_id,
                        spec,
                    });
                }
                IngestOutcome {
                    events: vec![SupervisorEvent::Gap {
                        instrument_id,
                        expected,
                        got,
                    }],
                    commands,
                }
            }
            ApplyOutcome::IgnoredDegraded => IngestOutcome {
                events: vec![SupervisorEvent::IgnoredDegraded { instrument_id }],
                commands: Vec::new(),
            },
            ApplyOutcome::SnapshotRecovered => IngestOutcome {
                events: vec![SupervisorEvent::SnapshotRecovered { instrument_id }],
                commands: Vec::new(),
            },
        }
    }

    fn apply_book(
        &mut self,
        now: u64,
        raw: &[u8],
        vendor: VendorId,
        event: &BookEvent,
    ) -> IngestOutcome {
        self.raw.append(vendor, now, raw, None);
        let instrument_id = match event {
            BookEvent::Snapshot(s) => s.instrument_id(),
            BookEvent::Delta(d) => d.instrument_id(),
        };
        self.last_seen.insert(instrument_id, now);
        IngestOutcome {
            events: self.apply_book_event(event),
            commands: Vec::new(),
        }
    }

    fn apply_book_event(&mut self, event: &BookEvent) -> Vec<SupervisorEvent> {
        let Ok(outcome) = self.books.apply(event) else {
            return vec![SupervisorEvent::Skipped];
        };
        let instrument_id = match event {
            BookEvent::Snapshot(s) => s.instrument_id(),
            BookEvent::Delta(d) => d.instrument_id(),
        };
        match outcome {
            BookApplyOutcome::Applied { checksum } => {
                let ev = match event {
                    BookEvent::Snapshot(_) => SupervisorEvent::BookRebuilt {
                        instrument_id,
                        checksum,
                    },
                    BookEvent::Delta(_) => SupervisorEvent::BookDeltaApplied {
                        instrument_id,
                        checksum,
                    },
                };
                vec![ev]
            }
            BookApplyOutcome::Duplicate | BookApplyOutcome::IgnoredInvalidated => {
                vec![SupervisorEvent::Skipped]
            }
            BookApplyOutcome::GapInvalidated { expected, got } => vec![SupervisorEvent::Gap {
                instrument_id,
                expected,
                got,
            }],
        }
    }

    fn product_id(&self, instrument_id: InstrumentId) -> Option<String> {
        self.subscriptions
            .iter()
            .find(|(id, _)| *id == instrument_id)
            .map(|(_, p)| p.clone())
    }
}

fn reconnect_delay(attempt: u32) -> u64 {
    let shift = attempt.min(5);
    1_u64 << shift
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coinbase::CoinbaseExchange;
    use shinrai_instruments::{btc_usd, phase1_master};
    use shinrai_market_data::{state_digest, BookEvent, BookStatus, FeedStatus};

    fn ready(now: u64) -> (FeedSupervisor, CoinbaseExchange, InstrumentMaster) {
        let vendor = CoinbaseExchange;
        let master = phase1_master();
        let mut sup = FeedSupervisor::new(30);
        sup.watch(btc_usd().id(), "BTC-USD");
        let commands = sup.on_connected(&vendor, now);
        assert!(commands
            .iter()
            .any(|c| matches!(c, FeedCommand::Subscribe { .. })));
        assert!(commands
            .iter()
            .any(|c| matches!(c, FeedCommand::RequestSnapshot { .. })));
        (sup, vendor, master)
    }

    fn snapshot_body(seq: u64) -> Vec<u8> {
        format!(
            r#"{{"sequence":{seq},"bids":[["65000.10","1.0",1]],"asks":[["65000.12","1.0",1]]}}"#
        )
        .into_bytes()
    }

    fn ticker(seq: u64) -> Vec<u8> {
        format!(r#"{{"type":"ticker","sequence":{seq},"product_id":"BTC-USD","price":"65000.12"}}"#)
            .into_bytes()
    }

    #[test]
    fn connect_waits_for_snapshot_then_applies() {
        let (mut sup, vendor, master) = ready(0);
        let skipped = sup.ingest(&vendor, &master, 1, &ticker(50_001));
        assert!(matches!(
            skipped.events[0],
            SupervisorEvent::IgnoredDegraded { .. }
        ));
        let recovered = sup
            .ingest_snapshot(&vendor, &master, 2, "BTC-USD", &snapshot_body(50_000))
            .expect("snap");
        assert!(matches!(
            recovered.events[0],
            SupervisorEvent::SnapshotRecovered { .. }
        ));
        assert_eq!(
            sup.consumer().feed_status(btc_usd().id()),
            FeedStatus::Healthy
        );
        let applied = sup.ingest(&vendor, &master, 3, &ticker(50_001));
        assert!(matches!(applied.events[0], SupervisorEvent::Applied(_)));
        assert_eq!(sup.raw().len(), 3);
    }

    #[test]
    fn gap_requests_snapshot_and_does_not_apply() {
        let (mut sup, vendor, master) = ready(0);
        sup.ingest_snapshot(&vendor, &master, 1, "BTC-USD", &snapshot_body(10))
            .expect("snap");
        let gap = sup.ingest(&vendor, &master, 2, &ticker(12));
        assert!(matches!(
            gap.events[0],
            SupervisorEvent::Gap {
                expected: 11,
                got: 12,
                ..
            }
        ));
        assert!(matches!(
            gap.commands[0],
            FeedCommand::RequestSnapshot { .. }
        ));
        assert!(matches!(
            sup.consumer().feed_status(btc_usd().id()),
            FeedStatus::Degraded { missing_from: 11 }
        ));
        assert_eq!(
            sup.consumer().last_price(btc_usd().id()).unwrap().scaled(),
            6_500_010
        );
    }

    #[test]
    fn reconnect_restores_via_snapshot() {
        let (mut sup, vendor, master) = ready(0);
        sup.ingest_snapshot(&vendor, &master, 1, "BTC-USD", &snapshot_body(10))
            .expect("snap");
        sup.ingest(&vendor, &master, 2, &ticker(11));
        let reconnect = sup.on_disconnect();
        assert!(matches!(
            reconnect[0],
            FeedCommand::Reconnect {
                attempt: 1,
                delay_logical: 2
            }
        ));
        let cmds = sup.on_connected(&vendor, 5);
        assert!(cmds
            .iter()
            .any(|c| matches!(c, FeedCommand::RequestSnapshot { .. })));
        let live = sup.ingest(&vendor, &master, 6, &ticker(40));
        assert!(matches!(
            live.events[0],
            SupervisorEvent::IgnoredDegraded { .. }
        ));
        let snap = sup
            .ingest_snapshot(&vendor, &master, 7, "BTC-USD", &snapshot_body(40))
            .expect("snap");
        assert!(matches!(
            snap.events[0],
            SupervisorEvent::SnapshotRecovered { .. }
        ));
        assert_eq!(
            sup.consumer().feed_status(btc_usd().id()),
            FeedStatus::Healthy
        );
    }

    #[test]
    fn heartbeat_prevents_stale_alert() {
        let (mut sup, vendor, master) = ready(0);
        sup.ingest_snapshot(&vendor, &master, 0, "BTC-USD", &snapshot_body(10))
            .expect("snap");
        let hb = br#"{"type":"heartbeat","sequence":10,"product_id":"BTC-USD","last_trade_id":1}"#;
        sup.ingest(&vendor, &master, 10, hb);
        assert!(sup.on_clock(30).is_empty());
        let stale = sup.on_clock(41);
        assert!(matches!(
            stale[0],
            SupervisorEvent::Stale { silent_for: 31, .. }
        ));
    }

    #[test]
    fn duplicate_seq_does_not_change_price() {
        let (mut sup, vendor, master) = ready(0);
        sup.ingest_snapshot(&vendor, &master, 1, "BTC-USD", &snapshot_body(10))
            .expect("snap");
        sup.ingest(&vendor, &master, 2, &ticker(11));
        let price = sup.consumer().last_price(btc_usd().id()).unwrap();
        let dup = sup.ingest(&vendor, &master, 3, &ticker(11));
        assert!(matches!(dup.events[0], SupervisorEvent::Duplicate { .. }));
        assert_eq!(sup.consumer().last_price(btc_usd().id()).unwrap(), price);
        assert_eq!(sup.raw().len(), 3);
    }

    #[test]
    fn same_raw_frames_same_digest() {
        let (mut a, vendor, master) = ready(0);
        let (mut b, _, _) = ready(0);
        let snap = snapshot_body(10);
        let t11 = ticker(11);
        let t12 = ticker(12);
        for sup in [&mut a, &mut b] {
            sup.ingest_snapshot(&vendor, &master, 1, "BTC-USD", &snap)
                .expect("snap");
            sup.ingest(&vendor, &master, 2, &t11);
            sup.ingest(&vendor, &master, 3, &t12);
        }
        assert_eq!(state_digest(a.consumer()), state_digest(b.consumer()));
        assert_eq!(a.raw().len(), b.raw().len());
    }

    #[test]
    fn l2_gap_clears_book_then_snapshot_checksum_matches() {
        let (mut sup, vendor, master) = ready(0);
        let body = snapshot_body(10);
        let recovered = sup
            .ingest_snapshot(&vendor, &master, 1, "BTC-USD", &body)
            .expect("snap");
        assert!(recovered
            .events
            .iter()
            .any(|e| matches!(e, SupervisorEvent::BookRebuilt { .. })));
        let book = sup.books().book(btc_usd().id()).expect("book");
        assert_eq!(book.status(), BookStatus::Healthy);
        let checksum = book.checksum();
        let BookEvent::Snapshot(snap) = vendor
            .decode_book_snapshot(&body, 1, "BTC-USD", &master)
            .expect("decode")
        else {
            panic!("snap");
        };
        assert_eq!(checksum, snap.checksum());

        sup.ingest(&vendor, &master, 2, &ticker(12));
        assert_eq!(
            sup.books().book(btc_usd().id()).expect("b").status(),
            BookStatus::Invalidated
        );
        let skipped = sup.ingest(
            &vendor,
            &master,
            3,
            br#"{"type":"l2update","product_id":"BTC-USD","changes":[["buy","65000.10","2.0"]]}"#,
        );
        assert!(matches!(skipped.events[0], SupervisorEvent::Skipped));

        let rebuilt = snapshot_body(12);
        sup.ingest_snapshot(&vendor, &master, 4, "BTC-USD", &rebuilt)
            .expect("rebuild");
        let book = sup.books().book(btc_usd().id()).expect("book");
        assert_eq!(book.status(), BookStatus::Healthy);
        let BookEvent::Snapshot(snap) = vendor
            .decode_book_snapshot(&rebuilt, 4, "BTC-USD", &master)
            .expect("d2")
        else {
            panic!("snap2");
        };
        assert_eq!(book.checksum(), snap.checksum());
    }
}

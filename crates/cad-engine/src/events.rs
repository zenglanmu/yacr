//! Bounded engine event trace ring (P4).
//!
//! The ring is a UI-independent, owner-thread structure shared by the host and
//! its adapters. It stores one shared `Arc<EventEnvelope>` per record: clones
//! are shallow and never deep-copy a `ChangeSet`. Subscribers are cursors into
//! the same shared records, so a lagging subscriber is normal ring overwrite,
//! never a business failure.
//!
//! Sequence numbers and subscription ids are allocated with checked arithmetic.
//! The sequence space is bounded by a reservation ("permit") model: a request
//! reserves the maximum number of trace events it may publish up front, so the
//! ring can never oversell headroom and never truncates a committed batch.

use crate::protocol::{RequestError, RequestId, RequestOrigin, SharedRequestResult};
use cad_app::{CommandId, CommandOutcome};
use cad_db::ChangeSet;
use cad_domain::{CadError, CadResult, DocumentId, Revision, SelectionRef, TransactionId};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

/// Default number of retained event records.
pub const DEFAULT_RING_CAPACITY: usize = 1024;
/// Maximum live subscriptions.
pub const MAX_SUBSCRIBERS: usize = 32;
/// Maximum events returned by one poll; larger limits are clamped.
pub const MAX_POLL: usize = 256;
/// Trace allowance reserved by each accepted request, in sequence slots. The
/// ring itself does not pre-reserve; a request owns its permit and returns
/// unused slots on finish.
pub const TRACE_ALLOWANCE: u64 = 16;

/// Monotonic, non-reused subscription identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubscriptionId(pub u64);

/// Minimal request classification for `RequestStarted`; no business state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Command,
    Interaction,
    Api,
}

/// Minimal tool classification consistent with the app `ToolState`, without
/// exposing active business state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Idle,
    Selecting,
    Drawing,
    Measuring,
    Panning,
}

/// One engine trace event. Payloads are shared (`Arc`) so publishing and polling
/// never deep-copy a `ChangeSet` or command outcome.
#[derive(Debug)]
pub enum EngineEvent {
    RequestStarted {
        classification: RequestKind,
    },
    RequestFinished {
        result: SharedRequestResult,
    },
    CommandStarted {
        command: CommandId,
    },
    CommandFinished {
        command: CommandId,
        result: Arc<Result<Arc<CommandOutcome>, CadError>>,
    },
    EntitiesChanged {
        changes: Arc<ChangeSet>,
    },
    SelectionChanged {
        revision: u64,
        refs: Vec<SelectionRef>,
    },
    ToolChanged {
        generation: u64,
        kind: ToolKind,
    },
}

/// A published trace record. `sequence` is assigned by the ring on publish.
#[derive(Debug)]
pub struct EventEnvelope {
    pub sequence: u64,
    pub request_id: Option<RequestId>,
    pub origin: RequestOrigin,
    pub document: DocumentId,
    pub generation: u64,
    pub revision: Revision,
    pub transaction: Option<TransactionId>,
    pub event: EngineEvent,
}

/// Result of one subscription poll.
#[derive(Debug, Clone)]
pub enum EventPoll {
    Events(Vec<Arc<EventEnvelope>>),
    /// The cursor fell behind the retained window; it advances to `next_sequence`
    /// (the oldest retained sequence) and the next poll delivers real events.
    Lagged {
        missed: u64,
        next_sequence: u64,
    },
}

/// Host-facing ring watermark for snapshots and resync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EventWatermark {
    pub oldest_sequence: Option<u64>,
    pub next_sequence: u64,
    pub records: usize,
    pub capacity: usize,
    pub exhausted: bool,
}

struct Subscription {
    id: SubscriptionId,
    next: u64,
}

struct RingCore {
    records: VecDeque<Arc<EventEnvelope>>,
    capacity: usize,
    next_sequence: u64,
    /// Sequence slots reserved by outstanding permits plus the startup reserve.
    reserved: u64,
    exhausted: bool,
    subscribers: Vec<Option<Subscription>>,
    next_subscription_id: u64,
}

fn exhausted_error() -> RequestError {
    RequestError::Execution(CadError::Invariant("event sequence space exhausted".into()))
}

impl RingCore {
    fn reserve(&mut self, count: u64) -> Result<(), RequestError> {
        if self.exhausted {
            return Err(exhausted_error());
        }
        let headroom = u64::MAX - self.next_sequence;
        let Some(reserved) = self.reserved.checked_add(count) else {
            // New admission cannot be fulfilled; close admissions permanently.
            self.exhausted = true;
            return Err(exhausted_error());
        };
        if reserved > headroom {
            // The reservation cannot be fulfilled; close new admissions. This
            // only refuses *new* requests: permits already reserved keep their
            // promised slots and must still publish.
            self.exhausted = true;
            return Err(exhausted_error());
        }
        self.reserved = reserved;
        Ok(())
    }

    fn release(&mut self, count: u64) {
        self.reserved = self.reserved.saturating_sub(count);
    }

    /// Publish a validated batch atomically.
    ///
    /// The whole batch is validated (checked sequence headroom for every event)
    /// before any mutation, so a rejected batch changes nothing. This never
    /// consults the admission-exhausted flag: permits that were already reserved
    /// must always be fulfilled. A genuine overflow is refused rather than
    /// wrapped.
    fn publish_batch(&mut self, envelopes: Vec<EventEnvelope>) -> Result<Vec<u64>, RequestError> {
        let count = envelopes.len() as u64;
        let Some(end) = self.next_sequence.checked_add(count) else {
            return Err(exhausted_error());
        };
        let start = self.next_sequence;
        let mut sequences = Vec::with_capacity(envelopes.len());
        for (offset, mut envelope) in envelopes.into_iter().enumerate() {
            let sequence = start + offset as u64;
            envelope.sequence = sequence;
            sequences.push(sequence);
            self.records.push_back(Arc::new(envelope));
        }
        while self.records.len() > self.capacity {
            self.records.pop_front();
        }
        self.next_sequence = end;
        self.reserved = self.reserved.saturating_sub(count);
        Ok(sequences)
    }

    fn subscribe(&mut self) -> CadResult<SubscriptionId> {
        let slot = match self.subscribers.iter().position(Option::is_none) {
            Some(slot) => slot,
            None if self.subscribers.len() < MAX_SUBSCRIBERS => {
                self.subscribers.push(None);
                self.subscribers.len() - 1
            }
            None => {
                return Err(CadError::ResourceMissing(
                    "event subscriber slots exhausted".into(),
                ))
            }
        };
        let Some(next_id) = self.next_subscription_id.checked_add(1) else {
            return Err(CadError::Invariant(
                "subscription id space exhausted".into(),
            ));
        };
        let id = SubscriptionId(self.next_subscription_id);
        self.next_subscription_id = next_id;
        self.subscribers[slot] = Some(Subscription {
            id,
            next: self.next_sequence,
        });
        Ok(id)
    }

    fn unsubscribe(&mut self, id: SubscriptionId) -> CadResult<()> {
        let slot = self
            .subscribers
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|sub| sub.id == id));
        match slot {
            Some(slot) => {
                *slot = None;
                Ok(())
            }
            None => Err(CadError::ResourceMissing(
                "unknown event subscription".into(),
            )),
        }
    }

    fn poll(&mut self, id: SubscriptionId, limit: usize) -> CadResult<EventPoll> {
        // 1. Validate the subscription first: an unknown id is always an explicit
        //    error, even when the ring is exhausted.
        let Some(slot) = self
            .subscribers
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|sub| sub.id == id))
        else {
            return Err(CadError::ResourceMissing(
                "unknown event subscription".into(),
            ));
        };
        let sub = slot.as_mut().expect("matched subscription slot");

        // 2. Lag takes precedence: advance the cursor to the oldest retained
        //    record and report the gap.
        let oldest = self.records.front().map(|record| record.sequence);
        if let Some(oldest) = oldest.filter(|oldest| sub.next < *oldest) {
            let missed = oldest - sub.next;
            sub.next = oldest;
            return Ok(EventPoll::Lagged {
                missed,
                next_sequence: oldest,
            });
        }

        // 3. Unread retained events are delivered normally. `limit == 0` yields an
        //    empty batch and leaves the cursor untouched.
        let events: Vec<Arc<EventEnvelope>> = self
            .records
            .iter()
            .filter(|record| record.sequence >= sub.next)
            .take(limit)
            .cloned()
            .collect();
        if let Some(last) = events.last() {
            sub.next = last.sequence.saturating_add(1);
        }

        // 4. Exhausted is reported only when this poll delivered no event and the
        //    actual cursor has reached the tail with no live permit. A non-empty
        //    batch (which may have advanced the cursor to the tail) is always
        //    returned first, so the final batch is never dropped; the caller sees
        //    Exhausted on the *next* poll. `limit == 0` never advances the cursor,
        //    so unread events are not mistaken for the tail. This marks admission
        //    closed only, never that the traced work did not execute.
        if events.is_empty()
            && self.exhausted
            && sub.next == self.next_sequence
            && self.reserved == 0
        {
            return Err(CadError::InvalidInput(
                "event ring exhausted: admission closed and all traces drained".into(),
            ));
        }
        Ok(EventPoll::Events(events))
    }

    fn watermark(&self) -> EventWatermark {
        EventWatermark {
            oldest_sequence: self.records.front().map(|record| record.sequence),
            next_sequence: self.next_sequence,
            records: self.records.len(),
            capacity: self.capacity,
            exhausted: self.exhausted,
        }
    }
}

/// Owned reservation of `remaining` sequence slots.
///
/// The permit borrows the ring interior only inside `publish`/`drop`; no borrow
/// is held across the caller's business execution. Unused reservation is
/// released on drop. A permit can never publish more events than reserved.
pub struct TracePermit {
    core: Rc<RefCell<RingCore>>,
    remaining: u64,
}

impl TracePermit {
    /// Sequence slots still reservable through this permit.
    pub fn remaining(&self) -> u64 {
        self.remaining
    }

    /// Publish one event; delegates to [`TracePermit::publish_batch`].
    pub fn publish(&mut self, envelope: EventEnvelope) -> Result<u64, RequestError> {
        let mut sequences = self.publish_batch(vec![envelope])?;
        Ok(sequences.pop().expect("one published sequence"))
    }

    /// Publish a whole batch under a single ring borrow.
    ///
    /// The batch is validated up front: `count <= remaining` and enough checked
    /// sequence headroom for the entire batch. Only after validation are the
    /// records, sequence and reservation updated, so a rejected batch (for
    /// example 17 events against a 16-event allowance) changes nothing and can
    /// never partially commit. Payloads stay behind `Arc` (shallow, with no deep
    /// `ChangeSet` copy).
    pub fn publish_batch(
        &mut self,
        envelopes: Vec<EventEnvelope>,
    ) -> Result<Vec<u64>, RequestError> {
        let count = envelopes.len() as u64;
        if count > self.remaining {
            return Err(RequestError::Execution(CadError::Invariant(
                "trace batch exceeds permit reservation".into(),
            )));
        }
        let sequences = self.core.borrow_mut().publish_batch(envelopes)?;
        self.remaining -= count;
        Ok(sequences)
    }
}

impl Drop for TracePermit {
    fn drop(&mut self) {
        if self.remaining > 0 {
            self.core.borrow_mut().release(self.remaining);
        }
    }
}

/// Bounded, owner-thread event ring shared with adapters.
pub struct EventRing {
    core: Rc<RefCell<RingCore>>,
}

impl Default for EventRing {
    fn default() -> Self {
        Self::new()
    }
}

impl EventRing {
    /// A ring with the default capacity. `reserved` starts at zero: the ring
    /// holds no startup reservation; each accepted request reserves its own
    /// `TRACE_ALLOWANCE` permit.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_RING_CAPACITY)
    }

    /// A ring with an explicit retained-record capacity. Capacity is at least 1.
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity >= 1, "event ring capacity must be at least 1");
        Self {
            core: Rc::new(RefCell::new(RingCore {
                records: VecDeque::new(),
                capacity,
                next_sequence: 0,
                reserved: 0,
                exhausted: false,
                subscribers: Vec::new(),
                next_subscription_id: 0,
            })),
        }
    }

    /// Reserve `count` sequence slots for a request's trace. Reserving does not
    /// advance the sequence; only publishing does (checked).
    pub fn reserve_trace(&self, count: u64) -> Result<TracePermit, RequestError> {
        self.core.borrow_mut().reserve(count)?;
        Ok(TracePermit {
            core: Rc::clone(&self.core),
            remaining: count,
        })
    }

    pub fn subscribe(&self) -> CadResult<SubscriptionId> {
        self.core.borrow_mut().subscribe()
    }

    /// Poll at most `limit` (clamped to `MAX_POLL`) events for a subscription.
    ///
    /// Order: unknown id is an explicit error; lag is reported before retained
    /// events; unread retained events are delivered normally. When admissions
    /// are permanently closed, an explicit `Err` is returned only once the
    /// actual cursor has reached the tail and no live permit remains (`reserved
    /// == 0`); a live permit or unread event still yields events. This marks
    /// admission closed only and never implies the traced work did not execute.
    pub fn poll_events(&self, id: SubscriptionId, limit: usize) -> CadResult<EventPoll> {
        self.core.borrow_mut().poll(id, limit.min(MAX_POLL))
    }

    pub fn unsubscribe(&self, id: SubscriptionId) -> CadResult<()> {
        self.core.borrow_mut().unsubscribe(id)
    }

    /// Current watermark for host snapshots/resync.
    pub fn watermark(&self) -> EventWatermark {
        self.core.borrow().watermark()
    }

    pub fn is_exhausted(&self) -> bool {
        self.core.borrow().exhausted
    }

    /// Sequence slots currently reserved (startup reserve plus live permits).
    pub fn outstanding_reserved(&self) -> u64 {
        self.core.borrow().reserved
    }

    /// Next sequence number the ring would assign.
    pub fn next_sequence(&self) -> u64 {
        self.core.borrow().next_sequence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> EventEnvelope {
        EventEnvelope {
            sequence: 0,
            request_id: Some(RequestId(1)),
            origin: RequestOrigin::Ui,
            document: DocumentId(0),
            generation: 0,
            revision: Revision(0),
            transaction: None,
            event: EngineEvent::ToolChanged {
                generation: 0,
                kind: ToolKind::Idle,
            },
        }
    }

    fn publish_one(ring: &EventRing) -> u64 {
        let mut permit = ring.reserve_trace(1).unwrap();
        permit.publish(envelope()).unwrap()
    }

    #[test]
    fn events_are_shared_by_arc_across_subscribers() {
        let ring = EventRing::new();
        let a = ring.subscribe().unwrap();
        let b = ring.subscribe().unwrap();
        assert_eq!(publish_one(&ring), 0);

        let EventPoll::Events(first) = ring.poll_events(a, MAX_POLL).unwrap() else {
            panic!("expected events");
        };
        let EventPoll::Events(second) = ring.poll_events(b, MAX_POLL).unwrap() else {
            panic!("expected events");
        };
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert!(Arc::ptr_eq(&first[0], &second[0]), "records are shared");
    }

    #[test]
    fn ring_lag_advances_cursor_to_oldest_then_delivers_real_events() {
        let ring = EventRing::with_capacity(2);
        let sub = ring.subscribe().unwrap();
        publish_one(&ring);
        publish_one(&ring);
        publish_one(&ring); // overwrites sequence 0

        let lag = ring.poll_events(sub, MAX_POLL).unwrap();
        assert!(matches!(
            lag,
            EventPoll::Lagged {
                missed: 1,
                next_sequence: 1
            }
        ));
        // The cursor was advanced to the oldest retained sequence.
        let EventPoll::Events(events) = ring.poll_events(sub, MAX_POLL).unwrap() else {
            panic!("expected events after lag");
        };
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 1);
        assert_eq!(events[1].sequence, 2);
    }

    #[test]
    fn watermark_reports_oldest_tail_and_capacity() {
        let ring = EventRing::with_capacity(3);
        let empty = ring.watermark();
        assert_eq!(empty.oldest_sequence, None);
        assert_eq!(empty.next_sequence, 0);
        assert_eq!(empty.records, 0);
        assert_eq!(empty.capacity, 3, "capacity is not the default zero");
        assert!(!empty.exhausted);
        publish_one(&ring);
        publish_one(&ring);
        let mark = ring.watermark();
        assert_eq!(mark.oldest_sequence, Some(0));
        assert_eq!(mark.next_sequence, 2);
        assert_eq!(mark.records, 2);
        assert_eq!(mark.capacity, 3);
        assert!(!mark.exhausted);
    }

    #[test]
    fn subscribe_caps_at_thirty_two_and_unsubscribe_frees_a_slot() {
        let ring = EventRing::new();
        let mut ids = Vec::new();
        for _ in 0..MAX_SUBSCRIBERS {
            ids.push(ring.subscribe().unwrap());
        }
        assert!(
            ring.subscribe().is_err(),
            "the 33rd subscription is refused"
        );

        ring.unsubscribe(ids[0]).unwrap();
        let retry = ring.subscribe().unwrap();
        assert!(
            !ids.contains(&retry),
            "ids are never reused after a slot is freed"
        );
        assert!(ring.subscribe().is_err(), "still capped at 32");
    }

    #[test]
    fn unknown_subscription_ids_are_explicit_errors() {
        let ring = EventRing::new();
        let missing = SubscriptionId(99);
        assert!(ring.poll_events(missing, MAX_POLL).is_err());
        assert!(ring.unsubscribe(missing).is_err());
        // A well-formed id that was unsubscribed is also unknown.
        let id = ring.subscribe().unwrap();
        ring.unsubscribe(id).unwrap();
        assert!(ring.poll_events(id, MAX_POLL).is_err());
        assert!(ring.unsubscribe(id).is_err());
    }

    #[test]
    fn poll_respects_limit_and_capacity_overwrites_oldest() {
        let ring = EventRing::with_capacity(4);
        let sub = ring.subscribe().unwrap();
        for _ in 0..6 {
            publish_one(&ring);
        }
        assert_eq!(
            ring.watermark().records,
            4,
            "capacity bounds retained records"
        );
        // The subscriber was at tail 0; the oldest retained is sequence 2, so the
        // first poll reports an explicit lag and advances the cursor.
        let lag = ring.poll_events(sub, 2).unwrap();
        assert!(matches!(
            lag,
            EventPoll::Lagged {
                missed: 2,
                next_sequence: 2
            }
        ));
        // The next poll delivers real retained events, limited to two at a time.
        let EventPoll::Events(first) = ring.poll_events(sub, 2).unwrap() else {
            panic!("expected events after lag");
        };
        assert_eq!(first.len(), 2, "poll honours the limit");
        assert_eq!(first[0].sequence, 2, "oldest retained is delivered first");
        assert_eq!(first[1].sequence, 3);
        let EventPoll::Events(second) = ring.poll_events(sub, 2).unwrap() else {
            panic!("expected the remaining events");
        };
        assert_eq!(second.len(), 2);
        assert_eq!(second[0].sequence, 4);
        assert_eq!(second[1].sequence, 5);
    }

    #[test]
    fn sequence_exhaustion_is_permanent_and_poll_reports_it_at_the_tail() {
        let ring = EventRing::new();
        {
            let mut core = ring.core.borrow_mut();
            core.next_sequence = u64::MAX;
            core.reserved = 0;
        }
        // Subscribing now places the cursor exactly at the tail.
        let sub = ring.subscribe().unwrap();
        assert!(ring.reserve_trace(1).is_err(), "no headroom left");
        assert!(ring.is_exhausted());
        assert!(ring.reserve_trace(1).is_err(), "exhaustion is permanent");
        // Caught up to the tail with no live permit: explicit Exhausted error.
        assert!(matches!(
            ring.poll_events(sub, MAX_POLL),
            Err(CadError::InvalidInput(_))
        ));
        // An unknown subscription is still an explicit unknown-id error.
        assert!(matches!(
            ring.poll_events(SubscriptionId(12345), MAX_POLL),
            Err(CadError::ResourceMissing(_))
        ));
    }

    #[test]
    fn exhausted_ring_delivers_committed_events_then_reports_at_the_tail() {
        let ring = EventRing::new();
        {
            let mut core = ring.core.borrow_mut();
            core.next_sequence = u64::MAX - 2;
            core.reserved = 0;
        }
        let sub = ring.subscribe().unwrap(); // cursor at the tail (MAX - 2)
        let mut first = ring.reserve_trace(1).unwrap();
        let mut second = ring.reserve_trace(1).unwrap();
        assert!(
            ring.reserve_trace(1).is_err(),
            "no headroom for a new admission"
        );
        assert!(ring.is_exhausted());

        // At the tail with live permits: empty events, never a premature error.
        let poll = ring.poll_events(sub, MAX_POLL).unwrap();
        assert!(matches!(poll, EventPoll::Events(ref events) if events.is_empty()));

        // The already-committed permits still fulfil their promised slots.
        assert_eq!(first.publish(envelope()).unwrap(), u64::MAX - 2);
        assert_eq!(second.publish(envelope()).unwrap(), u64::MAX - 1);
        let EventPoll::Events(events) = ring.poll_events(sub, MAX_POLL).unwrap() else {
            panic!("committed events must be readable");
        };
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, u64::MAX - 2);
        assert_eq!(events[1].sequence, u64::MAX - 1);

        // Drained, no live permits left: now the explicit Exhausted error.
        assert!(matches!(
            ring.poll_events(sub, MAX_POLL),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn exhausted_ring_drains_lag_then_retained_events() {
        let ring = EventRing::with_capacity(4);
        let sub = ring.subscribe().unwrap(); // cursor 0
        {
            let mut core = ring.core.borrow_mut();
            core.next_sequence = u64::MAX - 4;
            core.reserved = 0;
        }
        let mut first = ring.reserve_trace(2).unwrap();
        let mut second = ring.reserve_trace(2).unwrap();
        assert!(ring.reserve_trace(1).is_err());
        assert!(ring.is_exhausted());
        first.publish_batch(vec![envelope(), envelope()]).unwrap();
        second.publish_batch(vec![envelope(), envelope()]).unwrap();

        // Old records were still readable: first an explicit lag to the oldest.
        let lag = ring.poll_events(sub, MAX_POLL).unwrap();
        assert!(matches!(
            lag,
            EventPoll::Lagged { next_sequence, .. } if next_sequence == u64::MAX - 4
        ));
        let EventPoll::Events(events) = ring.poll_events(sub, MAX_POLL).unwrap() else {
            panic!("retained events must be readable");
        };
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].sequence, u64::MAX - 4);
        assert_eq!(events[3].sequence, u64::MAX - 1);

        // Drained at the tail with no live permits: explicit Exhausted error.
        assert!(matches!(
            ring.poll_events(sub, MAX_POLL),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn single_last_finished_event_is_delivered_before_exhaustion() {
        let ring = EventRing::new();
        let mut permit = ring.reserve_trace(1).unwrap();
        {
            let mut core = ring.core.borrow_mut();
            core.next_sequence = u64::MAX - 1;
        }
        let sub = ring.subscribe().unwrap(); // cursor at the tail (MAX - 1)
        assert!(ring.reserve_trace(1).is_err(), "admission closes");
        assert!(ring.is_exhausted());

        // Live permit at the tail: empty events, not a premature error.
        assert!(matches!(
            ring.poll_events(sub, MAX_POLL).unwrap(),
            EventPoll::Events(ref events) if events.is_empty()
        ));

        // The single last finished trace is delivered with its real sequence.
        assert_eq!(permit.publish(envelope()).unwrap(), u64::MAX - 1);
        let EventPoll::Events(events) = ring.poll_events(sub, MAX_POLL).unwrap() else {
            panic!("the last event must be delivered");
        };
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sequence, u64::MAX - 1);

        // Only the next poll reports Exhausted.
        assert!(matches!(
            ring.poll_events(sub, MAX_POLL),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn final_batch_is_delivered_with_real_sequences_before_exhaustion() {
        let ring = EventRing::new();
        let mut first = ring.reserve_trace(2).unwrap();
        let mut second = ring.reserve_trace(2).unwrap();
        {
            let mut core = ring.core.borrow_mut();
            core.next_sequence = u64::MAX - 4;
        }
        let sub = ring.subscribe().unwrap(); // cursor at the tail (MAX - 4)
        assert!(ring.reserve_trace(1).is_err());
        assert!(ring.is_exhausted());

        assert_eq!(
            first.publish_batch(vec![envelope(), envelope()]).unwrap(),
            vec![u64::MAX - 4, u64::MAX - 3]
        );
        let EventPoll::Events(first_events) = ring.poll_events(sub, MAX_POLL).unwrap() else {
            panic!("the first batch must be delivered");
        };
        assert_eq!(first_events.len(), 2);

        // The final batch is delivered with its real sequences, before any error.
        assert_eq!(
            second.publish_batch(vec![envelope(), envelope()]).unwrap(),
            vec![u64::MAX - 2, u64::MAX - 1]
        );
        let EventPoll::Events(last) = ring.poll_events(sub, MAX_POLL).unwrap() else {
            panic!("the final batch must be delivered, not dropped");
        };
        assert_eq!(
            last.iter().map(|event| event.sequence).collect::<Vec<_>>(),
            vec![u64::MAX - 2, u64::MAX - 1]
        );

        // Only the following poll reports Exhausted.
        assert!(matches!(
            ring.poll_events(sub, MAX_POLL),
            Err(CadError::InvalidInput(_))
        ));
    }

    #[test]
    fn limit_zero_does_not_misreport_exhaustion_for_unread_events() {
        let ring = EventRing::new();
        let sub = ring.subscribe().unwrap();
        publish_one(&ring);
        publish_one(&ring);
        {
            let mut core = ring.core.borrow_mut();
            core.next_sequence = u64::MAX;
            core.reserved = 0;
        }
        assert!(ring.reserve_trace(1).is_err());
        assert!(ring.is_exhausted());

        // limit 0 must not advance the cursor, so the unread events keep the ring
        // away from the tail and this is empty events, not an Exhausted error.
        let poll = ring.poll_events(sub, 0).unwrap();
        assert!(matches!(poll, EventPoll::Events(ref events) if events.is_empty()));

        // The unread events remain readable afterwards.
        let EventPoll::Events(events) = ring.poll_events(sub, MAX_POLL).unwrap() else {
            panic!("unread events must remain readable");
        };
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 0);
        assert_eq!(events[1].sequence, 1);
    }

    #[test]
    fn reserved_permits_never_oversell_and_release_unused() {
        let ring = EventRing::new();
        assert_eq!(
            ring.outstanding_reserved(),
            0,
            "the ring holds no startup reservation"
        );
        assert_eq!(ring.next_sequence(), 0);
        let mut permit = ring.reserve_trace(TRACE_ALLOWANCE).unwrap();
        assert_eq!(ring.outstanding_reserved(), TRACE_ALLOWANCE);
        assert_eq!(
            ring.next_sequence(),
            0,
            "reserving does not advance the sequence"
        );
        permit.publish(envelope()).unwrap();
        assert_eq!(ring.next_sequence(), 1, "only publishing advances it");
        drop(permit);
        assert_eq!(
            ring.outstanding_reserved(),
            0,
            "drop returns the whole unused reservation"
        );
        assert!(
            ring.reserve_trace(0).is_ok(),
            "a zero reservation is a no-op"
        );
    }

    #[test]
    fn new_admission_refused_at_capacity_but_existing_permits_still_publish() {
        let ring = EventRing::new();
        {
            let mut core = ring.core.borrow_mut();
            core.next_sequence = u64::MAX - 4;
            core.reserved = 0;
        }
        // Two permits reserve the entire remaining headroom (4 slots).
        let mut first = ring.reserve_trace(2).unwrap();
        let mut second = ring.reserve_trace(2).unwrap();
        assert_eq!(ring.outstanding_reserved(), 4);

        // A new admission is permanently refused and closes admissions.
        assert!(ring.reserve_trace(1).is_err());
        assert!(ring.is_exhausted());

        // The already-committed permits must still be fulfilled without wrapping.
        let first_seqs = first.publish_batch(vec![envelope(), envelope()]).unwrap();
        let second_seqs = second.publish_batch(vec![envelope(), envelope()]).unwrap();
        assert_eq!(first_seqs, vec![u64::MAX - 4, u64::MAX - 3]);
        assert_eq!(second_seqs, vec![u64::MAX - 2, u64::MAX - 1]);
        assert_eq!(ring.next_sequence(), u64::MAX);

        // No new request may start.
        assert!(ring.reserve_trace(1).is_err());
    }

    #[test]
    fn publish_batch_rejects_atomically_without_partial_commit() {
        let ring = EventRing::new();
        let mut permit = ring.reserve_trace(TRACE_ALLOWANCE).unwrap();

        // 17 > the 16-event allowance: rejected with zero state change.
        let too_many: Vec<EventEnvelope> = (0..17).map(|_| envelope()).collect();
        assert!(matches!(
            permit.publish_batch(too_many),
            Err(RequestError::Execution(CadError::Invariant(_)))
        ));
        assert_eq!(permit.remaining(), TRACE_ALLOWANCE);
        assert_eq!(ring.next_sequence(), 0);
        assert_eq!(ring.outstanding_reserved(), TRACE_ALLOWANCE);

        // A batch that fits publishes whole and preserves Arc-shared payloads.
        let batch: Vec<EventEnvelope> = (0..2).map(|_| envelope()).collect();
        assert_eq!(permit.publish_batch(batch).unwrap(), vec![0, 1]);
        assert_eq!(ring.next_sequence(), 2);
        assert_eq!(permit.remaining(), TRACE_ALLOWANCE - 2);

        // Insufficient sequence headroom is refused atomically too.
        {
            let mut core = ring.core.borrow_mut();
            core.next_sequence = u64::MAX - 1;
        }
        let overflow: Vec<EventEnvelope> = (0..4).map(|_| envelope()).collect();
        assert!(matches!(
            permit.publish_batch(overflow),
            Err(RequestError::Execution(CadError::Invariant(_)))
        ));
        assert_eq!(ring.next_sequence(), u64::MAX - 1, "no partial commit");
        assert_eq!(permit.remaining(), TRACE_ALLOWANCE - 2, "permit untouched");
    }

    #[test]
    fn reservation_cannot_exceed_sequence_headroom() {
        let ring = EventRing::new();
        {
            let mut core = ring.core.borrow_mut();
            core.next_sequence = u64::MAX - 2;
            core.reserved = 0;
        }
        assert!(ring.reserve_trace(3).is_err(), "3 > 2 headroom is refused");
        assert!(ring.is_exhausted());
    }

    #[test]
    fn trace_permit_cannot_publish_beyond_its_reservation() {
        let ring = EventRing::new();
        let mut permit = ring.reserve_trace(1).unwrap();
        permit.publish(envelope()).unwrap();
        assert!(
            matches!(
                permit.publish(envelope()),
                Err(RequestError::Execution(CadError::Invariant(_)))
            ),
            "a spent permit refuses rather than faking success"
        );
    }
}

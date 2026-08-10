//! Synchronous, bounded, platform-neutral observe-only pipeline.
//!
//! This crate has no event source implementation. It accepts mock sources and
//! typed values from `zonkey-types`; it never observes hardware or edits text.

use std::collections::VecDeque;

use zonkey_types::{ObservedInputEvent, ObserverError, ObserverStatus};

/// The default maximum number of events retained by an observe queue.
pub const DEFAULT_OBSERVE_QUEUE_CAPACITY: usize = 256;

/// Error returned when an observe queue capacity is zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueCapacityError;

/// Result of attempting to enqueue one event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnqueueOutcome {
    /// The event was accepted at the FIFO tail.
    Enqueued,
    /// The queue was full and the newest event was dropped.
    DroppedFull,
}

/// An event removed from a queue, including a one-time continuity marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DequeuedEvent {
    /// The accepted event.
    pub event: ObservedInputEvent,
    /// True only for the first event dequeued after one or more drops.
    pub discontinuity_before_event: bool,
}

/// Result of one non-blocking dequeue attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DequeueOutcome {
    /// An event was available.
    Event(DequeuedEvent),
    /// The queue contained no event.
    Empty,
}

/// Aggregate queue state; no event history or text is retained here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueStats {
    /// Configured maximum number of queued events.
    pub capacity: usize,
    /// Current number of queued events.
    pub queued: usize,
    /// Monotonic number of incoming events dropped while full.
    pub dropped_events: u64,
    /// Whether the next successful dequeue will report a discontinuity.
    pub discontinuity_pending: bool,
}

/// A bounded FIFO queue for typed observed events.
#[derive(Debug)]
pub struct ObserveQueue {
    capacity: usize,
    events: VecDeque<ObservedInputEvent>,
    dropped_events: u64,
    discontinuity_pending: bool,
}

impl ObserveQueue {
    /// Constructs a queue with a validated non-zero capacity.
    ///
    /// # Errors
    ///
    /// Returns [`QueueCapacityError`] when `capacity` is zero.
    pub fn new(capacity: usize) -> Result<Self, QueueCapacityError> {
        if capacity == 0 {
            return Err(QueueCapacityError);
        }
        Ok(Self {
            capacity,
            events: VecDeque::with_capacity(capacity),
            dropped_events: 0,
            discontinuity_pending: false,
        })
    }

    /// Attempts to enqueue without waiting or evicting an older event.
    pub fn try_enqueue(&mut self, event: ObservedInputEvent) -> EnqueueOutcome {
        if self.events.len() == self.capacity {
            self.dropped_events = self.dropped_events.saturating_add(1);
            self.discontinuity_pending = true;
            EnqueueOutcome::DroppedFull
        } else {
            self.events.push_back(event);
            EnqueueOutcome::Enqueued
        }
    }

    /// Removes the oldest queued event without waiting.
    pub fn try_dequeue(&mut self) -> DequeueOutcome {
        let Some(event) = self.events.pop_front() else {
            return DequeueOutcome::Empty;
        };
        let discontinuity_before_event = self.discontinuity_pending;
        self.discontinuity_pending = false;
        DequeueOutcome::Event(DequeuedEvent {
            event,
            discontinuity_before_event,
        })
    }

    /// Returns aggregate queue state without exposing mutable internals.
    #[must_use]
    pub fn stats(&self) -> QueueStats {
        QueueStats {
            capacity: self.capacity,
            queued: self.events.len(),
            dropped_events: self.dropped_events,
            discontinuity_pending: self.discontinuity_pending,
        }
    }
}

impl Default for ObserveQueue {
    fn default() -> Self {
        Self::new(DEFAULT_OBSERVE_QUEUE_CAPACITY).expect("default capacity is non-zero")
    }
}

/// A mock-only source of already validated observed events.
pub trait EventSource {
    /// Returns the next event, normal exhaustion, or a redacted typed failure.
    ///
    /// # Errors
    ///
    /// Returns a typed [`ObserverError`] when the mock source fails.
    fn next_event(&mut self) -> Result<Option<ObservedInputEvent>, ObserverError>;
}

/// Aggregate classification returned by a platform-neutral processor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessorClassification {
    /// Event was intentionally ignored.
    Ignored,
    /// Event was observed and accepted for aggregate processing.
    Observed,
    /// Event represented a neutral boundary.
    BoundaryObserved,
    /// Event could not be classified by the processor.
    Unsupported,
    /// Event failed processor-side neutral validation.
    Invalid,
}

/// A processor boundary that cannot return text or execute edits.
pub trait EventProcessor {
    /// Resets token-related state after queue continuity loss.
    fn reset_after_discontinuity(&mut self);

    /// Processes one validated event and returns aggregate classification only.
    fn process(&mut self, event: &ObservedInputEvent) -> ProcessorClassification;
}

/// Aggregate service counters with no raw event or token payload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AggregateReport {
    /// Number of events returned by the source.
    pub received: u64,
    /// Number accepted by the bounded queue.
    pub accepted: u64,
    /// Number rejected because the queue was full.
    pub dropped: u64,
    /// Number processed by the processor.
    pub processed: u64,
    /// Number of discontinuity resets delivered to the processor.
    pub discontinuities: u64,
    /// Number of source failures.
    pub source_failures: u64,
    /// Number classified as unsupported.
    pub unsupported_events: u64,
    /// Number classified as invalid.
    pub invalid_events: u64,
}

/// A deterministic synchronous service around a bounded observe queue.
pub struct ObserveService {
    queue: ObserveQueue,
    status: ObserverStatus,
    stop_requested: bool,
    report: AggregateReport,
}

impl ObserveService {
    /// Creates a service in the `Starting` state.
    #[must_use]
    pub fn new(queue: ObserveQueue) -> Self {
        Self {
            queue,
            status: ObserverStatus::Starting,
            stop_requested: false,
            report: AggregateReport::default(),
        }
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub const fn status(&self) -> ObserverStatus {
        self.status
    }

    /// Requests a graceful stop; repeated requests are harmless.
    pub fn request_stop(&mut self) {
        if !matches!(
            self.status,
            ObserverStatus::Stopped | ObserverStatus::Failed
        ) {
            self.stop_requested = true;
        }
    }

    /// Runs a finite mock source synchronously and drains accepted events.
    ///
    /// Source exhaustion and an explicit stop both drain accepted events and
    /// end in `Stopped`. A source error ends in `Failed` without fabricating
    /// or processing missing events.
    pub fn run<S: EventSource, P: EventProcessor>(
        &mut self,
        source: &mut S,
        processor: &mut P,
    ) -> AggregateReport {
        if matches!(
            self.status,
            ObserverStatus::Stopped | ObserverStatus::Failed
        ) {
            return self.report;
        }
        self.status = ObserverStatus::Running;
        while !self.stop_requested {
            match source.next_event() {
                Ok(Some(event)) => {
                    self.report.received += 1;
                    match self.queue.try_enqueue(event) {
                        EnqueueOutcome::Enqueued => self.report.accepted += 1,
                        EnqueueOutcome::DroppedFull => self.report.dropped += 1,
                    }
                }
                Ok(None) => {
                    self.finish_gracefully(processor);
                    return self.report;
                }
                Err(_) => {
                    self.report.source_failures += 1;
                    self.status = ObserverStatus::Failed;
                    return self.report;
                }
            }
        }
        self.finish_gracefully(processor);
        self.report
    }

    /// Returns the queue's aggregate state after a run or between runs.
    #[must_use]
    pub fn queue_stats(&self) -> QueueStats {
        self.queue.stats()
    }

    /// Returns the aggregate report accumulated by this service.
    #[must_use]
    pub const fn report(&self) -> AggregateReport {
        self.report
    }

    fn finish_gracefully<P: EventProcessor>(&mut self, processor: &mut P) {
        self.status = ObserverStatus::Stopping;
        while let DequeueOutcome::Event(dequeued) = self.queue.try_dequeue() {
            if dequeued.discontinuity_before_event {
                processor.reset_after_discontinuity();
                self.report.discontinuities += 1;
            }
            self.report.processed += 1;
            match processor.process(&dequeued.event) {
                ProcessorClassification::Unsupported => self.report.unsupported_events += 1,
                ProcessorClassification::Invalid => self.report.invalid_events += 1,
                ProcessorClassification::Ignored
                | ProcessorClassification::Observed
                | ProcessorClassification::BoundaryObserved => {}
            }
        }
        self.status = ObserverStatus::Stopped;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zonkey_types::{EventSequence, KeyEventKind, ModifierState, ObservedKey};

    fn event(sequence: u64) -> ObservedInputEvent {
        ObservedInputEvent {
            key: ObservedKey::letter('a').expect("test key is valid"),
            kind: KeyEventKind::KeyDown,
            modifiers: ModifierState::new(),
            injection_origin: zonkey_types::InjectionOrigin::PhysicalOrUnmarked,
            sequence: EventSequence::new(sequence).expect("test sequence is non-zero"),
        }
    }

    #[test]
    fn queue_has_default_capacity_and_rejects_zero() {
        assert_eq!(DEFAULT_OBSERVE_QUEUE_CAPACITY, 256);
        assert!(matches!(ObserveQueue::new(0), Err(QueueCapacityError)));
        assert_eq!(ObserveQueue::default().stats().capacity, 256);
    }

    #[test]
    fn queue_accepts_fifo_and_empty_is_explicit() {
        let mut queue = ObserveQueue::new(2).unwrap();
        assert_eq!(queue.try_enqueue(event(1)), EnqueueOutcome::Enqueued);
        assert_eq!(queue.try_enqueue(event(2)), EnqueueOutcome::Enqueued);
        assert!(
            matches!(queue.try_dequeue(), DequeueOutcome::Event(value) if value.event.sequence.get() == 1)
        );
        assert!(
            matches!(queue.try_dequeue(), DequeueOutcome::Event(value) if value.event.sequence.get() == 2)
        );
        assert_eq!(queue.try_dequeue(), DequeueOutcome::Empty);
    }

    #[test]
    fn full_queue_drops_newest_and_preserves_older_events() {
        let mut queue = ObserveQueue::new(2).unwrap();
        queue.try_enqueue(event(1));
        queue.try_enqueue(event(2));
        assert_eq!(queue.try_enqueue(event(3)), EnqueueOutcome::DroppedFull);
        assert_eq!(queue.try_enqueue(event(4)), EnqueueOutcome::DroppedFull);
        assert_eq!(queue.stats().dropped_events, 2);
        assert_eq!(queue.stats().queued, 2);
        assert!(
            matches!(queue.try_dequeue(), DequeueOutcome::Event(value) if value.event.sequence.get() == 1 && value.discontinuity_before_event)
        );
        assert!(
            matches!(queue.try_dequeue(), DequeueOutcome::Event(value) if value.event.sequence.get() == 2 && !value.discontinuity_before_event)
        );
        assert!(!queue.stats().discontinuity_pending);
    }

    #[test]
    fn second_overflow_is_a_new_discontinuity_episode() {
        let mut queue = ObserveQueue::new(1).unwrap();
        queue.try_enqueue(event(1));
        assert_eq!(queue.try_enqueue(event(2)), EnqueueOutcome::DroppedFull);
        let first = queue.try_dequeue();
        assert!(matches!(first, DequeueOutcome::Event(value) if value.discontinuity_before_event));
        queue.try_enqueue(event(3));
        assert_eq!(queue.try_enqueue(event(4)), EnqueueOutcome::DroppedFull);
        let second = queue.try_dequeue();
        assert!(matches!(second, DequeueOutcome::Event(value) if value.discontinuity_before_event));
        assert_eq!(queue.stats().dropped_events, 2);
    }

    #[derive(Default)]
    struct MockSource {
        events: VecDeque<Result<Option<ObservedInputEvent>, ObserverError>>,
    }

    impl EventSource for MockSource {
        fn next_event(&mut self) -> Result<Option<ObservedInputEvent>, ObserverError> {
            self.events.pop_front().unwrap_or(Ok(None))
        }
    }

    struct MockPlatformAdapter {
        events: VecDeque<ObservedInputEvent>,
    }

    impl MockPlatformAdapter {
        fn new(events: Vec<ObservedInputEvent>) -> Self {
            Self {
                events: events.into(),
            }
        }
    }

    impl EventSource for MockPlatformAdapter {
        fn next_event(&mut self) -> Result<Option<ObservedInputEvent>, ObserverError> {
            Ok(self.events.pop_front())
        }
    }

    struct CountingFailingAdapter {
        responses: VecDeque<Result<Option<ObservedInputEvent>, ObserverError>>,
        polls: usize,
    }

    impl EventSource for CountingFailingAdapter {
        fn next_event(&mut self) -> Result<Option<ObservedInputEvent>, ObserverError> {
            self.polls += 1;
            self.responses.pop_front().unwrap_or(Ok(None))
        }
    }

    #[derive(Default)]
    struct MockProcessor {
        processed: Vec<u64>,
        resets: usize,
    }

    impl EventProcessor for MockProcessor {
        fn reset_after_discontinuity(&mut self) {
            self.resets += 1;
        }

        fn process(&mut self, event: &ObservedInputEvent) -> ProcessorClassification {
            self.processed.push(event.sequence.get());
            ProcessorClassification::Observed
        }
    }

    struct BoundaryProcessor;

    impl EventProcessor for BoundaryProcessor {
        fn reset_after_discontinuity(&mut self) {}

        fn process(&mut self, _: &ObservedInputEvent) -> ProcessorClassification {
            ProcessorClassification::BoundaryObserved
        }
    }

    #[test]
    fn service_processes_finite_source_in_order_and_exhausts_cleanly() {
        let mut source = MockSource {
            events: VecDeque::from([Ok(Some(event(1))), Ok(Some(event(2))), Ok(None)]),
        };
        let mut processor = MockProcessor::default();
        let mut service = ObserveService::new(ObserveQueue::new(4).unwrap());
        let report = service.run(&mut source, &mut processor);
        assert_eq!(service.status(), ObserverStatus::Stopped);
        assert_eq!(processor.processed, vec![1, 2]);
        assert_eq!(report.received, 2);
        assert_eq!(report.accepted, 2);
        assert_eq!(report.processed, 2);
    }

    #[test]
    fn mock_platform_adapter_feeds_validated_events_to_service_fifo() {
        let mut adapter = MockPlatformAdapter::new(vec![event(7), event(8), event(9)]);
        let mut processor = MockProcessor::default();
        let mut service = ObserveService::new(ObserveQueue::new(3).unwrap());

        let report = service.run(&mut adapter, &mut processor);

        assert_eq!(report.received, 3);
        assert_eq!(report.accepted, 3);
        assert_eq!(report.dropped, 0);
        assert_eq!(report.processed, 3);
        assert_eq!(processor.processed, vec![7, 8, 9]);
    }

    #[test]
    fn invalid_event_inputs_are_rejected_before_adapter_boundary() {
        assert!(zonkey_types::ObservedKey::letter('é').is_err());
        assert!(zonkey_types::EventSequence::new(0).is_err());
    }

    #[test]
    fn source_failure_is_terminal_without_repoll_or_queue_drain() {
        let mut adapter = CountingFailingAdapter {
            responses: VecDeque::from([
                Ok(Some(event(10))),
                Ok(Some(event(11))),
                Err(ObserverError::EventSourceClosed),
                Ok(Some(event(12))),
            ]),
            polls: 0,
        };
        let mut processor = MockProcessor::default();
        let mut service = ObserveService::new(ObserveQueue::new(4).unwrap());

        let first_report = service.run(&mut adapter, &mut processor);
        assert_eq!(adapter.polls, 3);
        assert_eq!(service.status(), ObserverStatus::Failed);
        assert_eq!(first_report.received, 2);
        assert_eq!(first_report.accepted, 2);
        assert_eq!(first_report.source_failures, 1);
        assert_eq!(first_report.processed, 0);
        assert_eq!(service.queue_stats().queued, 2);
        assert!(processor.processed.is_empty());

        let second_report = service.run(&mut adapter, &mut processor);
        assert_eq!(adapter.polls, 3);
        assert_eq!(service.status(), ObserverStatus::Failed);
        assert_eq!(second_report, first_report);
        assert_eq!(service.queue_stats().queued, 2);
        assert!(processor.processed.is_empty());
    }

    #[test]
    fn source_exhaustion_stops_after_fifo_drain_without_repoll() {
        let mut adapter = CountingFailingAdapter {
            responses: VecDeque::from([
                Ok(Some(event(20))),
                Ok(Some(event(21))),
                Ok(None),
                Ok(Some(event(22))),
            ]),
            polls: 0,
        };
        let mut processor = MockProcessor::default();
        let mut service = ObserveService::new(ObserveQueue::new(4).unwrap());

        let first_report = service.run(&mut adapter, &mut processor);
        assert_eq!(adapter.polls, 3);
        assert_eq!(service.status(), ObserverStatus::Stopped);
        assert_eq!(first_report.received, 2);
        assert_eq!(first_report.accepted, 2);
        assert_eq!(first_report.processed, 2);
        assert_eq!(first_report.dropped, 0);
        assert_eq!(first_report.discontinuities, 0);
        assert_eq!(processor.processed, vec![20, 21]);
        assert_eq!(service.queue_stats().queued, 0);

        let second_report = service.run(&mut adapter, &mut processor);
        assert_eq!(adapter.polls, 3);
        assert_eq!(service.status(), ObserverStatus::Stopped);
        assert_eq!(second_report, first_report);
        assert_eq!(processor.processed, vec![20, 21]);
        assert_eq!(service.queue_stats().queued, 0);
    }

    #[test]
    fn service_source_error_fails_and_retains_counters() {
        let mut source = MockSource {
            events: VecDeque::from([Ok(Some(event(1))), Err(ObserverError::EventSourceClosed)]),
        };
        let mut processor = MockProcessor::default();
        let mut service = ObserveService::new(ObserveQueue::new(4).unwrap());
        let report = service.run(&mut source, &mut processor);
        assert_eq!(service.status(), ObserverStatus::Failed);
        assert_eq!(report.received, 1);
        assert_eq!(report.accepted, 1);
        assert_eq!(report.source_failures, 1);
        assert!(processor.processed.is_empty());
    }

    #[test]
    fn service_reports_overflow_and_resets_before_affected_event() {
        let mut source = MockSource {
            events: VecDeque::from([
                Ok(Some(event(1))),
                Ok(Some(event(2))),
                Ok(Some(event(3))),
                Ok(None),
            ]),
        };
        let mut processor = MockProcessor::default();
        let mut service = ObserveService::new(ObserveQueue::new(2).unwrap());
        let report = service.run(&mut source, &mut processor);
        assert_eq!(report.dropped, 1);
        assert_eq!(report.discontinuities, 1);
        assert_eq!(processor.resets, 1);
        assert_eq!(processor.processed, vec![1, 2]);
    }

    #[test]
    fn stop_is_idempotent_and_graceful_drain_is_exact() {
        let mut source = MockSource {
            events: VecDeque::from([Ok(Some(event(1))), Ok(Some(event(2))), Ok(None)]),
        };
        let mut processor = MockProcessor::default();
        let mut service = ObserveService::new(ObserveQueue::new(4).unwrap());
        service.request_stop();
        service.request_stop();
        let report = service.run(&mut source, &mut processor);
        service.request_stop();
        assert_eq!(service.status(), ObserverStatus::Stopped);
        assert_eq!(processor.processed, Vec::<u64>::new());
        assert_eq!(report.received, 0);
        assert_eq!(service.run(&mut source, &mut processor), report);
    }

    #[test]
    fn explicit_stop_does_not_poll_source_and_drains_existing_queue_once() {
        let mut queue = ObserveQueue::new(2).unwrap();
        queue.try_enqueue(event(1));
        queue.try_enqueue(event(2));
        let mut source = MockSource {
            events: VecDeque::from([Ok(Some(event(3))), Ok(None)]),
        };
        let mut processor = MockProcessor::default();
        let mut service = ObserveService::new(queue);
        service.request_stop();
        service.request_stop();
        let report = service.run(&mut source, &mut processor);
        assert_eq!(service.status(), ObserverStatus::Stopped);
        assert_eq!(report.received, 0);
        assert_eq!(report.processed, 2);
        assert_eq!(processor.processed, vec![1, 2]);
        assert_eq!(service.run(&mut source, &mut processor), report);
    }

    #[test]
    fn processor_classifications_are_aggregate_only() {
        let mut source = MockSource {
            events: VecDeque::from([Ok(Some(event(1))), Ok(None)]),
        };
        let mut processor = BoundaryProcessor;
        let mut service = ObserveService::new(ObserveQueue::default());
        let report = service.run(&mut source, &mut processor);
        assert_eq!(report.processed, 1);
        assert_eq!(report.unsupported_events, 0);
        assert_eq!(report.invalid_events, 0);
    }
}

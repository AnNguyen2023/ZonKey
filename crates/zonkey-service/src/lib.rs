//! Synchronous, bounded, platform-neutral observe-only pipeline.
//!
//! This crate has no event source implementation. It accepts mock sources and
//! typed values from `zonkey-types`; it never observes hardware or edits text.

use std::collections::VecDeque;

use zonkey_detect::{BuiltInDictionaries, Classifier, DetectionRequest, LexicalEvidence};
use zonkey_policy::SafePolicy;
use zonkey_telex::TelexEngine;
use zonkey_types::{
    EngineEvent, InjectionOrigin, InputContext, KeyEventKind, ObservedInputEvent, ObserverError,
    ObserverStatus, TokenBoundary,
};

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

    /// Runs a source synchronously, processing one queued event after each
    /// source event and draining accepted events at terminal shutdown.
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
                    self.process_one(processor);
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
        while self.process_one(processor) {}
        self.status = ObserverStatus::Stopped;
    }

    fn process_one<P: EventProcessor>(&mut self, processor: &mut P) -> bool {
        let DequeueOutcome::Event(dequeued) = self.queue.try_dequeue() else {
            return false;
        };
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
        true
    }
}

/// Sanitized decision category emitted by the diagnostic processor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticDecision {
    /// No recovery decision was warranted.
    Keep,
    /// The policy found a candidate, but it remains diagnostic-only.
    RestoreCandidate,
    /// The evidence was insufficient for recovery.
    Ambiguous,
    /// The token was outside the supported diagnostic scope.
    Unsupported,
}

/// Stateful, observe-only bridge from validated events to M1/M2 decisions.
///
/// This processor reports only sanitized decision categories. It never
/// executes an [`EditPlan`](zonkey_types::EditPlan), edits user text, or
/// returns a command to a platform adapter.
#[allow(clippy::struct_excessive_bools)]
pub struct DiagnosticDecisionProcessor {
    telex: TelexEngine,
    classifier: Classifier<BuiltInDictionaries>,
    policy: SafePolicy,
    context: InputContext,
    show_token: bool,
    completed_tokens: u64,
    resets: u64,
    injected_events: u64,
    control_down: bool,
    alt_down: bool,
    meta_down: bool,
    last_token_lengths: Option<(usize, usize)>,
    last_decision: Option<DiagnosticDecision>,
}

impl DiagnosticDecisionProcessor {
    /// Creates a writing-context processor. `show_token` is a temporary,
    /// foreground-only development diagnostic and is disabled by default.
    #[must_use]
    pub fn new(show_token: bool) -> Self {
        Self {
            telex: TelexEngine::new(),
            classifier: Classifier::new(BuiltInDictionaries),
            policy: SafePolicy,
            context: InputContext::Writing,
            show_token,
            completed_tokens: 0,
            resets: 0,
            injected_events: 0,
            control_down: false,
            alt_down: false,
            meta_down: false,
            last_token_lengths: None,
            last_decision: None,
        }
    }

    /// Number of token boundaries evaluated so far.
    #[must_use]
    pub const fn completed_tokens(&self) -> u64 {
        self.completed_tokens
    }

    /// Number of continuity resets delivered by the service.
    #[must_use]
    pub const fn reset_count(&self) -> u64 {
        self.resets
    }

    /// Number of injected-origin events excluded from token mutation.
    #[must_use]
    pub const fn injected_events(&self) -> u64 {
        self.injected_events
    }

    /// Last sanitized policy category, if a boundary has been evaluated.
    #[must_use]
    pub const fn last_decision(&self) -> Option<DiagnosticDecision> {
        self.last_decision
    }

    /// Returns the lengths of the most recently evaluated non-empty token.
    #[must_use]
    pub const fn last_token_lengths(&self) -> Option<(usize, usize)> {
        self.last_token_lengths
    }

    fn reset_token(&mut self) {
        self.telex = TelexEngine::new();
    }

    fn update_shortcut_state(&mut self, event: &ObservedInputEvent) -> bool {
        let Some(modifier) = event.key.modifier_value() else {
            return false;
        };
        let active = !matches!(event.kind, KeyEventKind::KeyUp | KeyEventKind::SystemKeyUp);
        match modifier {
            zonkey_types::ModifierKey::Control => self.control_down = active,
            zonkey_types::ModifierKey::Alt => self.alt_down = active,
            zonkey_types::ModifierKey::Meta => self.meta_down = active,
            zonkey_types::ModifierKey::Shift => {}
        }
        true
    }

    fn boundary_for(event: &ObservedInputEvent) -> Option<TokenBoundary> {
        if event.key.is_space() {
            Some(TokenBoundary::Space)
        } else if event.key.is_enter() {
            Some(TokenBoundary::Enter)
        } else if event.key.is_tab() {
            Some(TokenBoundary::Tab)
        } else {
            event
                .key
                .punctuation_value()
                .map(TokenBoundary::Punctuation)
        }
    }

    fn evaluate_boundary(&mut self, boundary: TokenBoundary) -> ProcessorClassification {
        let raw = self.telex.token().raw_ascii.clone();
        let rendered = self.telex.token().rendered.clone();
        if raw.is_empty() && rendered.is_empty() {
            let _ = self.telex.process(EngineEvent::Boundary(boundary));
            return ProcessorClassification::BoundaryObserved;
        }
        self.last_token_lengths = Some((raw.len(), rendered.len()));
        let evidence = self.classifier.classify(DetectionRequest {
            raw: &raw,
            rendered: &rendered,
            boundary: &boundary,
            context: self.context,
        });
        let outcome = self
            .policy
            .decide(evidence.clone(), self.context, &rendered);
        let decision = match outcome.decision {
            zonkey_types::RecoveryDecision::RestoreEnglish { .. } => {
                DiagnosticDecision::RestoreCandidate
            }
            zonkey_types::RecoveryDecision::Ambiguous { .. } => DiagnosticDecision::Ambiguous,
            zonkey_types::RecoveryDecision::KeepVietnamese { .. } => match evidence {
                LexicalEvidence::Unsupported => DiagnosticDecision::Unsupported,
                _ => DiagnosticDecision::Keep,
            },
        };
        self.completed_tokens = self.completed_tokens.saturating_add(1);
        self.last_decision = Some(decision);
        if self.show_token {
            println!(
                "decision={decision:?} token={raw:?} rendered_len={} boundary={boundary:?}",
                rendered.len()
            );
        } else {
            println!(
                "decision={decision:?} token_len={} rendered_len={} boundary={boundary:?}",
                raw.len(),
                rendered.len()
            );
        }
        let _ = self.telex.process(EngineEvent::Boundary(boundary));
        ProcessorClassification::BoundaryObserved
    }
}

impl Default for DiagnosticDecisionProcessor {
    fn default() -> Self {
        Self::new(false)
    }
}

impl EventProcessor for DiagnosticDecisionProcessor {
    fn reset_after_discontinuity(&mut self) {
        self.reset_token();
        self.control_down = false;
        self.alt_down = false;
        self.meta_down = false;
        self.resets = self.resets.saturating_add(1);
        println!("discontinuity=true decision_state=reset");
    }

    fn process(&mut self, event: &ObservedInputEvent) -> ProcessorClassification {
        if matches!(
            event.injection_origin,
            InjectionOrigin::MarkedInjected | InjectionOrigin::LowerIntegrityInjected
        ) {
            self.injected_events = self.injected_events.saturating_add(1);
            println!("decision=Ignored reason=injected");
            return ProcessorClassification::Ignored;
        }
        if self.update_shortcut_state(event) {
            return ProcessorClassification::Ignored;
        }
        if matches!(event.kind, KeyEventKind::KeyUp | KeyEventKind::SystemKeyUp) {
            return ProcessorClassification::Ignored;
        }
        if event.modifiers.control()
            || event.modifiers.alt()
            || event.modifiers.meta()
            || self.control_down
            || self.alt_down
            || self.meta_down
        {
            return ProcessorClassification::Ignored;
        }
        if event.key.modifier_value().is_some() {
            return ProcessorClassification::Ignored;
        }
        if event.key.is_escape() {
            self.reset_token();
            return ProcessorClassification::Ignored;
        }
        if event.key.is_backspace() {
            let _ = self.telex.process(EngineEvent::Backspace);
            return ProcessorClassification::Observed;
        }
        if let Some(boundary) = Self::boundary_for(event) {
            return self.evaluate_boundary(boundary);
        }
        let character = event
            .key
            .letter_value()
            .map(|value| {
                if event.modifiers.shift() {
                    value
                } else {
                    value.to_ascii_lowercase()
                }
            })
            .or_else(|| {
                event
                    .key
                    .digit_value()
                    .map(|value| char::from(b'0' + value))
            })
            .ok_or(());
        let Ok(character) = character else {
            self.reset_token();
            return ProcessorClassification::Unsupported;
        };
        if self
            .telex
            .process(EngineEvent::Character(character))
            .is_ok()
        {
            ProcessorClassification::Observed
        } else {
            self.reset_token();
            ProcessorClassification::Unsupported
        }
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

    fn key_event(
        sequence: u64,
        key: zonkey_types::ObservedKey,
        kind: KeyEventKind,
        modifiers: ModifierState,
        injection_origin: zonkey_types::InjectionOrigin,
    ) -> ObservedInputEvent {
        ObservedInputEvent {
            key,
            kind,
            modifiers,
            injection_origin,
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
    fn live_source_is_processed_continuously_without_capacity_overflow() {
        let events = (1..=600).map(|sequence| Ok(Some(event(sequence))));
        let mut source = MockSource {
            events: events.chain([Ok(None)]).collect(),
        };
        let mut processor = MockProcessor::default();
        let mut service = ObserveService::new(ObserveQueue::default());

        let report = service.run(&mut source, &mut processor);

        assert_eq!(report.received, 600);
        assert_eq!(report.accepted, 600);
        assert_eq!(report.dropped, 0);
        assert_eq!(report.processed, 600);
        assert_eq!(processor.processed, (1..=600).collect::<Vec<_>>());
        assert_eq!(service.queue_stats().queued, 0);
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
        let mut queue = ObserveQueue::new(4).unwrap();
        queue.try_enqueue(event(1));
        queue.try_enqueue(event(2));
        let mut adapter = CountingFailingAdapter {
            responses: VecDeque::from([Ok(Some(event(10))), Err(ObserverError::EventSourceClosed)]),
            polls: 0,
        };
        let mut processor = MockProcessor::default();
        let mut service = ObserveService::new(queue);

        let first_report = service.run(&mut adapter, &mut processor);
        assert_eq!(adapter.polls, 2);
        assert_eq!(service.status(), ObserverStatus::Failed);
        assert_eq!(first_report.received, 1);
        assert_eq!(first_report.accepted, 1);
        assert_eq!(first_report.source_failures, 1);
        assert_eq!(first_report.processed, 1);
        assert_eq!(service.queue_stats().queued, 2);
        assert_eq!(processor.processed, vec![1]);

        let second_report = service.run(&mut adapter, &mut processor);
        assert_eq!(adapter.polls, 2);
        assert_eq!(service.status(), ObserverStatus::Failed);
        assert_eq!(second_report, first_report);
        assert_eq!(service.queue_stats().queued, 2);
        assert_eq!(processor.processed, vec![1]);
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
        assert_eq!(processor.processed, vec![1]);
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
        assert_eq!(report.dropped, 0);
        assert_eq!(report.discontinuities, 0);
        assert_eq!(processor.resets, 0);
        assert_eq!(processor.processed, vec![1, 2, 3]);
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

    #[test]
    fn diagnostic_processor_evaluates_english_and_telex_tokens() {
        let mut processor = DiagnosticDecisionProcessor::default();
        let mut sequence = 1;
        for character in "resume".chars() {
            let key = ObservedKey::letter(character).expect("ASCII letter");
            assert_eq!(
                processor.process(&key_event(
                    sequence,
                    key,
                    KeyEventKind::KeyDown,
                    ModifierState::new(),
                    zonkey_types::InjectionOrigin::PhysicalOrUnmarked,
                )),
                ProcessorClassification::Observed
            );
            sequence += 1;
        }
        assert_eq!(
            processor.process(&key_event(
                sequence,
                ObservedKey::space(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                zonkey_types::InjectionOrigin::PhysicalOrUnmarked,
            )),
            ProcessorClassification::BoundaryObserved
        );
        assert_eq!(processor.completed_tokens(), 1);
        assert_eq!(
            processor.last_decision(),
            Some(DiagnosticDecision::RestoreCandidate)
        );

        for character in "dungf".chars() {
            let key = ObservedKey::letter(character).expect("ASCII letter");
            processor.process(&key_event(
                sequence,
                key,
                KeyEventKind::KeyDown,
                ModifierState::new(),
                zonkey_types::InjectionOrigin::PhysicalOrUnmarked,
            ));
            sequence += 1;
        }
        processor.process(&key_event(
            sequence,
            ObservedKey::enter(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            zonkey_types::InjectionOrigin::PhysicalOrUnmarked,
        ));
        assert_eq!(processor.completed_tokens(), 2);
    }

    #[test]
    fn diagnostic_processor_handles_backspace_shortcuts_shift_keyup_and_injection() {
        let mut processor = DiagnosticDecisionProcessor::default();
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        let shifted = ModifierState::new().with_shift(true);
        assert_eq!(
            processor.process(&key_event(
                1,
                ObservedKey::letter('A').unwrap(),
                KeyEventKind::KeyDown,
                shifted,
                physical,
            )),
            ProcessorClassification::Observed
        );
        assert_eq!(
            processor.process(&key_event(
                2,
                ObservedKey::letter('A').unwrap(),
                KeyEventKind::KeyUp,
                shifted,
                physical,
            )),
            ProcessorClassification::Ignored
        );
        assert_eq!(
            processor.process(&key_event(
                3,
                ObservedKey::backspace(),
                KeyEventKind::KeyDown,
                shifted,
                physical,
            )),
            ProcessorClassification::Observed
        );
        assert_eq!(
            processor.process(&key_event(
                4,
                ObservedKey::letter('C').unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new().with_control(true),
                physical,
            )),
            ProcessorClassification::Ignored
        );
        assert_eq!(
            processor.process(&key_event(
                5,
                ObservedKey::letter('B').unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                zonkey_types::InjectionOrigin::MarkedInjected,
            )),
            ProcessorClassification::Ignored
        );
        assert_eq!(processor.injected_events(), 1);
        processor.reset_after_discontinuity();
        assert_eq!(processor.reset_count(), 1);
        assert_eq!(processor.completed_tokens(), 0);
    }

    #[test]
    fn diagnostic_processor_ignores_repeated_empty_boundaries() {
        let mut processor = DiagnosticDecisionProcessor::default();
        for key in [
            ObservedKey::enter(),
            ObservedKey::space(),
            ObservedKey::tab(),
            ObservedKey::punctuation('.').unwrap(),
        ] {
            assert_eq!(
                processor.process(&key_event(
                    1,
                    key,
                    KeyEventKind::KeyDown,
                    ModifierState::new(),
                    zonkey_types::InjectionOrigin::PhysicalOrUnmarked,
                )),
                ProcessorClassification::BoundaryObserved
            );
        }
        assert_eq!(processor.completed_tokens(), 0);
        assert_eq!(processor.last_decision(), None);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn diagnostic_processor_shortcuts_preserve_token_state() {
        let mut processor = DiagnosticDecisionProcessor::default();
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        for (sequence, character) in [(1, 'a'), (2, 'b')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            3,
            ObservedKey::modifier(zonkey_types::ModifierKey::Control),
            KeyEventKind::KeyDown,
            ModifierState::new().with_control(true),
            physical,
        ));
        processor.process(&key_event(
            4,
            ObservedKey::letter('X').unwrap(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        processor.process(&key_event(
            5,
            ObservedKey::letter('X').unwrap(),
            KeyEventKind::KeyUp,
            ModifierState::new(),
            physical,
        ));
        processor.process(&key_event(
            6,
            ObservedKey::modifier(zonkey_types::ModifierKey::Control),
            KeyEventKind::KeyUp,
            ModifierState::new(),
            physical,
        ));
        for (sequence, character) in [(7, 'c'), (8, 'd')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            9,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(processor.last_token_lengths(), Some((4, 4)));

        for (sequence, character) in [(10, 'a'), (11, 'b')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            12,
            ObservedKey::modifier(zonkey_types::ModifierKey::Alt),
            KeyEventKind::KeyDown,
            ModifierState::new().with_alt(true),
            physical,
        ));
        processor.process(&key_event(
            13,
            ObservedKey::letter('A').unwrap(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        processor.process(&key_event(
            14,
            ObservedKey::modifier(zonkey_types::ModifierKey::Alt),
            KeyEventKind::KeyUp,
            ModifierState::new(),
            physical,
        ));
        processor.process(&key_event(
            15,
            ObservedKey::letter('c').unwrap(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        processor.process(&key_event(
            16,
            ObservedKey::letter('d').unwrap(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        processor.process(&key_event(
            17,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(processor.last_token_lengths(), Some((4, 4)));
    }

    #[test]
    fn diagnostic_processor_backspace_recomposes_hellp_to_hello() {
        let mut processor = DiagnosticDecisionProcessor::default();
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        for (sequence, character) in [(1, 'h'), (2, 'e'), (3, 'l'), (4, 'l'), (5, 'p')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            6,
            ObservedKey::backspace(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        processor.process(&key_event(
            7,
            ObservedKey::letter('o').unwrap(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        processor.process(&key_event(
            8,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(processor.last_token_lengths(), Some((5, 5)));
    }
}

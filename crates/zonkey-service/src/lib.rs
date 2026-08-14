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

/// Read status supplied by a platform adapter for one sampled evidence bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundarySampleStatus {
    /// All requested fields were read.
    Available,
    /// A required read failed.
    Failed,
    /// A bounded read expired.
    TimedOut,
}

#[cfg(test)]
mod boundary_validator_tests {
    use super::{
        BoundaryControlEvidence, BoundaryRejection, BoundarySample, BoundarySampleStatus,
        BoundaryValidation, validate_boundary,
    };

    const CONTROL: BoundaryControlEvidence = BoundaryControlEvidence {
        identity: 7,
        supported: true,
        secure: false,
        style: 1,
    };

    fn sample(text: &[u16], start: usize, end: usize) -> BoundarySample<'_> {
        BoundarySample {
            utf16_text: text,
            start,
            end,
            control: CONTROL,
            status: BoundarySampleStatus::Available,
        }
    }

    fn assert_rejected(
        first: BoundarySample<'_>,
        second: BoundarySample<'_>,
        expected: &str,
        reason: BoundaryRejection,
    ) {
        assert_eq!(
            validate_boundary(first, second, expected),
            BoundaryValidation::Rejected(reason)
        );
    }

    #[test]
    fn exact_ascii_is_validated() {
        let text: Vec<u16> = "resume".encode_utf16().collect();
        assert_eq!(
            validate_boundary(sample(&text, 6, 6), sample(&text, 6, 6), "resume"),
            BoundaryValidation::BoundaryValidated
        );
    }

    #[test]
    fn non_bmp_before_and_inside_candidate_is_validated() {
        let text: Vec<u16> = "x😀resume".encode_utf16().collect();
        assert_eq!(
            validate_boundary(sample(&text, 9, 9), sample(&text, 9, 9), "😀resume"),
            BoundaryValidation::BoundaryValidated
        );
    }

    #[test]
    fn surrogate_split_is_rejected() {
        let text: Vec<u16> = "😀resume".encode_utf16().collect();
        assert_rejected(
            sample(&text, 1, 1),
            sample(&text, 1, 1),
            "",
            BoundaryRejection::SurrogateSplit,
        );
    }

    #[test]
    fn malformed_utf16_is_rejected() {
        let text = [0xd800, u16::from(b'r')];
        assert_rejected(
            sample(&text, 2, 2),
            sample(&text, 2, 2),
            "r",
            BoundaryRejection::MalformedUtf16,
        );
    }

    #[test]
    fn changed_text_and_selection_are_rejected() {
        let first_text: Vec<u16> = "resume".encode_utf16().collect();
        let second_text: Vec<u16> = "resumf".encode_utf16().collect();
        assert_rejected(
            sample(&first_text, 6, 6),
            sample(&second_text, 6, 6),
            "resume",
            BoundaryRejection::SamplesDisagree,
        );
        assert_rejected(
            sample(&first_text, 6, 6),
            sample(&first_text, 0, 2),
            "resume",
            BoundaryRejection::SamplesDisagree,
        );
    }

    #[test]
    fn identity_style_security_and_support_are_rejected() {
        let text: Vec<u16> = "resume".encode_utf16().collect();
        let mut other_identity = sample(&text, 6, 6);
        other_identity.control.identity = 8;
        assert_rejected(
            sample(&text, 6, 6),
            other_identity,
            "resume",
            BoundaryRejection::IdentityChanged,
        );
        let mut other_style = sample(&text, 6, 6);
        other_style.control.style = 2;
        assert_rejected(
            sample(&text, 6, 6),
            other_style,
            "resume",
            BoundaryRejection::StyleChanged,
        );
        let mut secure = sample(&text, 6, 6);
        secure.control.secure = true;
        assert_rejected(secure, secure, "resume", BoundaryRejection::SecureControl);
        let mut unsupported = sample(&text, 6, 6);
        unsupported.control.supported = false;
        assert_rejected(
            unsupported,
            unsupported,
            "resume",
            BoundaryRejection::UnsupportedControl,
        );
    }

    #[test]
    fn prefix_and_range_errors_are_rejected() {
        let text: Vec<u16> = "resume".encode_utf16().collect();
        assert_rejected(
            sample(&text, 6, 6),
            sample(&text, 6, 6),
            "other",
            BoundaryRejection::PrefixMismatch,
        );
        assert_rejected(
            sample(&text, 7, 7),
            sample(&text, 7, 7),
            "resume",
            BoundaryRejection::IndexOutOfBounds,
        );
        assert_rejected(
            sample(&text, 0, 2),
            sample(&text, 0, 2),
            "resume",
            BoundaryRejection::SelectionNotEmpty,
        );
        assert_rejected(
            sample(&text, 6, 5),
            sample(&text, 6, 5),
            "",
            BoundaryRejection::InvalidRange,
        );
    }

    #[test]
    fn unavailable_samples_are_rejected() {
        let text: Vec<u16> = "resume".encode_utf16().collect();
        let mut failed = sample(&text, 6, 6);
        failed.status = BoundarySampleStatus::Failed;
        assert_rejected(
            failed,
            failed,
            "resume",
            BoundaryRejection::SampleUnavailable,
        );
        let mut timeout = sample(&text, 6, 6);
        timeout.status = BoundarySampleStatus::TimedOut;
        assert_rejected(
            timeout,
            timeout,
            "resume",
            BoundaryRejection::SampleUnavailable,
        );
    }
}

/// Sanitized capability and identity evidence for one sampled control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundaryControlEvidence {
    /// Stable identity token supplied by the adapter; never a native handle.
    pub identity: u64,
    /// Whether the adapter proved the supported standard-EDIT class.
    pub supported: bool,
    /// Whether the adapter proved the control is secure/password-like.
    pub secure: bool,
    /// Sanitized style evidence used for sample agreement.
    pub style: u32,
}

/// One borrowed UTF-16 evidence sample from a platform adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundarySample<'a> {
    /// Complete sampled UTF-16 text. No offset is derived from scalar lengths.
    pub utf16_text: &'a [u16],
    /// Selection/caret start in UTF-16 code units.
    pub start: usize,
    /// Selection/caret end in UTF-16 code units.
    pub end: usize,
    /// Sanitized control evidence.
    pub control: BoundaryControlEvidence,
    /// Read status for this complete sample.
    pub status: BoundarySampleStatus,
}

/// Result of validating a sampled standard-EDIT range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryValidation {
    /// Internally valid for the supplied sampled UTF-16 state only.
    BoundaryValidated,
    /// Rejected before any execution consideration.
    Rejected(BoundaryRejection),
}

/// Fail-closed reasons for boundary validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryRejection {
    SampleUnavailable,
    SamplesDisagree,
    IdentityChanged,
    StyleChanged,
    UnsupportedControl,
    SecureControl,
    SelectionNotEmpty,
    IndexOutOfBounds,
    InvalidRange,
    MalformedUtf16,
    SurrogateSplit,
    PrefixMismatch,
}

/// Validates two bounded, read-only standard-EDIT samples.
///
/// The result describes only internal validity of the sampled UTF-16 state. It
/// is not a freshness, atomicity, mutation-safety, or execution authorization
/// decision.
#[must_use]
pub fn validate_boundary(
    first: BoundarySample<'_>,
    second: BoundarySample<'_>,
    expected: &str,
) -> BoundaryValidation {
    if first.status != BoundarySampleStatus::Available
        || second.status != BoundarySampleStatus::Available
    {
        return BoundaryValidation::Rejected(BoundaryRejection::SampleUnavailable);
    }
    if first.control.identity != second.control.identity {
        return BoundaryValidation::Rejected(BoundaryRejection::IdentityChanged);
    }
    if first.control.style != second.control.style {
        return BoundaryValidation::Rejected(BoundaryRejection::StyleChanged);
    }
    if first.control != second.control
        || first.utf16_text != second.utf16_text
        || first.start != second.start
        || first.end != second.end
    {
        return BoundaryValidation::Rejected(BoundaryRejection::SamplesDisagree);
    }
    if !second.control.supported {
        return BoundaryValidation::Rejected(BoundaryRejection::UnsupportedControl);
    }
    if second.control.secure {
        return BoundaryValidation::Rejected(BoundaryRejection::SecureControl);
    }
    if second.start > second.utf16_text.len() || second.end > second.utf16_text.len() {
        return BoundaryValidation::Rejected(BoundaryRejection::IndexOutOfBounds);
    }
    if second.start > second.end {
        return BoundaryValidation::Rejected(BoundaryRejection::InvalidRange);
    }
    if second.start != second.end {
        return BoundaryValidation::Rejected(BoundaryRejection::SelectionNotEmpty);
    }
    if !is_well_formed_utf16(second.utf16_text) {
        return BoundaryValidation::Rejected(BoundaryRejection::MalformedUtf16);
    }
    if splits_surrogate(second.utf16_text, second.start)
        || splits_surrogate(second.utf16_text, second.end)
    {
        return BoundaryValidation::Rejected(BoundaryRejection::SurrogateSplit);
    }
    let expected_units: Vec<u16> = expected.encode_utf16().collect();
    if expected_units.len() > second.start
        || second.utf16_text[second.start - expected_units.len()..second.start] != expected_units
    {
        return BoundaryValidation::Rejected(BoundaryRejection::PrefixMismatch);
    }
    BoundaryValidation::BoundaryValidated
}

fn is_well_formed_utf16(text: &[u16]) -> bool {
    char::decode_utf16(text.iter().copied()).all(|value| value.is_ok())
}

fn splits_surrogate(text: &[u16], index: usize) -> bool {
    index > 0
        && index < text.len()
        && (0xd800..=0xdbff).contains(&text[index - 1])
        && (0xdc00..=0xdfff).contains(&text[index])
}

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

/// A bounded, platform-neutral description of a restore that would be
/// appropriate for one completed token. It is data only: no execution API is
/// provided and `execution_allowed` is permanently false.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestorePlan {
    /// The intended token reconstructed from the raw token state.
    pub original_token: String,
    /// The token currently rendered by Telex.
    pub rendered_token: String,
    /// The replacement text that simulation would use.
    pub replacement_token: String,
    /// Number of Unicode scalar values in the rendered token.
    pub rendered_units_to_replace: usize,
    /// Number of Unicode scalar values in the replacement token.
    pub replacement_units: usize,
    /// Existing policy evidence that justified the candidate.
    pub reason: zonkey_types::DecisionReason,
    /// Always false; execution belongs to a separately approved milestone.
    execution_allowed: bool,
    generation: u64,
}

impl RestorePlan {
    /// Returns false for every plan; M3C-01 has no execution capability.
    #[must_use]
    pub const fn execution_allowed(&self) -> bool {
        self.execution_allowed
    }

    /// Returns the service-local identity captured for this plan.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// Fail-closed result for simulation eligibility, never OS editability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanEligibility {
    /// The current plan is internally consistent and may be considered by a
    /// separately approved future execution boundary.
    EligibleForFutureExecutionConsideration,
    /// No current plan or an internal invariant is not satisfied.
    Ineligible(PlanIneligibilityReason),
}

/// Reasons observable by the platform-neutral simulation layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanIneligibilityReason {
    /// The processor does not currently hold a plan.
    NoCurrentPlan,
    /// Stored logical lengths do not match the stored token values.
    InternalSpanInconsistent,
    /// The plan is not marked as simulation-only.
    ExecutionCapabilityPresent,
}

/// Validates one optional plan without I/O, mutation, or policy reruns.
#[must_use]
pub fn validate_restore_plan(plan: Option<&RestorePlan>) -> PlanEligibility {
    let Some(plan) = plan else {
        return PlanEligibility::Ineligible(PlanIneligibilityReason::NoCurrentPlan);
    };
    if plan.execution_allowed() {
        return PlanEligibility::Ineligible(PlanIneligibilityReason::ExecutionCapabilityPresent);
    }
    if plan.original_token.is_empty()
        || plan.replacement_token.is_empty()
        || plan.rendered_units_to_replace != plan.rendered_token.chars().count()
        || plan.replacement_units != plan.replacement_token.chars().count()
    {
        return PlanEligibility::Ineligible(PlanIneligibilityReason::InternalSpanInconsistent);
    }
    PlanEligibility::EligibleForFutureExecutionConsideration
}

/// Immutable capture-time snapshot of an eligible simulation plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestorePlanHandoff {
    /// Rendered token captured from the current plan.
    pub rendered_token: String,
    /// Replacement token captured from the current plan.
    pub replacement_token: String,
    /// Rendered span length in Unicode scalar values.
    pub rendered_units_to_replace: usize,
    /// Replacement length in Unicode scalar values.
    pub replacement_units: usize,
    /// Policy evidence captured with the plan.
    pub reason: zonkey_types::DecisionReason,
    /// Service-local plan identity at capture time.
    pub generation: u64,
    simulation_only: bool,
}

impl RestorePlanHandoff {
    /// Returns true to make the non-execution boundary explicit.
    #[must_use]
    pub const fn simulation_only(&self) -> bool {
        self.simulation_only
    }
}

/// Result of comparing a captured handoff with current service state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandoffRevalidation {
    /// The handoff still matches the current logical plan.
    Current,
    /// The handoff is not current for a service-owned reason.
    Stale(HandoffStaleReason),
}

/// Fail-closed reasons for service-local handoff revalidation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffStaleReason {
    NoCurrentPlan,
    DifferentGeneration,
    MalformedSnapshot,
}

/// Service-side gate result; passing only permits a future external
/// validation stage and never authorizes mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InternalExecutionGate {
    PassedForExternalValidation,
    Rejected(InternalGateRejection),
}

/// Fail-closed reasons derived from existing service contracts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InternalGateRejection {
    NoCurrentPlan,
    PlanIneligible,
    NoCurrentHandoff,
    HandoffStale,
    HandoffMalformed,
    GenerationMismatch,
    InternalSpanInconsistent,
    SimulationInvariantBroken,
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
    last_restore_plan: Option<RestorePlan>,
    next_plan_generation: Option<u64>,
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
            last_restore_plan: None,
            next_plan_generation: Some(1),
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

    /// Returns the latest bounded simulation plan, if the last completed
    /// token produced a restore candidate.
    #[must_use]
    pub fn last_restore_plan(&self) -> Option<&RestorePlan> {
        self.last_restore_plan.as_ref()
    }

    /// Validates the current plan using only processor-owned state.
    #[must_use]
    pub fn plan_eligibility(&self) -> PlanEligibility {
        validate_restore_plan(self.last_restore_plan())
    }

    /// Captures the current eligible plan as an owned historical snapshot.
    ///
    /// The returned value is not linked to processor state and cannot be
    /// presented back as current by this API.
    #[must_use]
    pub fn current_restore_handoff(&self) -> Option<RestorePlanHandoff> {
        if !matches!(
            self.plan_eligibility(),
            PlanEligibility::EligibleForFutureExecutionConsideration
        ) {
            return None;
        }
        let plan = self.last_restore_plan.as_ref()?;
        Some(RestorePlanHandoff {
            rendered_token: plan.rendered_token.clone(),
            replacement_token: plan.replacement_token.clone(),
            rendered_units_to_replace: plan.rendered_units_to_replace,
            replacement_units: plan.replacement_units,
            reason: plan.reason.clone(),
            generation: plan.generation(),
            simulation_only: true,
        })
    }

    /// Revalidates a captured handoff against only current service state.
    #[must_use]
    pub fn revalidate_restore_handoff(&self, handoff: &RestorePlanHandoff) -> HandoffRevalidation {
        let Some(plan) = self.last_restore_plan.as_ref() else {
            return HandoffRevalidation::Stale(HandoffStaleReason::NoCurrentPlan);
        };
        if !handoff.simulation_only
            || handoff.rendered_token.is_empty()
            || handoff.replacement_token.is_empty()
            || handoff.rendered_units_to_replace != handoff.rendered_token.chars().count()
            || handoff.replacement_units != handoff.replacement_token.chars().count()
        {
            return HandoffRevalidation::Stale(HandoffStaleReason::MalformedSnapshot);
        }
        if !matches!(
            self.plan_eligibility(),
            PlanEligibility::EligibleForFutureExecutionConsideration
        ) {
            return HandoffRevalidation::Stale(HandoffStaleReason::NoCurrentPlan);
        }
        if handoff.generation != plan.generation()
            || handoff.rendered_token != plan.rendered_token
            || handoff.replacement_token != plan.replacement_token
            || handoff.rendered_units_to_replace != plan.rendered_units_to_replace
            || handoff.replacement_units != plan.replacement_units
            || handoff.reason != plan.reason
        {
            return HandoffRevalidation::Stale(HandoffStaleReason::DifferentGeneration);
        }
        HandoffRevalidation::Current
    }

    /// Composes current plan eligibility and handoff revalidation. A pass is
    /// only a stop at the boundary before future external validation.
    #[must_use]
    pub fn evaluate_internal_execution_gate(
        &self,
        handoff: &RestorePlanHandoff,
    ) -> InternalExecutionGate {
        if !matches!(
            self.plan_eligibility(),
            PlanEligibility::EligibleForFutureExecutionConsideration
        ) {
            return InternalExecutionGate::Rejected(InternalGateRejection::PlanIneligible);
        }
        let Some(current) = self.current_restore_handoff() else {
            return InternalExecutionGate::Rejected(InternalGateRejection::NoCurrentHandoff);
        };
        if !handoff.simulation_only() {
            return InternalExecutionGate::Rejected(
                InternalGateRejection::SimulationInvariantBroken,
            );
        }
        match self.revalidate_restore_handoff(handoff) {
            HandoffRevalidation::Current => {
                if current.generation == handoff.generation {
                    InternalExecutionGate::PassedForExternalValidation
                } else {
                    InternalExecutionGate::Rejected(InternalGateRejection::GenerationMismatch)
                }
            }
            HandoffRevalidation::Stale(HandoffStaleReason::NoCurrentPlan) => {
                InternalExecutionGate::Rejected(InternalGateRejection::NoCurrentPlan)
            }
            HandoffRevalidation::Stale(HandoffStaleReason::DifferentGeneration) => {
                InternalExecutionGate::Rejected(InternalGateRejection::GenerationMismatch)
            }
            HandoffRevalidation::Stale(HandoffStaleReason::MalformedSnapshot) => {
                InternalExecutionGate::Rejected(InternalGateRejection::HandoffMalformed)
            }
        }
    }

    fn reset_token(&mut self) {
        self.telex = TelexEngine::new();
    }

    fn allocate_plan_generation(&mut self) -> Option<u64> {
        let generation = self.next_plan_generation?;
        self.next_plan_generation = generation.checked_add(1);
        Some(generation)
    }

    fn invalidate_restore_plan(&mut self) {
        self.last_restore_plan = None;
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
        self.last_restore_plan = None;
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
            zonkey_types::RecoveryDecision::RestoreEnglish { text, reason, .. } => {
                let Some(generation) = self.allocate_plan_generation() else {
                    self.last_restore_plan = None;
                    self.last_decision = Some(DiagnosticDecision::Ambiguous);
                    let _ = self.telex.process(EngineEvent::Boundary(boundary));
                    return ProcessorClassification::BoundaryObserved;
                };
                self.last_restore_plan = Some(RestorePlan {
                    original_token: raw.clone(),
                    rendered_token: rendered.clone(),
                    replacement_token: text.clone(),
                    rendered_units_to_replace: rendered.chars().count(),
                    replacement_units: text.chars().count(),
                    reason,
                    execution_allowed: false,
                    generation,
                });
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
        if decision == DiagnosticDecision::RestoreCandidate {
            let plan = self
                .last_restore_plan
                .as_ref()
                .expect("restore candidate always has a simulation plan");
            println!("restore_plan=yes eligibility=simulation-current");
            if self.show_token {
                println!(
                    "restore_plan=yes rendered={:?} replacement={:?}",
                    plan.rendered_token, plan.replacement_token
                );
            } else {
                println!(
                    "restore_plan=yes replace_len={} replacement_len={}",
                    plan.rendered_units_to_replace, plan.replacement_units
                );
            }
        } else {
            println!("restore_plan=no eligibility=no-plan");
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
        self.last_restore_plan = None;
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
            self.last_restore_plan = None;
            return ProcessorClassification::Ignored;
        }
        if event.key.is_backspace() {
            self.invalidate_restore_plan();
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
            self.last_restore_plan = None;
            return ProcessorClassification::Unsupported;
        };
        self.invalidate_restore_plan();
        if self
            .telex
            .process(EngineEvent::Character(character))
            .is_ok()
        {
            ProcessorClassification::Observed
        } else {
            self.reset_token();
            self.last_restore_plan = None;
            ProcessorClassification::Unsupported
        }
    }
}

#[cfg(test)]
#[allow(dead_code)]
mod controlled_surface {
    use super::RestorePlanHandoff;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct TextUnit(u32);

    impl TextUnit {
        pub(super) const fn new(value: u32) -> Self {
            Self(value)
        }
        pub(super) const fn get(self) -> u32 {
            self.0
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct SurfaceId(u64);

    impl SurfaceId {
        pub(super) const fn new(value: u64) -> Self {
            Self(value)
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum SelectionState {
        None,
        Range { start: TextUnit, end: TextUnit },
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum CompositionState {
        Inactive,
        Active,
        Unknown,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum SecureState {
        KnownNonSecure,
        Secure,
        Unknown,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum SessionState {
        SupportedLocal,
        UnsupportedRemote,
        Unknown,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum ControlledValidationError {
        TargetIdentityMismatch,
        RevisionMismatch,
        TextMismatch,
        CaretMismatch,
        SelectionNotEmpty,
        CompositionActive,
        CompositionUnknown,
        SecureTarget,
        SecureUnknown,
        UnsupportedSession,
        SessionUnknown,
        OperationUnitsUnproven,
        RangeInvalid,
        RevisionOverflow,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) struct ControlledEvidenceSnapshot {
        pub(super) surface_id: SurfaceId,
        pub(super) revision: u64,
        pub(super) text: String,
        pub(super) range: (TextUnit, TextUnit),
        pub(super) caret: TextUnit,
        pub(super) selection: SelectionState,
        pub(super) operation_units_proven: bool,
        pub(super) composition: CompositionState,
        pub(super) secure: SecureState,
        pub(super) session: SessionState,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) struct ControlledSurface {
        surface_id: SurfaceId,
        text: String,
        caret: TextUnit,
        selection: SelectionState,
        operation_units_proven: bool,
        composition: CompositionState,
        secure: SecureState,
        session: SessionState,
        revision: u64,
    }

    impl ControlledSurface {
        pub(super) fn new(surface_id: SurfaceId, text: impl Into<String>) -> Self {
            Self {
                surface_id,
                text: text.into(),
                caret: TextUnit::new(0),
                selection: SelectionState::None,
                operation_units_proven: true,
                composition: CompositionState::Inactive,
                secure: SecureState::KnownNonSecure,
                session: SessionState::SupportedLocal,
                revision: 1,
            }
        }
        fn units(&self) -> u32 {
            self.text.chars().count().try_into().unwrap_or(u32::MAX)
        }
        fn bump(&mut self) -> Result<(), ControlledValidationError> {
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or(ControlledValidationError::RevisionOverflow)?;
            Ok(())
        }
        pub(super) fn text(&self) -> &str {
            &self.text
        }
        pub(super) fn revision(&self) -> u64 {
            self.revision
        }
        pub(super) fn set_caret(
            &mut self,
            caret: TextUnit,
        ) -> Result<(), ControlledValidationError> {
            if caret.get() > self.units() {
                return Err(ControlledValidationError::RangeInvalid);
            }
            self.bump()?;
            self.caret = caret;
            self.selection = SelectionState::None;
            Ok(())
        }
        pub(super) fn set_selection(
            &mut self,
            selection: SelectionState,
        ) -> Result<(), ControlledValidationError> {
            if let SelectionState::Range { start, end } = selection
                && (start.get() > end.get() || end.get() > self.units())
            {
                return Err(ControlledValidationError::RangeInvalid);
            }
            self.bump()?;
            self.selection = selection;
            Ok(())
        }
        pub(super) fn replace_text(
            &mut self,
            text: impl Into<String>,
        ) -> Result<(), ControlledValidationError> {
            self.bump()?;
            self.text = text.into();
            if self.caret.get() > self.units() {
                self.caret = TextUnit::new(self.units());
            }
            self.selection = SelectionState::None;
            Ok(())
        }
        pub(super) fn set_surface_id(
            &mut self,
            id: SurfaceId,
        ) -> Result<(), ControlledValidationError> {
            self.bump()?;
            self.surface_id = id;
            Ok(())
        }
        pub(super) fn set_composition(
            &mut self,
            value: CompositionState,
        ) -> Result<(), ControlledValidationError> {
            self.bump()?;
            self.composition = value;
            Ok(())
        }
        pub(super) fn set_secure(
            &mut self,
            value: SecureState,
        ) -> Result<(), ControlledValidationError> {
            self.bump()?;
            self.secure = value;
            Ok(())
        }
        pub(super) fn set_session(
            &mut self,
            value: SessionState,
        ) -> Result<(), ControlledValidationError> {
            self.bump()?;
            self.session = value;
            Ok(())
        }
        pub(super) fn set_operation_units_proven(
            &mut self,
            value: bool,
        ) -> Result<(), ControlledValidationError> {
            self.bump()?;
            self.operation_units_proven = value;
            Ok(())
        }
        pub(super) fn capture(
            &self,
            rendered: &str,
        ) -> Result<ControlledEvidenceSnapshot, ControlledValidationError> {
            let length: u32 = rendered
                .chars()
                .count()
                .try_into()
                .map_err(|_| ControlledValidationError::RangeInvalid)?;
            if length > self.caret.get() {
                return Err(ControlledValidationError::RangeInvalid);
            }
            let start = TextUnit::new(self.caret.get() - length);
            let actual: String = self
                .text
                .chars()
                .skip(start.get() as usize)
                .take(length as usize)
                .collect();
            if actual != rendered {
                return Err(ControlledValidationError::TextMismatch);
            }
            Ok(ControlledEvidenceSnapshot {
                surface_id: self.surface_id,
                revision: self.revision,
                text: actual,
                range: (start, self.caret),
                caret: self.caret,
                selection: self.selection,
                operation_units_proven: self.operation_units_proven,
                composition: self.composition,
                secure: self.secure,
                session: self.session,
            })
        }
        pub(super) fn validate(
            &self,
            handoff: &RestorePlanHandoff,
            snapshot: &ControlledEvidenceSnapshot,
        ) -> Result<(), ControlledValidationError> {
            if snapshot.surface_id != self.surface_id {
                return Err(ControlledValidationError::TargetIdentityMismatch);
            }
            if snapshot.revision != self.revision {
                return Err(ControlledValidationError::RevisionMismatch);
            }
            if snapshot.caret != self.caret {
                return Err(ControlledValidationError::CaretMismatch);
            }
            if snapshot.range.1 != self.caret || snapshot.text != handoff.rendered_token {
                return Err(ControlledValidationError::TextMismatch);
            }
            if !matches!(self.selection, SelectionState::None) {
                return Err(ControlledValidationError::SelectionNotEmpty);
            }
            match self.composition {
                CompositionState::Inactive => {}
                CompositionState::Active => {
                    return Err(ControlledValidationError::CompositionActive);
                }
                CompositionState::Unknown => {
                    return Err(ControlledValidationError::CompositionUnknown);
                }
            }
            match self.secure {
                SecureState::KnownNonSecure => {}
                SecureState::Secure => return Err(ControlledValidationError::SecureTarget),
                SecureState::Unknown => return Err(ControlledValidationError::SecureUnknown),
            }
            match self.session {
                SessionState::SupportedLocal => {}
                SessionState::UnsupportedRemote => {
                    return Err(ControlledValidationError::UnsupportedSession);
                }
                SessionState::Unknown => return Err(ControlledValidationError::SessionUnknown),
            }
            if !self.operation_units_proven {
                return Err(ControlledValidationError::OperationUnitsUnproven);
            }
            Ok(())
        }
        pub(super) fn compare_revision_and_replace(
            &mut self,
            handoff: &RestorePlanHandoff,
            snapshot: &ControlledEvidenceSnapshot,
            replacement: &str,
        ) -> Result<(), ControlledValidationError> {
            self.validate(handoff, snapshot)?;
            self.bump()?;
            let (start, end) = snapshot.range;
            let chars: Vec<char> = self.text.chars().collect();
            let mut next = String::new();
            next.extend(chars[..start.get() as usize].iter());
            next.push_str(replacement);
            next.extend(chars[end.get() as usize..].iter());
            self.text = next;
            let replacement_units = u32::try_from(replacement.chars().count())
                .map_err(|_| ControlledValidationError::RangeInvalid)?;
            self.caret = TextUnit::new(
                start
                    .get()
                    .checked_add(replacement_units)
                    .ok_or(ControlledValidationError::RangeInvalid)?,
            );
            self.selection = SelectionState::None;
            Ok(())
        }
        #[cfg(test)]
        pub(super) fn set_revision_for_test(&mut self, revision: u64) {
            self.revision = revision;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::{DiagnosticDecisionProcessor, EventProcessor};
        use zonkey_types::{
            EventSequence, InjectionOrigin, KeyEventKind, ModifierState, ObservedInputEvent,
            ObservedKey,
        };
        fn handoff(rendered: &str) -> RestorePlanHandoff {
            RestorePlanHandoff {
                rendered_token: rendered.into(),
                replacement_token: "resume".into(),
                rendered_units_to_replace: rendered.chars().count(),
                replacement_units: 6,
                reason: zonkey_types::DecisionReason::ExactEnglishDictionary,
                generation: 1,
                simulation_only: true,
            }
        }

        #[test]
        fn real_service_handoff_binds_to_controlled_surface_and_race() {
            let mut processor = DiagnosticDecisionProcessor::default();
            for (sequence, character) in
                [(1, 'r'), (2, 'e'), (3, 's'), (4, 'u'), (5, 'm'), (6, 'e')]
            {
                processor.process(&ObservedInputEvent {
                    key: ObservedKey::letter(character).unwrap(),
                    kind: KeyEventKind::KeyDown,
                    modifiers: ModifierState::new(),
                    injection_origin: InjectionOrigin::PhysicalOrUnmarked,
                    sequence: EventSequence::new(sequence).unwrap(),
                });
            }
            processor.process(&ObservedInputEvent {
                key: ObservedKey::space(),
                kind: KeyEventKind::KeyDown,
                modifiers: ModifierState::new(),
                injection_origin: InjectionOrigin::PhysicalOrUnmarked,
                sequence: EventSequence::new(7).unwrap(),
            });
            let handoff = processor
                .current_restore_handoff()
                .expect("resume creates a handoff");
            let mut surface =
                ControlledSurface::new(SurfaceId::new(7), handoff.rendered_token.clone());
            surface
                .set_caret(TextUnit::new(
                    u32::try_from(handoff.rendered_units_to_replace).unwrap(),
                ))
                .unwrap();
            let snapshot = surface.capture(&handoff.rendered_token).unwrap();
            assert_eq!(surface.validate(&handoff, &snapshot), Ok(()));
            surface.set_secure(SecureState::Secure).unwrap();
            assert_eq!(
                surface.validate(&handoff, &snapshot),
                Err(ControlledValidationError::RevisionMismatch)
            );
            assert_eq!(surface.text(), handoff.rendered_token);
        }
        #[test]
        fn controlled_surface_validates_exact_unicode_range() {
            let mut s = ControlledSurface::new(SurfaceId::new(1), "rÃ©ume");
            s.set_caret(TextUnit::new(6)).unwrap();
            let snap = s.capture("rÃ©ume").unwrap();
            assert_eq!(snap.text, "rÃ©ume");
            assert_eq!(snap.range.0.get(), 0);
        }
        #[test]
        fn controlled_surface_rejects_selection_and_unknown_states() {
            let mut s = ControlledSurface::new(SurfaceId::new(1), "resume");
            s.set_caret(TextUnit::new(6)).unwrap();
            let h = handoff("resume");
            let snap = s.capture("resume").unwrap();
            s.set_selection(SelectionState::Range {
                start: TextUnit::new(0),
                end: TextUnit::new(1),
            })
            .unwrap();
            assert_eq!(
                s.validate(&h, &snap),
                Err(ControlledValidationError::RevisionMismatch)
            );
            let _ = s.set_selection(SelectionState::None);
            s.set_composition(CompositionState::Unknown).unwrap();
            let snap = s.capture("resume").unwrap();
            assert_eq!(
                s.validate(&h, &snap),
                Err(ControlledValidationError::CompositionUnknown)
            );
        }
        #[test]
        fn compare_replace_is_atomic_and_rejects_stale_snapshot() {
            let mut s = ControlledSurface::new(SurfaceId::new(1), "resume");
            s.set_caret(TextUnit::new(6)).unwrap();
            let h = handoff("resume");
            let snap = s.capture("resume").unwrap();
            s.replace_text("resume").unwrap();
            assert_eq!(
                s.compare_revision_and_replace(&h, &snap, "x"),
                Err(ControlledValidationError::RevisionMismatch)
            );
            assert_eq!(s.text(), "resume");
        }
        #[test]
        fn compare_replace_happy_path_updates_once() {
            let mut s = ControlledSurface::new(SurfaceId::new(1), "rÃ©ume");
            s.set_caret(TextUnit::new(6)).unwrap();
            let h = handoff("rÃ©ume");
            let snap = s.capture("rÃ©ume").unwrap();
            let rev = s.revision();
            s.compare_revision_and_replace(&h, &snap, "resume").unwrap();
            assert_eq!(s.text(), "resume");
            assert_eq!(s.revision(), rev + 1);
        }
        #[test]
        fn revision_overflow_fails_closed() {
            let mut s = ControlledSurface::new(SurfaceId::new(1), "x");
            s.set_revision_for_test(u64::MAX);
            assert_eq!(
                s.set_caret(TextUnit::new(0)),
                Err(ControlledValidationError::RevisionOverflow)
            );
        }
        #[test]
        fn identical_surfaces_do_not_share_identity() {
            let mut a = ControlledSurface::new(SurfaceId::new(1), "resume");
            let mut b = ControlledSurface::new(SurfaceId::new(2), "resume");
            a.set_caret(TextUnit::new(6)).unwrap();
            b.set_caret(TextUnit::new(6)).unwrap();
            let snap = a.capture("resume").unwrap();
            let h = handoff("resume");
            assert_eq!(
                b.validate(&h, &snap),
                Err(ControlledValidationError::TargetIdentityMismatch)
            );
        }
    }
}

#[cfg(test)]
mod dummy_host {
    use std::collections::BTreeMap;

    pub const PROTOCOL_ID: &str = "zonkey.inproc-host/1";
    const UNIT_SCHEMA: u16 = 1;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TargetIdentity {
        document_id: u64,
        control_id: u64,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TextRange {
        unit_schema: u16,
        start: u32,
        end: u32,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SelectionState {
        Empty,
        Range { start: u32, end: u32 },
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SecureState {
        KnownNonSecure,
        Secure,
        Unknown,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum CompositionState {
        Inactive,
        Active,
        Unknown,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SessionState {
        SupportedLocal,
        UnsupportedRemote,
        Unknown,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Capabilities {
        flags: u8,
        unit_schema: u16,
    }

    impl Capabilities {
        const ALL: Self = Self {
            flags: 0b0000_1111,
            unit_schema: UNIT_SCHEMA,
        };
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct RequestId(u64);

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum IndeterminateReason {
        InjectedAmbiguousCommit,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum RejectReason {
        MalformedRequest,
        ProtocolMismatch,
        AuthenticationFailed,
        HostIdentityMismatch,
        SessionMismatch,
        TargetIdentityMismatch,
        RevisionMismatch,
        TextMismatch,
        RangeMismatch,
        UnitMismatch,
        CaretMismatch,
        SelectionNotEmpty,
        SecureTarget,
        SecureUnknown,
        CompositionActive,
        CompositionUnknown,
        UnsupportedSession,
        SessionUnknown,
        CapabilityMismatch,
        RequestIdReuse,
        RevisionOverflow,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum HostResult {
        Applied { new_revision: u64 },
        Rejected(RejectReason),
        Indeterminate(IndeterminateReason),
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CompareAndReplaceRequest {
        protocol_id: String,
        authenticated: bool,
        host_id: u64,
        session_id: u64,
        request_id: RequestId,
        identity: TargetIdentity,
        revision: u64,
        expected_range: TextRange,
        expected_text: String,
        replacement: String,
        caret: u32,
        selection: SelectionState,
        secure: SecureState,
        composition: CompositionState,
        session: SessionState,
        capabilities: Capabilities,
        force_indeterminate: bool,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CachedResult {
        request: CompareAndReplaceRequest,
        result: HostResult,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct HostSnapshot {
        host_id: u64,
        session_id: u64,
        identity: TargetIdentity,
        revision: u64,
        unit_schema: u16,
        text: String,
        caret: u32,
        selection: SelectionState,
        secure: SecureState,
        composition: CompositionState,
        session: SessionState,
        capabilities: Capabilities,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct DummyHost {
        host_id: u64,
        session_id: u64,
        identity: TargetIdentity,
        revision: u64,
        unit_schema: u16,
        text: String,
        caret: u32,
        selection: SelectionState,
        secure: SecureState,
        composition: CompositionState,
        session: SessionState,
        capabilities: Capabilities,
        request_results: BTreeMap<RequestId, CachedResult>,
    }

    impl DummyHost {
        fn new(
            host_id: u64,
            session_id: u64,
            document_id: u64,
            control_id: u64,
            text: &str,
        ) -> Self {
            Self {
                host_id,
                session_id,
                identity: TargetIdentity {
                    document_id,
                    control_id,
                },
                revision: 1,
                unit_schema: UNIT_SCHEMA,
                text: text.to_owned(),
                caret: 0,
                selection: SelectionState::Empty,
                secure: SecureState::KnownNonSecure,
                composition: CompositionState::Inactive,
                session: SessionState::SupportedLocal,
                capabilities: Capabilities::ALL,
                request_results: BTreeMap::new(),
            }
        }

        fn units(&self) -> u32 {
            self.text.chars().count().try_into().unwrap_or(u32::MAX)
        }

        fn snapshot(&self) -> HostSnapshot {
            HostSnapshot {
                host_id: self.host_id,
                session_id: self.session_id,
                identity: self.identity,
                revision: self.revision,
                unit_schema: self.unit_schema,
                text: self.text.clone(),
                caret: self.caret,
                selection: self.selection,
                secure: self.secure,
                composition: self.composition,
                session: self.session,
                capabilities: self.capabilities,
            }
        }

        fn request(
            &self,
            request_id: RequestId,
            expected_range: TextRange,
            expected_text: &str,
            replacement: &str,
        ) -> CompareAndReplaceRequest {
            CompareAndReplaceRequest {
                protocol_id: PROTOCOL_ID.to_owned(),
                authenticated: true,
                host_id: self.host_id,
                session_id: self.session_id,
                request_id,
                identity: self.identity,
                revision: self.revision,
                expected_range,
                expected_text: expected_text.to_owned(),
                replacement: replacement.to_owned(),
                caret: self.caret,
                selection: self.selection,
                secure: self.secure,
                composition: self.composition,
                session: self.session,
                capabilities: self.capabilities,
                force_indeterminate: false,
            }
        }

        fn bump(&mut self) -> Result<u64, RejectReason> {
            let next = self
                .revision
                .checked_add(1)
                .ok_or(RejectReason::RevisionOverflow)?;
            self.revision = next;
            Ok(next)
        }

        fn set_secure(&mut self, value: SecureState) -> Result<(), RejectReason> {
            self.bump()?;
            self.secure = value;
            Ok(())
        }

        fn set_composition(&mut self, value: CompositionState) -> Result<(), RejectReason> {
            self.bump()?;
            self.composition = value;
            Ok(())
        }

        fn set_session(&mut self, value: SessionState) -> Result<(), RejectReason> {
            self.bump()?;
            self.session = value;
            Ok(())
        }

        fn set_selection(&mut self, value: SelectionState) -> Result<(), RejectReason> {
            if let SelectionState::Range { start, end } = value
                && (start > end || end > self.units())
            {
                return Err(RejectReason::RangeMismatch);
            }
            self.bump()?;
            self.selection = value;
            Ok(())
        }

        fn restart(&mut self) -> Result<(), RejectReason> {
            let next_session_id = self
                .session_id
                .checked_add(1)
                .ok_or(RejectReason::RevisionOverflow)?;
            let next_revision = self
                .revision
                .checked_add(1)
                .ok_or(RejectReason::RevisionOverflow)?;
            self.session_id = next_session_id;
            self.revision = next_revision;
            self.request_results.clear();
            Ok(())
        }

        fn compare_and_replace(&mut self, request: CompareAndReplaceRequest) -> HostResult {
            if request.request_id.0 == 0 {
                return HostResult::Rejected(RejectReason::MalformedRequest);
            }
            if let Some(cached) = self.request_results.get(&request.request_id) {
                if cached.request == request {
                    return cached.result.clone();
                }
                return HostResult::Rejected(RejectReason::RequestIdReuse);
            }

            let result = self.validate_and_apply(&request);
            self.request_results.insert(
                request.request_id,
                CachedResult {
                    request,
                    result: result.clone(),
                },
            );
            result
        }

        #[allow(clippy::too_many_lines)]
        fn validate_and_apply(&mut self, request: &CompareAndReplaceRequest) -> HostResult {
            if request.protocol_id != PROTOCOL_ID {
                return HostResult::Rejected(RejectReason::ProtocolMismatch);
            }
            if !request.authenticated {
                return HostResult::Rejected(RejectReason::AuthenticationFailed);
            }
            if request.host_id != self.host_id {
                return HostResult::Rejected(RejectReason::HostIdentityMismatch);
            }
            if request.session_id != self.session_id {
                return HostResult::Rejected(RejectReason::SessionMismatch);
            }
            if request.identity != self.identity {
                return HostResult::Rejected(RejectReason::TargetIdentityMismatch);
            }
            if request.capabilities != self.capabilities {
                return HostResult::Rejected(RejectReason::CapabilityMismatch);
            }
            if request.expected_range.unit_schema != self.unit_schema {
                return HostResult::Rejected(RejectReason::UnitMismatch);
            }
            if request.revision != self.revision {
                return HostResult::Rejected(RejectReason::RevisionMismatch);
            }
            if request.expected_range.start > request.expected_range.end
                || request.expected_range.end > self.units()
            {
                return HostResult::Rejected(RejectReason::RangeMismatch);
            }
            if request.caret != self.caret || request.expected_range.end != request.caret {
                return HostResult::Rejected(RejectReason::CaretMismatch);
            }
            if !matches!(request.selection, SelectionState::Empty)
                || !matches!(self.selection, SelectionState::Empty)
            {
                return HostResult::Rejected(RejectReason::SelectionNotEmpty);
            }
            if request.selection != self.selection {
                return HostResult::Rejected(RejectReason::SelectionNotEmpty);
            }
            if request.secure != self.secure {
                return HostResult::Rejected(RejectReason::SecureTarget);
            }
            match self.secure {
                SecureState::KnownNonSecure => {}
                SecureState::Secure => {
                    return HostResult::Rejected(RejectReason::SecureTarget);
                }
                SecureState::Unknown => {
                    return HostResult::Rejected(RejectReason::SecureUnknown);
                }
            }
            if request.composition != self.composition {
                return HostResult::Rejected(RejectReason::CompositionActive);
            }
            match self.composition {
                CompositionState::Inactive => {}
                CompositionState::Active => {
                    return HostResult::Rejected(RejectReason::CompositionActive);
                }
                CompositionState::Unknown => {
                    return HostResult::Rejected(RejectReason::CompositionUnknown);
                }
            }
            if request.session != self.session {
                return HostResult::Rejected(RejectReason::SessionMismatch);
            }
            match self.session {
                SessionState::SupportedLocal => {}
                SessionState::UnsupportedRemote => {
                    return HostResult::Rejected(RejectReason::UnsupportedSession);
                }
                SessionState::Unknown => {
                    return HostResult::Rejected(RejectReason::SessionUnknown);
                }
            }

            let start = request.expected_range.start as usize;
            let end = request.expected_range.end as usize;
            let chars: Vec<char> = self.text.chars().collect();
            let actual: String = chars[start..end].iter().collect();
            if actual != request.expected_text {
                return HostResult::Rejected(RejectReason::TextMismatch);
            }
            let Ok(replacement_units) = u32::try_from(request.replacement.chars().count()) else {
                return HostResult::Rejected(RejectReason::RangeMismatch);
            };
            let Some(new_revision) = self.revision.checked_add(1) else {
                return HostResult::Rejected(RejectReason::RevisionOverflow);
            };
            let mut next = String::new();
            next.extend(chars[..start].iter());
            next.push_str(&request.replacement);
            next.extend(chars[end..].iter());
            let Some(new_caret) = request.expected_range.start.checked_add(replacement_units)
            else {
                return HostResult::Rejected(RejectReason::RangeMismatch);
            };

            self.text = next;
            self.caret = new_caret;
            self.selection = SelectionState::Empty;
            self.revision = new_revision;
            if request.force_indeterminate {
                HostResult::Indeterminate(IndeterminateReason::InjectedAmbiguousCommit)
            } else {
                HostResult::Applied { new_revision }
            }
        }

        #[cfg(test)]
        fn set_revision_for_test(&mut self, revision: u64) {
            self.revision = revision;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn host() -> DummyHost {
            let mut host = DummyHost::new(1, 10, 20, 30, "resume");
            host.caret = 6;
            host
        }

        fn request(host: &DummyHost, request_id: u64) -> CompareAndReplaceRequest {
            host.request(
                RequestId(request_id),
                TextRange {
                    unit_schema: UNIT_SCHEMA,
                    start: 0,
                    end: 6,
                },
                "resume",
                "restored",
            )
        }

        #[test]
        fn successful_apply_is_atomic_and_advances_revision() {
            let mut host = host();
            let before = host.snapshot();
            let result = host.compare_and_replace(request(&host, 1));
            assert_eq!(result, HostResult::Applied { new_revision: 2 });
            assert_eq!(host.text, "restored");
            assert_eq!(host.revision, before.revision + 1);
            assert_eq!(host.caret, 8);
            assert_eq!(host.selection, SelectionState::Empty);
        }

        #[test]
        fn stale_revision_rejects_without_partial_mutation() {
            let mut host = host();
            let mut request = request(&host, 1);
            request.revision = 0;
            let before = host.snapshot();
            assert_eq!(
                host.compare_and_replace(request),
                HostResult::Rejected(RejectReason::RevisionMismatch)
            );
            assert_eq!(host.snapshot(), before);
        }

        #[test]
        fn wrong_document_control_and_session_reject() {
            let mut host = host();
            let mut wrong_document = request(&host, 1);
            wrong_document.identity.document_id = 99;
            assert_eq!(
                host.compare_and_replace(wrong_document),
                HostResult::Rejected(RejectReason::TargetIdentityMismatch)
            );
            let mut wrong_control = request(&host, 2);
            wrong_control.identity.control_id = 99;
            assert_eq!(
                host.compare_and_replace(wrong_control),
                HostResult::Rejected(RejectReason::TargetIdentityMismatch)
            );
            let mut wrong_session = request(&host, 3);
            wrong_session.session_id = 11;
            assert_eq!(
                host.compare_and_replace(wrong_session),
                HostResult::Rejected(RejectReason::SessionMismatch)
            );
        }

        #[test]
        fn text_and_range_mismatch_reject_without_mutation() {
            let mut host = host();
            let before = host.snapshot();
            let mut text_mismatch = request(&host, 1);
            text_mismatch.expected_text = "resume!".into();
            assert_eq!(
                host.compare_and_replace(text_mismatch),
                HostResult::Rejected(RejectReason::TextMismatch)
            );
            let mut range_mismatch = request(&host, 2);
            range_mismatch.expected_range.end = 5;
            assert_eq!(
                host.compare_and_replace(range_mismatch),
                HostResult::Rejected(RejectReason::CaretMismatch)
            );
            assert_eq!(host.snapshot(), before);
        }

        #[test]
        fn unit_protocol_and_capability_mismatch_reject() {
            let mut host = host();
            let mut unit = request(&host, 1);
            unit.expected_range.unit_schema = 2;
            assert_eq!(
                host.compare_and_replace(unit),
                HostResult::Rejected(RejectReason::UnitMismatch)
            );
            let mut protocol = request(&host, 2);
            protocol.protocol_id = "zonkey.inproc-host/0".into();
            assert_eq!(
                host.compare_and_replace(protocol),
                HostResult::Rejected(RejectReason::ProtocolMismatch)
            );
            let mut capabilities = request(&host, 3);
            capabilities.capabilities.flags &= !0b0000_0001;
            assert_eq!(
                host.compare_and_replace(capabilities),
                HostResult::Rejected(RejectReason::CapabilityMismatch)
            );
        }

        #[test]
        fn secure_composition_and_session_states_reject() {
            let mut secure = host();
            secure.set_secure(SecureState::Secure).unwrap();
            assert_eq!(
                secure.compare_and_replace(request(&secure, 1)),
                HostResult::Rejected(RejectReason::SecureTarget)
            );
            let mut unknown_secure = host();
            unknown_secure.set_secure(SecureState::Unknown).unwrap();
            assert_eq!(
                unknown_secure.compare_and_replace(request(&unknown_secure, 1)),
                HostResult::Rejected(RejectReason::SecureUnknown)
            );
            let mut composition = host();
            composition
                .set_composition(CompositionState::Active)
                .unwrap();
            assert_eq!(
                composition.compare_and_replace(request(&composition, 1)),
                HostResult::Rejected(RejectReason::CompositionActive)
            );
            let mut unknown_composition = host();
            unknown_composition
                .set_composition(CompositionState::Unknown)
                .unwrap();
            assert_eq!(
                unknown_composition.compare_and_replace(request(&unknown_composition, 1)),
                HostResult::Rejected(RejectReason::CompositionUnknown)
            );
            let mut session = host();
            session
                .set_session(SessionState::UnsupportedRemote)
                .unwrap();
            assert_eq!(
                session.compare_and_replace(request(&session, 1)),
                HostResult::Rejected(RejectReason::UnsupportedSession)
            );
            let mut unknown_session = host();
            unknown_session.set_session(SessionState::Unknown).unwrap();
            assert_eq!(
                unknown_session.compare_and_replace(request(&unknown_session, 1)),
                HostResult::Rejected(RejectReason::SessionUnknown)
            );
        }

        #[test]
        fn duplicate_exact_request_is_idempotent_and_conflicting_reuse_rejects() {
            let mut host = host();
            let original_request = request(&host, 1);
            let first = host.compare_and_replace(original_request.clone());
            let second = host.compare_and_replace(original_request);
            assert_eq!(first, HostResult::Applied { new_revision: 2 });
            assert_eq!(second, first);
            assert_eq!(host.text, "restored");
            let mut conflict = request(&host, 1);
            conflict.revision = 2;
            conflict.expected_text = "restored".into();
            conflict.expected_range.end = 8;
            conflict.caret = 8;
            assert_eq!(
                host.compare_and_replace(conflict),
                HostResult::Rejected(RejectReason::RequestIdReuse)
            );
        }

        #[test]
        fn restart_invalidates_old_session_requests() {
            let mut host = host();
            let old = request(&host, 1);
            host.restart().unwrap();
            assert_eq!(host.session_id, 11);
            assert_eq!(
                host.compare_and_replace(old),
                HostResult::Rejected(RejectReason::SessionMismatch)
            );
        }

        #[test]
        fn malformed_request_and_overflow_fail_closed() {
            let mut host = host();
            let mut malformed = request(&host, 0);
            assert_eq!(
                host.compare_and_replace(malformed.clone()),
                HostResult::Rejected(RejectReason::MalformedRequest)
            );
            malformed.request_id = RequestId(1);
            malformed.expected_range.start = 7;
            malformed.expected_range.end = 6;
            let before = host.snapshot();
            assert_eq!(
                host.compare_and_replace(malformed),
                HostResult::Rejected(RejectReason::RangeMismatch)
            );
            assert_eq!(host.snapshot(), before);

            host.set_revision_for_test(u64::MAX);
            let overflow = request(&host, 2);
            let before = host.snapshot();
            assert_eq!(
                host.compare_and_replace(overflow),
                HostResult::Rejected(RejectReason::RevisionOverflow)
            );
            assert_eq!(host.snapshot(), before);
        }

        #[test]
        fn indeterminate_result_is_cached_without_automatic_retry() {
            let mut host = host();
            let mut request = request(&host, 1);
            request.force_indeterminate = true;
            let result = host.compare_and_replace(request.clone());
            assert_eq!(
                result,
                HostResult::Indeterminate(IndeterminateReason::InjectedAmbiguousCommit)
            );
            assert_eq!(host.text, "restored");
            assert_eq!(host.revision, 2);
            assert_eq!(host.compare_and_replace(request), result);
            assert_eq!(host.revision, 2);
        }

        #[test]
        fn non_empty_selection_rejects_without_mutation() {
            let mut host = host();
            host.set_selection(SelectionState::Range { start: 0, end: 1 })
                .unwrap();
            let before = host.snapshot();
            assert_eq!(
                host.compare_and_replace(request(&host, 1)),
                HostResult::Rejected(RejectReason::SelectionNotEmpty)
            );
            assert_eq!(host.snapshot(), before);
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
        let plan = processor
            .last_restore_plan()
            .expect("restore candidate has a simulation plan");
        assert_eq!(plan.original_token, "resume");
        assert_eq!(plan.replacement_token, "resume");
        assert!(!plan.rendered_token.is_empty());
        assert!(!plan.execution_allowed());
        assert_eq!(
            processor.plan_eligibility(),
            PlanEligibility::EligibleForFutureExecutionConsideration
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
        assert_eq!(processor.last_restore_plan(), None);
        assert_eq!(
            processor.plan_eligibility(),
            PlanEligibility::Ineligible(PlanIneligibilityReason::NoCurrentPlan)
        );
    }

    #[test]
    fn diagnostic_restore_plan_is_only_for_restore_candidates() {
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        let mut processor = DiagnosticDecisionProcessor::default();
        assert_eq!(
            processor.plan_eligibility(),
            PlanEligibility::Ineligible(PlanIneligibilityReason::NoCurrentPlan)
        );

        for (sequence, character) in [(1, 'd'), (2, 'u'), (3, 'n'), (4, 'g'), (5, 'f')] {
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
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(processor.last_decision(), Some(DiagnosticDecision::Keep));
        assert_eq!(processor.last_restore_plan(), None);

        for (sequence, character) in [(7, 'h'), (8, 'e'), (9, 'l'), (10, 'l'), (11, 'o')] {
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
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(
            processor.last_decision(),
            Some(DiagnosticDecision::Ambiguous)
        );
        assert_eq!(processor.last_restore_plan(), None);

        assert_eq!(
            processor.process(&key_event(
                13,
                ObservedKey::other(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            )),
            ProcessorClassification::Unsupported
        );
        assert_eq!(processor.last_restore_plan(), None);

        processor.process(&key_event(
            14,
            ObservedKey::enter(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(processor.last_restore_plan(), None);
    }

    #[test]
    fn discontinuity_clears_restore_plan() {
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        let mut processor = DiagnosticDecisionProcessor::default();
        for (sequence, character) in [(1, 'r'), (2, 'e'), (3, 's'), (4, 'u'), (5, 'm'), (6, 'e')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            7,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert!(processor.last_restore_plan().is_some());
        processor.reset_after_discontinuity();
        assert_eq!(processor.last_restore_plan(), None);
        assert_eq!(
            processor.plan_eligibility(),
            PlanEligibility::Ineligible(PlanIneligibilityReason::NoCurrentPlan)
        );
    }

    #[test]
    fn validator_rejects_inconsistent_internal_span() {
        let mut processor = DiagnosticDecisionProcessor::default();
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        for (sequence, character) in [(1, 'r'), (2, 'e'), (3, 's'), (4, 'u'), (5, 'm'), (6, 'e')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            7,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        let mut malformed = processor.last_restore_plan().unwrap().clone();
        malformed.rendered_token.push('x');
        assert_eq!(
            validate_restore_plan(Some(&malformed)),
            PlanEligibility::Ineligible(PlanIneligibilityReason::InternalSpanInconsistent)
        );
    }

    #[test]
    fn eligible_plan_creates_immutable_handoff_snapshot() {
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        let mut processor = DiagnosticDecisionProcessor::default();
        for (sequence, character) in [(1, 'r'), (2, 'e'), (3, 's'), (4, 'u'), (5, 'm'), (6, 'e')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            7,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        let handoff = processor
            .current_restore_handoff()
            .expect("eligible plan has a handoff");
        assert_eq!(handoff.replacement_token, "resume");
        assert_eq!(handoff.rendered_units_to_replace, 5);
        assert_eq!(handoff.replacement_units, 6);
        assert!(handoff.simulation_only());
        assert_eq!(
            processor.revalidate_restore_handoff(&handoff),
            HandoffRevalidation::Current
        );
        let mut malformed = handoff.clone();
        malformed.rendered_token.push('x');
        assert_eq!(
            processor.revalidate_restore_handoff(&malformed),
            HandoffRevalidation::Stale(HandoffStaleReason::MalformedSnapshot)
        );

        processor.process(&key_event(
            8,
            ObservedKey::letter('h').unwrap(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(processor.current_restore_handoff(), None);
        assert_eq!(
            processor.revalidate_restore_handoff(&handoff),
            HandoffRevalidation::Stale(HandoffStaleReason::NoCurrentPlan)
        );
        assert_eq!(handoff.replacement_token, "resume");
    }

    #[test]
    fn new_candidate_has_a_new_current_handoff() {
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        let mut first = DiagnosticDecisionProcessor::default();
        for (sequence, character) in [(1, 'r'), (2, 'e'), (3, 's'), (4, 'u'), (5, 'm'), (6, 'e')] {
            first.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        first.process(&key_event(
            7,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        let handoff_a = first.current_restore_handoff().unwrap();

        let mut second = DiagnosticDecisionProcessor::default();
        for (sequence, character) in [(1, 'c'), (2, 'o'), (3, 'n'), (4, 'f'), (5, 'i'), (6, 'g')] {
            second.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        second.process(&key_event(
            7,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        let handoff_b = second.current_restore_handoff().unwrap();
        assert_eq!(handoff_a.replacement_token, "resume");
        assert_eq!(handoff_b.replacement_token, "config");
        assert_ne!(handoff_a, handoff_b);
    }

    #[test]
    fn same_content_after_invalidation_has_new_generation() {
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        let mut processor = DiagnosticDecisionProcessor::default();
        for (sequence, character) in [(1, 'r'), (2, 'e'), (3, 's'), (4, 'u'), (5, 'm'), (6, 'e')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            7,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        let handoff_a = processor.current_restore_handoff().unwrap();
        let generation_a = handoff_a.generation;

        processor.process(&key_event(
            8,
            ObservedKey::letter('h').unwrap(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(
            processor.revalidate_restore_handoff(&handoff_a),
            HandoffRevalidation::Stale(HandoffStaleReason::NoCurrentPlan)
        );
        processor.process(&key_event(
            9,
            ObservedKey::escape(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));

        for (sequence, character) in [
            (10, 'r'),
            (11, 'e'),
            (12, 's'),
            (13, 'u'),
            (14, 'm'),
            (15, 'e'),
        ] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            16,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        let handoff_b = processor.current_restore_handoff().unwrap();
        assert!(handoff_b.generation > generation_a);
        assert_eq!(
            processor.revalidate_restore_handoff(&handoff_b),
            HandoffRevalidation::Current
        );
        assert_eq!(
            processor.revalidate_restore_handoff(&handoff_a),
            HandoffRevalidation::Stale(HandoffStaleReason::DifferentGeneration)
        );
    }

    #[test]
    fn internal_gate_composes_current_evidence_and_rejects_stale() {
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        let mut processor = DiagnosticDecisionProcessor::default();
        for (sequence, character) in [(1, 'r'), (2, 'e'), (3, 's'), (4, 'u'), (5, 'm'), (6, 'e')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            7,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        let handoff = processor.current_restore_handoff().unwrap();
        assert_eq!(
            processor.evaluate_internal_execution_gate(&handoff),
            InternalExecutionGate::PassedForExternalValidation
        );
        processor.process(&key_event(
            8,
            ObservedKey::letter('h').unwrap(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(
            processor.evaluate_internal_execution_gate(&handoff),
            InternalExecutionGate::Rejected(InternalGateRejection::PlanIneligible)
        );
    }

    #[test]
    fn internal_gate_rejects_no_plan_and_malformed_handoff() {
        let processor = DiagnosticDecisionProcessor::default();
        assert_eq!(
            processor.evaluate_internal_execution_gate(&RestorePlanHandoff {
                rendered_token: String::new(),
                replacement_token: String::new(),
                rendered_units_to_replace: 0,
                replacement_units: 0,
                reason: zonkey_types::DecisionReason::ExactEnglishDictionary,
                generation: 0,
                simulation_only: true,
            }),
            InternalExecutionGate::Rejected(InternalGateRejection::PlanIneligible)
        );
    }

    #[test]
    fn semantic_input_invalidates_previous_restore_plan() {
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        let mut processor = DiagnosticDecisionProcessor::default();
        for (sequence, character) in [(1, 'r'), (2, 'e'), (3, 's'), (4, 'u'), (5, 'm'), (6, 'e')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            7,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert!(processor.last_restore_plan().is_some());

        processor.process(&key_event(
            8,
            ObservedKey::letter('h').unwrap(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(processor.last_restore_plan(), None);
    }

    #[test]
    fn later_keep_or_ambiguous_decision_clears_previous_plan() {
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        let mut processor = DiagnosticDecisionProcessor::default();
        for (sequence, character) in [(1, 'r'), (2, 'e'), (3, 's'), (4, 'u'), (5, 'm'), (6, 'e')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            7,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert!(processor.last_restore_plan().is_some());

        for (sequence, character) in [(8, 'd'), (9, 'u'), (10, 'n'), (11, 'g'), (12, 'f')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            13,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(processor.last_decision(), Some(DiagnosticDecision::Keep));
        assert_eq!(processor.last_restore_plan(), None);

        for (sequence, character) in [
            (14, 'r'),
            (15, 'e'),
            (16, 's'),
            (17, 'u'),
            (18, 'm'),
            (19, 'e'),
        ] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            20,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert!(processor.last_restore_plan().is_some());
        for (sequence, character) in [(21, 'h'), (22, 'e'), (23, 'l'), (24, 'l'), (25, 'o')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            26,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(
            processor.last_decision(),
            Some(DiagnosticDecision::Ambiguous)
        );
        assert_eq!(processor.last_restore_plan(), None);
    }

    #[test]
    fn new_restore_candidate_replaces_previous_plan_and_transitions_do_not() {
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        let mut processor = DiagnosticDecisionProcessor::default();
        for (sequence, character) in [(1, 'r'), (2, 'e'), (3, 's'), (4, 'u'), (5, 'm'), (6, 'e')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            7,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(
            processor.last_restore_plan().unwrap().original_token,
            "resume"
        );

        processor.process(&key_event(
            8,
            ObservedKey::letter('A').unwrap(),
            KeyEventKind::KeyUp,
            ModifierState::new(),
            physical,
        ));
        processor.process(&key_event(
            9,
            ObservedKey::modifier(zonkey_types::ModifierKey::Shift),
            KeyEventKind::KeyDown,
            ModifierState::new().with_shift(true),
            physical,
        ));
        assert_eq!(
            processor.last_restore_plan().unwrap().original_token,
            "resume"
        );

        for (sequence, character) in [
            (10, 'r'),
            (11, 'e'),
            (12, 'f'),
            (13, 'r'),
            (14, 'e'),
            (15, 's'),
            (16, 'h'),
        ] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            17,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert_eq!(
            processor.last_restore_plan().unwrap().original_token,
            "refresh"
        );
    }

    #[test]
    fn injected_event_does_not_create_or_clear_plan() {
        let physical = zonkey_types::InjectionOrigin::PhysicalOrUnmarked;
        let mut processor = DiagnosticDecisionProcessor::default();
        for (sequence, character) in [(1, 'r'), (2, 'e'), (3, 's'), (4, 'u'), (5, 'm'), (6, 'e')] {
            processor.process(&key_event(
                sequence,
                ObservedKey::letter(character).unwrap(),
                KeyEventKind::KeyDown,
                ModifierState::new(),
                physical,
            ));
        }
        processor.process(&key_event(
            7,
            ObservedKey::space(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            physical,
        ));
        assert!(processor.last_restore_plan().is_some());
        processor.process(&key_event(
            8,
            ObservedKey::letter('x').unwrap(),
            KeyEventKind::KeyDown,
            ModifierState::new(),
            zonkey_types::InjectionOrigin::MarkedInjected,
        ));
        assert!(processor.last_restore_plan().is_some());
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

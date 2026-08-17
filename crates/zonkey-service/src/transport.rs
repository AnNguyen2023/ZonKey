//! Bounded transport boundary for the cooperating-host protocol (M3D-19).
//!
//! This module is platform-neutral plumbing only: length-prefixed framing,
//! protocol/session binding, and a bounded idempotency ledger. Payloads stay
//! opaque UTF-8 strings; host-contract JSON is parsed by the endpoints, not
//! here. The module never opens sockets, reads clocks, mutates editor state,
//! or retries anything: a request whose result was lost to a timeout or a
//! dropped connection is recorded as `Ambiguous` and replayed as-is on the
//! next identical request. Trust is bound to the session established at
//! hello time; process ids, window handles, and port numbers are never
//! accepted as identity.
//!
//! The destination transport for Windows 11 x64 is a localhost named pipe;
//! that OS binding is deliberately not implemented in this crate.

use std::collections::{HashMap, VecDeque};

use crate::EventProcessor as _;
use crate::recovery_codec;

/// Protocol identity required at hello time on both endpoints.
pub const TRANSPORT_PROTOCOL_ID: &str = "zonkey.host-transport/1";

/// Hard frame ceiling; larger frames are rejected before allocation.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// Default bounded ledger capacity.
pub const DEFAULT_LEDGER_CAPACITY: usize = 256;

const FRAME_HEADER_BYTES: usize = 4;

/// Fail-closed framing errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// A frame with a zero-length payload was rejected.
    EmptyFrame,
    /// A frame exceeding [`MAX_FRAME_BYTES`] was rejected.
    OversizedFrame,
    /// A payload was not valid UTF-8.
    InvalidUtf8,
}

/// One decoded frame; the payload is opaque to this module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Opaque UTF-8 payload owned by the endpoints.
    pub payload: String,
}

/// Encodes one frame: little-endian `u32` length prefix plus payload bytes.
///
/// # Errors
///
/// Returns [`FrameError::EmptyFrame`] or [`FrameError::OversizedFrame`] for
/// empty or oversized payloads.
///
/// # Panics
///
/// Never panics in practice; the internal length conversion is bounded by
/// [`MAX_FRAME_BYTES`].
pub fn encode_frame(payload: &str) -> Result<Vec<u8>, FrameError> {
    if payload.is_empty() {
        return Err(FrameError::EmptyFrame);
    }
    if payload.len() > MAX_FRAME_BYTES {
        return Err(FrameError::OversizedFrame);
    }
    let length = u32::try_from(payload.len()).expect("length bounded by MAX_FRAME_BYTES");
    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(payload.as_bytes());
    Ok(frame)
}

/// Streaming decode: consumes one complete frame from the buffer front.
///
/// Returns `Ok(None)` while fewer bytes than one complete frame are
/// available. Malformed or oversized frames are hard errors; the caller must
/// drop the connection rather than resynchronize.
///
/// # Errors
///
/// Returns a [`FrameError`] for empty, oversized, or non-UTF-8 frames.
pub fn decode_frame(buffer: &mut &[u8]) -> Result<Option<Frame>, FrameError> {
    if buffer.len() < FRAME_HEADER_BYTES {
        return Ok(None);
    }
    let length = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
    if length == 0 {
        return Err(FrameError::EmptyFrame);
    }
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::OversizedFrame);
    }
    let end = FRAME_HEADER_BYTES + length;
    if buffer.len() < end {
        return Ok(None);
    }
    let payload = std::str::from_utf8(&buffer[FRAME_HEADER_BYTES..end])
        .map_err(|_| FrameError::InvalidUtf8)?
        .to_owned();
    *buffer = &buffer[end..];
    Ok(Some(Frame { payload }))
}

/// Idempotency classification of one result entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LedgerOutcome {
    /// A definite result (for example Applied or Rejected): replay is
    /// authoritative because the host transaction finished observably.
    Definite(String),
    /// An indeterminate result (lost or ambiguous): replayed as-is and
    /// never re-executed by this layer.
    Ambiguous(String),
}

/// Disposition of one incoming request id against the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestDisposition {
    /// Unknown or evicted id: the caller may execute and must record.
    Fresh,
    /// Exact duplicate: replay the recorded outcome without re-executing.
    Duplicate(LedgerOutcome),
    /// Known id with a different canonical request: reject.
    Conflict,
}

/// Error returned when a ledger capacity of zero is requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LedgerCapacityError;

/// Result of recording one outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordOutcome {
    /// Recorded; the ledger was below capacity.
    Inserted,
    /// Recorded; the oldest inserted evictable id was evicted
    /// deterministically.
    Evicted { evicted: String },
    /// The id already existed; nothing changed.
    AlreadyPresent,
    /// The ledger is at capacity and every retained entry is an unbacked
    /// `Ambiguous` outcome, which is pinned and never evicted (ADR 0035);
    /// nothing was recorded and the caller must surface the lost replay
    /// protection.
    RejectedLedgerFull,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LedgerEntry {
    canonical: String,
    outcome: LedgerOutcome,
    /// True when a durable recovery record backs this request (M3D-31
    /// preflight): its Ambiguous outcome may be evicted because the durable
    /// recovery state remains the source of truth.
    recovery_backed: bool,
}

/// A bounded request ledger with deterministic eviction.
///
/// Unbacked `Ambiguous` entries are pinned: eviction removes only definite
/// entries or `Ambiguous` entries marked recovery-backed (whose durable
/// recovery record persists independently). When every retained entry is an
/// unbacked ambiguous outcome the ledger refuses to record (fail closed)
/// rather than forget a result whose loss is unresolved (ADR 0035). Lookups
/// never refresh order; all outcome kinds are retained and replayed
/// verbatim.
#[derive(Debug)]
pub struct BoundedRequestLedger {
    capacity: usize,
    order: VecDeque<String>,
    entries: HashMap<String, LedgerEntry>,
}

impl BoundedRequestLedger {
    /// Constructs a ledger with a validated non-zero capacity.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerCapacityError`] when `capacity` is zero.
    pub fn new(capacity: usize) -> Result<Self, LedgerCapacityError> {
        if capacity == 0 {
            return Err(LedgerCapacityError);
        }
        Ok(Self {
            capacity,
            order: VecDeque::with_capacity(capacity.min(1024)),
            entries: HashMap::new(),
        })
    }

    /// Configured maximum number of retained entries.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current number of retained entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entry is retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Classifies one request id and canonical form against the ledger.
    #[must_use]
    pub fn classify(&self, request_id: &str, canonical: &str) -> RequestDisposition {
        match self.entries.get(request_id) {
            None => RequestDisposition::Fresh,
            Some(entry) if entry.canonical == canonical => {
                RequestDisposition::Duplicate(entry.outcome.clone())
            }
            Some(_) => RequestDisposition::Conflict,
        }
    }

    /// Records one outcome, evicting the oldest inserted evictable entry
    /// when full: definite entries, and `Ambiguous` entries whose request is
    /// recovery-backed. Unbacked ambiguous entries are pinned; a ledger that
    /// holds only unbacked ambiguous entries at capacity refuses to record
    /// and returns [`RecordOutcome::RejectedLedgerFull`].
    pub fn record(
        &mut self,
        request_id: &str,
        canonical: &str,
        outcome: LedgerOutcome,
    ) -> RecordOutcome {
        self.record_inner(request_id, canonical, outcome, false)
    }

    /// Records one outcome for a request whose durable recovery record
    /// already exists (M3D-31 preflight): its Ambiguous outcome may later be
    /// evicted because the durable recovery state remains the source of
    /// truth.
    pub fn record_with_recovery(
        &mut self,
        request_id: &str,
        canonical: &str,
        outcome: LedgerOutcome,
    ) -> RecordOutcome {
        self.record_inner(request_id, canonical, outcome, true)
    }

    fn record_inner(
        &mut self,
        request_id: &str,
        canonical: &str,
        outcome: LedgerOutcome,
        recovery_backed: bool,
    ) -> RecordOutcome {
        if self.entries.contains_key(request_id) {
            return RecordOutcome::AlreadyPresent;
        }
        let evicted = if self.order.len() < self.capacity {
            None
        } else {
            let evictable = self.order.iter().position(|id| match self.entries.get(id) {
                Some(entry) => {
                    !matches!(entry.outcome, LedgerOutcome::Ambiguous(_)) || entry.recovery_backed
                }
                None => true,
            });
            match evictable {
                Some(index) => self.order.remove(index).inspect(|oldest| {
                    self.entries.remove(oldest);
                }),
                None => return RecordOutcome::RejectedLedgerFull,
            }
        };
        self.order.push_back(request_id.to_owned());
        self.entries.insert(
            request_id.to_owned(),
            LedgerEntry {
                canonical: canonical.to_owned(),
                outcome,
                recovery_backed,
            },
        );
        match evicted {
            Some(evicted) => RecordOutcome::Evicted { evicted },
            None => RecordOutcome::Inserted,
        }
    }

    /// Clears all history; called on session restart.
    pub fn invalidate_all(&mut self) {
        self.order.clear();
        self.entries.clear();
    }
}

/// Fail-closed endpoint errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// No hello has established a session yet.
    NotEstablished,
    /// The hello protocol id did not match.
    ProtocolMismatch,
    /// A hello was structurally invalid (empty protocol or session id).
    InvalidHello,
    /// The request session id does not match the established session.
    SessionMismatch,
}

/// Host-side session state over the bounded ledger.
///
/// `accept_hello` binds one session id after verifying the protocol id; a
/// different session id on a later hello is a restart and invalidates the
/// whole ledger. Requests are checked against the established session and
/// classified by the ledger. Timeouts and connection loss after sending a
/// request but before receiving a result are mapped by the caller to
/// [`ambiguous_loss_outcome`] and recorded, never retried here.
#[derive(Debug)]
pub struct HostTransportEndpoint {
    protocol_id: &'static str,
    session_id: Option<String>,
    ledger: BoundedRequestLedger,
}

impl HostTransportEndpoint {
    /// Creates an unestablished endpoint with a bounded ledger.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerCapacityError`] when `ledger_capacity` is zero.
    pub fn new(ledger_capacity: usize) -> Result<Self, LedgerCapacityError> {
        Ok(Self {
            protocol_id: TRANSPORT_PROTOCOL_ID,
            session_id: None,
            ledger: BoundedRequestLedger::new(ledger_capacity)?,
        })
    }

    /// Handles one hello. Returns `true` when an established session was
    /// replaced (restart) and the ledger was invalidated.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::ProtocolMismatch`] for a wrong protocol id
    /// and [`TransportError::InvalidHello`] for empty identifiers; neither
    /// changes endpoint state.
    pub fn accept_hello(
        &mut self,
        protocol_id: &str,
        session_id: &str,
    ) -> Result<bool, TransportError> {
        if protocol_id.is_empty() || session_id.is_empty() {
            return Err(TransportError::InvalidHello);
        }
        if protocol_id != self.protocol_id {
            return Err(TransportError::ProtocolMismatch);
        }
        let restart = self
            .session_id
            .as_deref()
            .is_some_and(|current| current != session_id);
        if restart {
            self.ledger.invalidate_all();
        }
        self.session_id = Some(session_id.to_owned());
        Ok(restart)
    }

    /// The established session id, if any.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Verifies one request's session id against the established session.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::NotEstablished`] before any hello and
    /// [`TransportError::SessionMismatch`] on any disagreement.
    pub fn check_session(&self, session_id: &str) -> Result<(), TransportError> {
        match self.session_id.as_deref() {
            None => Err(TransportError::NotEstablished),
            Some(current) if current == session_id => Ok(()),
            Some(_) => Err(TransportError::SessionMismatch),
        }
    }

    /// Classifies one request against the bounded ledger.
    #[must_use]
    pub fn classify_request(&self, request_id: &str, canonical: &str) -> RequestDisposition {
        self.ledger.classify(request_id, canonical)
    }

    /// Records one request outcome into the bounded ledger.
    pub fn record(
        &mut self,
        request_id: &str,
        canonical: &str,
        outcome: LedgerOutcome,
    ) -> RecordOutcome {
        self.ledger.record(request_id, canonical, outcome)
    }

    /// Records one request outcome whose durable recovery record already
    /// exists (M3D-31 preflight): the entry may be evicted even when
    /// ambiguous, because the durable recovery state is the source of truth.
    pub fn record_with_recovery(
        &mut self,
        request_id: &str,
        canonical: &str,
        outcome: LedgerOutcome,
    ) -> RecordOutcome {
        self.ledger
            .record_with_recovery(request_id, canonical, outcome)
    }

    /// Read-only view of the bounded ledger.
    #[must_use]
    pub fn ledger(&self) -> &BoundedRequestLedger {
        &self.ledger
    }
}

/// Minimal platform-neutral recovery descriptor carried with any
/// future mutation-capable host request (M3D-31 / ADR 0036). The UTF-16
/// range is host-owned snapshot data carried verbatim — it is never derived
/// from service scalar token lengths. Persistence hashes the expected and
/// replacement values with the state-file salt; plaintext never reaches
/// disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryDescriptor {
    /// Request id of the carrying request.
    pub request_id: String,
    /// Logical target identity: document URI.
    pub uri: String,
    /// Host-owned UTF-16 range of the compare-and-replace target.
    pub range: (usize, usize),
    /// Expected rendered value; hash input on persistence.
    pub expected: String,
    /// Intended replacement value; hash input on persistence.
    pub replacement: String,
    /// Service-local plan generation at capture time.
    pub generation: u64,
}

/// The canonical outcome for a request whose result was lost to a timeout or
/// a dropped connection after the request was sent. Recording it keeps the
/// request id idempotent without ever re-executing the request here.
#[must_use]
pub fn ambiguous_loss_outcome() -> LedgerOutcome {
    LedgerOutcome::Ambiguous("connection_lost_before_result".to_owned())
}

/// Query-only composition gate for the M3D-21 validation endpoint.
///
/// Maps the envelope-declared composition state to a definite, replayable
/// rejection outcome. `Unknown` and any unrecognized value fail closed as
/// `CompositionUnknown`; `Active` fails closed as `CompositionActive`; even
/// a proven `Inactive` state is rejected because no host execution path is
/// approved in this milestone. This function never authorizes mutation.
#[must_use]
pub fn composition_gate_outcome(composition: &str) -> LedgerOutcome {
    match composition {
        "Inactive" => LedgerOutcome::Definite("rejected:ExecutionNotImplemented".to_owned()),
        "Active" => LedgerOutcome::Definite("rejected:CompositionActive".to_owned()),
        _ => LedgerOutcome::Definite("rejected:CompositionUnknown".to_owned()),
    }
}

/// Fields of a validated restore handoff mapped toward a host request (M3D-22).
///
/// The mapping deliberately excludes any host-native position or range: the
/// UTF-16 range, caret, and document identity remain owned by the host
/// snapshot/adapter. `request_id` is derived deterministically from the plan
/// generation so ledger idempotency follows naturally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffRequest {
    /// Deterministic identity: `handoff-<generation>`.
    pub request_id: String,
    /// Token currently rendered by Telex.
    pub rendered_token: String,
    /// Replacement text chosen by policy.
    pub replacement_token: String,
    /// Unicode scalar length of the rendered token.
    pub rendered_units: usize,
    /// Unicode scalar length of the replacement token.
    pub replacement_units: usize,
    /// Service-local plan generation at capture time.
    pub generation: u64,
}

/// Fail-closed reasons a handoff never reaches the transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffRequestError {
    /// No current plan or handoff exists.
    NoCurrentPlan,
    /// The submitted handoff no longer matches service state.
    StaleHandoff,
    /// The submitted generation disagrees with the current plan.
    GenerationMismatch,
    /// Logical span lengths disagree with the token values.
    MalformedSpan,
    /// The internal execution gate refused the handoff.
    InternalGateFailed,
}

/// Maps a submitted [`RestorePlanHandoff`] through revalidation and the
/// internal execution gate into a [`HandoffRequest`]. Every failure mode
/// rejects before any transport involvement; this function never authorizes
/// mutation and derives no host-native ranges.
///
/// # Errors
///
/// Returns a [`HandoffRequestError`] for every non-passing gate outcome.
pub fn build_host_request(
    processor: &crate::DiagnosticDecisionProcessor,
    submitted: &crate::RestorePlanHandoff,
) -> Result<HandoffRequest, HandoffRequestError> {
    use crate::InternalExecutionGate::Rejected;
    use crate::InternalGateRejection::{
        GenerationMismatch as GateGenerationMismatch, HandoffMalformed, HandoffStale,
        InternalSpanInconsistent, NoCurrentHandoff, NoCurrentPlan as GateNoCurrentPlan,
        PlanIneligible, SimulationInvariantBroken,
    };
    if processor.current_restore_handoff().is_none() {
        return Err(HandoffRequestError::NoCurrentPlan);
    }
    if let Rejected(reason) = processor.evaluate_internal_execution_gate(submitted) {
        return Err(match reason {
            GateNoCurrentPlan | PlanIneligible => HandoffRequestError::NoCurrentPlan,
            NoCurrentHandoff | HandoffStale => HandoffRequestError::StaleHandoff,
            GateGenerationMismatch => HandoffRequestError::GenerationMismatch,
            HandoffMalformed | InternalSpanInconsistent => HandoffRequestError::MalformedSpan,
            SimulationInvariantBroken => HandoffRequestError::InternalGateFailed,
        });
    }
    if submitted.rendered_token.is_empty()
        || submitted.replacement_token.is_empty()
        || submitted.rendered_units_to_replace != submitted.rendered_token.chars().count()
        || submitted.replacement_units != submitted.replacement_token.chars().count()
    {
        return Err(HandoffRequestError::MalformedSpan);
    }
    Ok(HandoffRequest {
        request_id: format!("handoff-{}", submitted.generation),
        rendered_token: submitted.rendered_token.clone(),
        replacement_token: submitted.replacement_token.clone(),
        rendered_units: submitted.rendered_units_to_replace,
        replacement_units: submitted.replacement_units,
        generation: submitted.generation,
    })
}

/// Feeds an ASCII token followed by a space boundary into the processor's
/// decision pipeline. Used by tests and the validation endpoint to produce
/// real plans from the real detection/policy path.
///
/// # Panics
///
/// Panics when `token` contains a non-ASCII-letter character; callers
/// validate tokens beforehand.
pub fn feed_token(
    processor: &mut crate::DiagnosticDecisionProcessor,
    token: &str,
    start_sequence: u64,
) {
    use crate::EventProcessor;
    let mut sequence = start_sequence;
    for character in token.chars() {
        let key = zonkey_types::ObservedKey::letter(character)
            .expect("token characters are ASCII letters");
        processor.process(&zonkey_types::ObservedInputEvent {
            key,
            kind: zonkey_types::KeyEventKind::KeyDown,
            modifiers: zonkey_types::ModifierState::new(),
            injection_origin: zonkey_types::InjectionOrigin::PhysicalOrUnmarked,
            sequence: zonkey_types::EventSequence::new(sequence).expect("sequence is non-zero"),
        });
        sequence += 1;
    }
    processor.process(&zonkey_types::ObservedInputEvent {
        key: zonkey_types::ObservedKey::space(),
        kind: zonkey_types::KeyEventKind::KeyDown,
        modifiers: zonkey_types::ModifierState::new(),
        injection_origin: zonkey_types::InjectionOrigin::PhysicalOrUnmarked,
        sequence: zonkey_types::EventSequence::new(sequence).expect("sequence is non-zero"),
    });
}

/// Deterministic verdict of a reconciliation readback (M3D-28 / ADR 0030).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryVerdict {
    /// The intended replacement is present exactly.
    AppliedAcknowledged,
    /// The original rendered text is still present exactly.
    NotApplied,
    /// Neither matches; a human must review.
    ConflictHumanReview,
}

/// Token material of a blocked target. Fresh operator blocks carry the
/// plaintext needed for exact comparisons and operator output; targets
/// restored from durable state carry only the salted hash (ADR 0035 never
/// persists plaintext document text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryText {
    /// Plaintext token (in-memory, same-user surface only).
    Plain(String),
    /// Salted SHA-256 of the token as persisted by [`crate::recovery_codec`].
    Hashed([u8; 32]),
}

impl RecoveryText {
    /// Exact-match test against live readback text. A hashed entry needs
    /// the file salt it was restored with; without it the match fails
    /// closed.
    #[must_use]
    pub fn matches(&self, salt: Option<&[u8; recovery_codec::SALT_BYTES]>, live: &str) -> bool {
        match self {
            RecoveryText::Plain(text) => text == live,
            RecoveryText::Hashed(hash) => match salt {
                Some(salt) => &recovery_codec::salted_hash(salt, live) == hash,
                None => false,
            },
        }
    }

    /// Operator-facing display form; hashed entries never reveal text.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            RecoveryText::Plain(text) => text.clone(),
            RecoveryText::Hashed(_) => "<hashed>".to_owned(),
        }
    }
}

/// One blocked logical target awaiting reconciliation and acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockedTarget {
    /// Session that recorded the Indeterminate outcome; empty for targets
    /// restored from durable state until the first operator action rebinds
    /// them to the current session.
    pub session_id: String,
    /// Document URI of the logical target.
    pub uri: String,
    /// Original rendered token at the time of the outcome.
    pub expected: RecoveryText,
    /// Intended replacement of the lost outcome.
    pub replacement: RecoveryText,
    /// UTF-16 range of the readback comparison.
    pub range: (usize, usize),
    /// Reconciliation state: none yet, or a verdict plus acknowledgement.
    pub state: Option<(RecoveryVerdict, bool)>,
    /// Service-local plan generation at block time; zero when unavailable.
    pub generation: u64,
}

/// Fail-closed registry errors for the recovery workflow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryError {
    /// No blocked target matches the request.
    UnknownTarget,
    /// The command came from a different session than the blocked target.
    SessionMismatch,
    /// Acknowledgement before any reconciliation readback.
    AckBeforeReconcile,
    /// The registry is at capacity with entries that may not be evicted
    /// (unresolved blocks); new blocks are rejected until the operator
    /// reconciles and acknowledges existing targets (ADR 0035).
    RegistryFull,
}

/// Bounded registry implementing the ADR 0030 recovery lifecycle: an
/// Indeterminate outcome blocks a logical target; only an explicit
/// reconciliation readback followed by an explicit operator acknowledgement
/// unblocks it. Unresolved blocks are never evicted — a full registry
/// rejects new blocks instead. Targets restored from durable state
/// (ADR 0035) carry hashed token material and an empty session id; the
/// first valid operator action rebinds them to the current session.
#[derive(Clone, Debug)]
pub struct RecoveryRegistry {
    capacity: usize,
    order: VecDeque<String>,
    entries: HashMap<String, BlockedTarget>,
    salt: Option<[u8; recovery_codec::SALT_BYTES]>,
}

impl RecoveryRegistry {
    /// Constructs a registry with a validated non-zero capacity.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerCapacityError`] when `capacity` is zero.
    pub fn new(capacity: usize) -> Result<Self, LedgerCapacityError> {
        if capacity == 0 {
            return Err(LedgerCapacityError);
        }
        Ok(Self {
            capacity,
            order: VecDeque::new(),
            entries: HashMap::new(),
            salt: None,
        })
    }

    /// Rebuilds a registry from durable records (M3D-31): restored targets
    /// load as blocked with hashed token material, an empty session id, and
    /// their recorded verdict.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerCapacityError`] when `capacity` is zero or smaller
    /// than the record count.
    pub fn restore(
        capacity: usize,
        salt: [u8; recovery_codec::SALT_BYTES],
        records: Vec<recovery_codec::PersistedTarget>,
    ) -> Result<Self, LedgerCapacityError> {
        if capacity == 0 || records.len() > capacity {
            return Err(LedgerCapacityError);
        }
        let mut registry = Self::new(capacity)?;
        registry.salt = Some(salt);
        for record in records {
            let verdict = match record.kind {
                recovery_codec::PersistedKind::Blocked { verdict } => verdict,
                // A pending preflight intent that survived a restart was
                // never definitively resolved: it reloads as a blocked
                // target (recovery-required), never as clean state.
                recovery_codec::PersistedKind::Pending => None,
            };
            let key = Self::key(&record.uri, &hex(&record.expected_hash));
            registry.order.push_back(key.clone());
            registry.entries.insert(
                key,
                BlockedTarget {
                    session_id: String::new(),
                    uri: record.uri,
                    expected: RecoveryText::Hashed(record.expected_hash),
                    replacement: RecoveryText::Hashed(record.replacement_hash),
                    range: record.range,
                    state: verdict.map(|verdict| (verdict, false)),
                    generation: record.generation,
                },
            );
        }
        Ok(registry)
    }

    /// The salt restored from durable state, if any; hashed entries fail
    /// closed without it.
    #[must_use]
    pub fn salt(&self) -> Option<[u8; recovery_codec::SALT_BYTES]> {
        self.salt
    }

    /// Configured maximum number of retained targets.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// True when the logical target (URI plus expected token) is blocked;
    /// operator-supplied plaintext resolves hashed restored entries via the
    /// file salt.
    #[must_use]
    pub fn is_blocked(&self, uri: &str, expected: &str) -> bool {
        self.entries.contains_key(&self.resolve_key(uri, expected))
    }

    fn key(uri: &str, expected: &str) -> String {
        format!("{uri}\u{0}{expected}")
    }

    /// Resolves the registry key for operator-supplied plaintext: the
    /// plaintext key when present, otherwise the salted-hash key for a
    /// restored target when the salt is known.
    fn resolve_key(&self, uri: &str, expected: &str) -> String {
        let plain = Self::key(uri, expected);
        if self.entries.contains_key(&plain) {
            return plain;
        }
        match self.salt {
            Some(salt) => Self::key(uri, &hex(&recovery_codec::salted_hash(&salt, expected))),
            None => plain,
        }
    }

    /// Records a blocked target; re-blocking the same target refreshes its
    /// state. Unresolved blocks are never evicted: when the capacity is
    /// reached the new block is rejected fail-closed (the caller surfaces
    /// [`RecoveryError::RegistryFull`] to the operator).
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::RegistryFull`] when the registry is full;
    /// acknowledged entries are removed, so a full registry always consists
    /// of unresolved targets.
    pub fn block(
        &mut self,
        session_id: &str,
        uri: &str,
        expected: &str,
        replacement: &str,
        range: (usize, usize),
    ) -> Result<Option<String>, RecoveryError> {
        let key = self.resolve_key(uri, expected);
        if !self.entries.contains_key(&key) && self.order.len() == self.capacity {
            return Err(RecoveryError::RegistryFull);
        }
        if !self.entries.contains_key(&key) {
            self.order.push_back(key.clone());
        }
        self.entries.insert(
            key,
            BlockedTarget {
                session_id: session_id.to_owned(),
                uri: uri.to_owned(),
                expected: RecoveryText::Plain(expected.to_owned()),
                replacement: RecoveryText::Plain(replacement.to_owned()),
                range,
                state: None,
                generation: 0,
            },
        );
        Ok(None)
    }

    /// Lists blocked targets in insertion order (sanitized view).
    #[must_use]
    pub fn list(&self) -> Vec<BlockedTarget> {
        self.order
            .iter()
            .filter_map(|key| self.entries.get(key).cloned())
            .collect()
    }

    /// Runs the reconciliation readback against live range text supplied by
    /// the host snapshot; idempotent (a second call returns the recorded
    /// verdict without re-evaluating). A target restored from durable state
    /// has an empty session id; the first valid operator action rebinds it
    /// to the calling session. Hashed (restored) entries compare by salted
    /// hash and fail closed without the file salt.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::UnknownTarget`] or
    /// [`RecoveryError::SessionMismatch`].
    pub fn reconcile(
        &mut self,
        session_id: &str,
        uri: &str,
        expected: &str,
        live_range_text: &str,
    ) -> Result<RecoveryVerdict, RecoveryError> {
        let key = self.resolve_key(uri, expected);
        let salt = self.salt;
        let target = self
            .entries
            .get_mut(&key)
            .ok_or(RecoveryError::UnknownTarget)?;
        if target.session_id.is_empty() {
            session_id.clone_into(&mut target.session_id);
        } else if target.session_id != session_id {
            return Err(RecoveryError::SessionMismatch);
        }
        if let Some((verdict, _)) = target.state {
            return Ok(verdict);
        }
        let verdict = if target.replacement.matches(salt.as_ref(), live_range_text) {
            RecoveryVerdict::AppliedAcknowledged
        } else if target.expected.matches(salt.as_ref(), live_range_text) {
            RecoveryVerdict::NotApplied
        } else {
            RecoveryVerdict::ConflictHumanReview
        };
        target.state = Some((verdict, false));
        Ok(verdict)
    }

    /// Explicit operator acknowledgement; only valid after reconciliation.
    /// A successful acknowledgement removes the block. A target restored
    /// from durable state rebinds to the calling session first.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError`] for unknown targets, session mismatches,
    /// or acknowledgements before reconciliation.
    pub fn acknowledge(
        &mut self,
        session_id: &str,
        uri: &str,
        expected: &str,
    ) -> Result<(), RecoveryError> {
        let key = self.resolve_key(uri, expected);
        let target = self
            .entries
            .get_mut(&key)
            .ok_or(RecoveryError::UnknownTarget)?;
        if target.session_id.is_empty() {
            session_id.clone_into(&mut target.session_id);
        } else if target.session_id != session_id {
            return Err(RecoveryError::SessionMismatch);
        }
        let Some((_verdict, _)) = target.state else {
            return Err(RecoveryError::AckBeforeReconcile);
        };
        self.entries.remove(&key);
        self.order.retain(|entry| entry != &key);
        Ok(())
    }
}

/// Lowercase hex encoding used for hash-keyed restored targets.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Shared live decision state for the M3D-23 live observer wiring.
///
/// The real observer path (`WH_KEYBOARD_LL` → `ObserveService` →
/// `DiagnosticDecisionProcessor`) forwards events here while a pipe endpoint
/// reads the current validated handoff from another thread. All decision
/// semantics stay owned by [`crate::DiagnosticDecisionProcessor`]; this type
/// only shares it and maps the current handoff into a [`HandoffRequest`].
pub struct SharedDecisionState {
    processor: std::sync::Mutex<crate::DiagnosticDecisionProcessor>,
}

impl Default for SharedDecisionState {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedDecisionState {
    /// Creates an empty shared decision state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            processor: std::sync::Mutex::new(crate::DiagnosticDecisionProcessor::default()),
        }
    }

    /// Processes one observed event under the shared lock.
    pub fn process(
        &self,
        event: &zonkey_types::ObservedInputEvent,
    ) -> crate::ProcessorClassification {
        self.processor
            .lock()
            .map_or(crate::ProcessorClassification::Invalid, |mut processor| {
                processor.process(event)
            })
    }

    /// Resets the token lifecycle after a queue discontinuity.
    pub fn reset_after_discontinuity(&self) {
        if let Ok(mut processor) = self.processor.lock() {
            processor.reset_after_discontinuity();
        }
    }

    /// Maps the *current* handoff (never a stale snapshot) into a host
    /// request; `Keep`, `Ambiguous`, injected input, and discontinuity paths
    /// naturally produce no handoff and reject here.
    ///
    /// # Errors
    ///
    /// Returns a [`HandoffRequestError`] when no current eligible handoff
    /// exists or the internal gate refuses it.
    pub fn current_handoff_request(&self) -> Result<HandoffRequest, HandoffRequestError> {
        let processor = self
            .processor
            .lock()
            .map_err(|_| HandoffRequestError::InternalGateFailed)?;
        let handoff = processor
            .current_restore_handoff()
            .ok_or(HandoffRequestError::NoCurrentPlan)?;
        build_host_request(&processor, &handoff)
    }
}

/// `EventProcessor` view over [`SharedDecisionState`] for the observer loop.
#[derive(Clone)]
pub struct SharedDecisionProcessor {
    state: std::sync::Arc<SharedDecisionState>,
}

impl SharedDecisionProcessor {
    /// Wraps shared state for the observer service loop.
    #[must_use]
    pub fn new(state: std::sync::Arc<SharedDecisionState>) -> Self {
        Self { state }
    }
}

impl crate::EventProcessor for SharedDecisionProcessor {
    fn reset_after_discontinuity(&mut self) {
        self.state.reset_after_discontinuity();
    }

    fn process(
        &mut self,
        event: &zonkey_types::ObservedInputEvent,
    ) -> crate::ProcessorClassification {
        self.state.process(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn established() -> HostTransportEndpoint {
        let mut endpoint = HostTransportEndpoint::new(4).expect("non-zero capacity");
        endpoint
            .accept_hello(TRANSPORT_PROTOCOL_ID, "session-1")
            .expect("valid hello");
        endpoint
    }

    #[test]
    fn duplicate_exact_request_replays_recorded_outcome() {
        let mut endpoint = established();
        assert_eq!(
            endpoint.classify_request("req-1", "{a:1}"),
            RequestDisposition::Fresh
        );
        endpoint.record("req-1", "{a:1}", LedgerOutcome::Definite("applied".into()));
        assert_eq!(
            endpoint.classify_request("req-1", "{a:1}"),
            RequestDisposition::Duplicate(LedgerOutcome::Definite("applied".into()))
        );
    }

    #[test]
    fn conflicting_request_id_reuse_reports_conflict() {
        let mut endpoint = established();
        endpoint.record("req-1", "{a:1}", LedgerOutcome::Definite("applied".into()));
        assert_eq!(
            endpoint.classify_request("req-1", "{a:2}"),
            RequestDisposition::Conflict
        );
        assert_eq!(
            endpoint.record("req-1", "{a:2}", LedgerOutcome::Definite("other".into())),
            RecordOutcome::AlreadyPresent
        );
    }

    #[test]
    fn fifo_eviction_is_deterministic() {
        let mut ledger = BoundedRequestLedger::new(2).expect("non-zero capacity");
        assert_eq!(
            ledger.record("a", "ca", LedgerOutcome::Definite("1".into())),
            RecordOutcome::Inserted
        );
        assert_eq!(
            ledger.record("b", "cb", LedgerOutcome::Definite("2".into())),
            RecordOutcome::Inserted
        );
        assert_eq!(
            ledger.record("c", "cc", LedgerOutcome::Definite("3".into())),
            RecordOutcome::Evicted {
                evicted: "a".to_owned()
            }
        );
        assert_eq!(ledger.classify("a", "ca"), RequestDisposition::Fresh);
        assert_eq!(
            ledger.classify("b", "cb"),
            RequestDisposition::Duplicate(LedgerOutcome::Definite("2".into()))
        );
        assert_eq!(
            ledger.classify("c", "cc"),
            RequestDisposition::Duplicate(LedgerOutcome::Definite("3".into()))
        );
        assert_eq!(ledger.len(), 2);
    }

    #[test]
    fn session_restart_invalidates_history() {
        let mut endpoint = established();
        endpoint.record("req-1", "{a:1}", LedgerOutcome::Definite("applied".into()));
        let restarted = endpoint
            .accept_hello(TRANSPORT_PROTOCOL_ID, "session-2")
            .expect("valid restart hello");
        assert!(restarted);
        assert!(endpoint.ledger().is_empty());
        assert_eq!(
            endpoint.classify_request("req-1", "{a:1}"),
            RequestDisposition::Fresh
        );
        assert_eq!(
            endpoint.check_session("session-1"),
            Err(TransportError::SessionMismatch)
        );
        assert_eq!(endpoint.check_session("session-2"), Ok(()));
    }

    #[test]
    fn same_session_hello_is_not_a_restart() {
        let mut endpoint = established();
        endpoint.record("req-1", "{a:1}", LedgerOutcome::Definite("applied".into()));
        let restarted = endpoint
            .accept_hello(TRANSPORT_PROTOCOL_ID, "session-1")
            .expect("valid hello");
        assert!(!restarted);
        assert_eq!(endpoint.ledger().len(), 1);
    }

    #[test]
    fn protocol_mismatch_rejects_hello_without_state_change() {
        let mut endpoint = HostTransportEndpoint::new(4).expect("non-zero capacity");
        assert_eq!(
            endpoint.accept_hello("zonkey.host-transport/0", "session-1"),
            Err(TransportError::ProtocolMismatch)
        );
        assert_eq!(endpoint.session_id(), None);
        assert_eq!(
            endpoint.check_session("session-1"),
            Err(TransportError::NotEstablished)
        );
    }

    #[test]
    fn invalid_hello_identifiers_fail_closed() {
        let mut endpoint = HostTransportEndpoint::new(4).expect("non-zero capacity");
        assert_eq!(
            endpoint.accept_hello("", "session-1"),
            Err(TransportError::InvalidHello)
        );
        assert_eq!(
            endpoint.accept_hello(TRANSPORT_PROTOCOL_ID, ""),
            Err(TransportError::InvalidHello)
        );
    }

    #[test]
    fn session_mismatch_and_unestablished_requests_reject() {
        let unestablished = HostTransportEndpoint::new(4).expect("non-zero capacity");
        assert_eq!(
            unestablished.check_session("session-1"),
            Err(TransportError::NotEstablished)
        );
        let endpoint = established();
        assert_eq!(
            endpoint.check_session("other"),
            Err(TransportError::SessionMismatch)
        );
    }

    #[test]
    fn connection_loss_maps_to_ambiguous_and_never_retries() {
        let mut endpoint = established();
        let outcome = ambiguous_loss_outcome();
        endpoint.record("req-1", "{a:1}", outcome.clone());
        assert_eq!(
            endpoint.classify_request("req-1", "{a:1}"),
            RequestDisposition::Duplicate(outcome)
        );
    }

    #[test]
    fn ambiguous_outcome_keeps_conflict_semantics() {
        let mut endpoint = established();
        endpoint.record("req-1", "{a:1}", ambiguous_loss_outcome());
        assert_eq!(
            endpoint.classify_request("req-1", "{a:2}"),
            RequestDisposition::Conflict
        );
    }

    #[test]
    fn composition_gate_fails_closed_for_every_state() {
        assert_eq!(
            composition_gate_outcome("Unknown"),
            LedgerOutcome::Definite("rejected:CompositionUnknown".into())
        );
        assert_eq!(
            composition_gate_outcome("Active"),
            LedgerOutcome::Definite("rejected:CompositionActive".into())
        );
        // Even proven inactivity cannot authorize execution in this milestone.
        assert_eq!(
            composition_gate_outcome("Inactive"),
            LedgerOutcome::Definite("rejected:ExecutionNotImplemented".into())
        );
        assert_eq!(
            composition_gate_outcome("anything-else"),
            LedgerOutcome::Definite("rejected:CompositionUnknown".into())
        );
    }

    fn processor_with(token: &str) -> crate::DiagnosticDecisionProcessor {
        let mut processor = crate::DiagnosticDecisionProcessor::default();
        feed_token(&mut processor, token, 1);
        processor
    }

    #[test]
    fn valid_restore_candidate_creates_request_with_deterministic_id() {
        let processor = processor_with("resume");
        let handoff = processor
            .current_restore_handoff()
            .expect("resume produces a handoff");
        let request = build_host_request(&processor, &handoff).expect("valid handoff maps");
        assert_eq!(
            request.request_id,
            format!("handoff-{}", handoff.generation)
        );
        assert_eq!(request.rendered_token, handoff.rendered_token);
        assert_eq!(request.replacement_token, handoff.replacement_token);
        assert_eq!(
            request.rendered_units,
            request.rendered_token.chars().count()
        );
        assert_eq!(
            request.replacement_units,
            request.replacement_token.chars().count()
        );
        assert_eq!(request.generation, handoff.generation);
        // No host-native range or caret field exists on the request.
        let again = build_host_request(&processor, &handoff).expect("rebuild maps");
        assert_eq!(again.request_id, request.request_id);
    }

    #[test]
    fn keep_and_ambiguous_create_no_request() {
        for token in ["dungf", "hello"] {
            let processor = processor_with(token);
            let submitted = crate::RestorePlanHandoff {
                rendered_token: token.to_owned(),
                replacement_token: token.to_owned(),
                rendered_units_to_replace: token.chars().count(),
                replacement_units: token.chars().count(),
                reason: zonkey_types::DecisionReason::ExactEnglishDictionary,
                generation: 1,
                simulation_only: true,
            };
            assert_eq!(
                build_host_request(&processor, &submitted),
                Err(HandoffRequestError::NoCurrentPlan),
                "token {token} must not produce a request"
            );
        }
    }

    #[test]
    fn stale_handoff_after_replacement_rejects() {
        let mut processor = processor_with("resume");
        let stale = processor
            .current_restore_handoff()
            .expect("initial handoff");
        feed_token(&mut processor, "config", 10);
        assert_eq!(
            build_host_request(&processor, &stale),
            Err(HandoffRequestError::GenerationMismatch)
        );
    }

    #[test]
    fn generation_mismatch_rejects() {
        let processor = processor_with("resume");
        let mut submitted = processor
            .current_restore_handoff()
            .expect("current handoff");
        submitted.generation += 1;
        assert_eq!(
            build_host_request(&processor, &submitted),
            Err(HandoffRequestError::GenerationMismatch)
        );
    }

    #[test]
    fn malformed_span_rejects() {
        let processor = processor_with("resume");
        let mut submitted = processor
            .current_restore_handoff()
            .expect("current handoff");
        submitted.rendered_units_to_replace += 1;
        assert_eq!(
            build_host_request(&processor, &submitted),
            Err(HandoffRequestError::MalformedSpan)
        );
    }

    #[test]
    fn simulation_invariant_failure_maps_to_gate_failure() {
        let processor = processor_with("resume");
        let genuine = processor
            .current_restore_handoff()
            .expect("current handoff");
        let broken = crate::RestorePlanHandoff {
            rendered_token: genuine.rendered_token.clone(),
            replacement_token: genuine.replacement_token.clone(),
            rendered_units_to_replace: genuine.rendered_units_to_replace,
            replacement_units: genuine.replacement_units,
            reason: genuine.reason.clone(),
            generation: genuine.generation,
            simulation_only: false,
        };
        assert_eq!(
            build_host_request(&processor, &broken),
            Err(HandoffRequestError::InternalGateFailed)
        );
    }

    #[test]
    fn request_pipeline_executes_exactly_once_per_id() {
        let mut endpoint = established();
        let mut executions = 0;
        let mut send = |endpoint: &mut HostTransportEndpoint, request_id: &str| {
            let disposition = endpoint.classify_request(request_id, "{canonical}");
            let outcome = match disposition {
                RequestDisposition::Duplicate(recorded) => recorded,
                RequestDisposition::Conflict => LedgerOutcome::Definite("request_id_reuse".into()),
                RequestDisposition::Fresh => {
                    executions += 1;
                    LedgerOutcome::Definite("applied".into())
                }
            };
            endpoint.record(request_id, "{canonical}", outcome.clone());
            outcome
        };
        assert_eq!(
            send(&mut endpoint, "req-1"),
            LedgerOutcome::Definite("applied".into())
        );
        assert_eq!(
            send(&mut endpoint, "req-1"),
            LedgerOutcome::Definite("applied".into())
        );
        assert_eq!(executions, 1);
    }

    #[test]
    fn malformed_frames_fail_closed() {
        assert_eq!(encode_frame(""), Err(FrameError::EmptyFrame));
        assert_eq!(
            decode_frame(&mut &[0, 0, 0, 0][..]),
            Err(FrameError::EmptyFrame)
        );
        let mut invalid_utf8 = 2u32.to_le_bytes().to_vec();
        invalid_utf8.extend_from_slice(&[0xd8, 0x00]);
        assert_eq!(
            decode_frame(&mut invalid_utf8.as_slice()),
            Err(FrameError::InvalidUtf8)
        );
    }

    #[test]
    fn oversized_frames_fail_closed() {
        let oversized = "x".repeat(MAX_FRAME_BYTES + 1);
        assert_eq!(encode_frame(&oversized), Err(FrameError::OversizedFrame));
        let mut header = u32::try_from(MAX_FRAME_BYTES + 1)
            .expect("fits u32")
            .to_le_bytes()
            .to_vec();
        header.extend_from_slice(&[0x78]);
        assert_eq!(
            decode_frame(&mut header.as_slice()),
            Err(FrameError::OversizedFrame)
        );
    }

    #[test]
    fn frames_roundtrip_and_stream_incrementally() {
        let frame = encode_frame("{\"hello\":\"zonkey\"}").expect("valid payload");
        let mut whole = frame.as_slice();
        assert_eq!(
            decode_frame(&mut whole).map(|decoded| decoded.map(|f| f.payload)),
            Ok(Some("{\"hello\":\"zonkey\"}".to_owned()))
        );
        assert_eq!(decode_frame(&mut whole), Ok(None));
        let mut partial = &frame[..frame.len() - 1];
        assert_eq!(decode_frame(&mut partial), Ok(None));
    }

    #[test]
    fn zero_ledger_capacity_is_rejected() {
        assert!(matches!(
            BoundedRequestLedger::new(0),
            Err(LedgerCapacityError)
        ));
        assert!(matches!(
            HostTransportEndpoint::new(0),
            Err(LedgerCapacityError)
        ));
    }

    fn live_event(
        sequence: u64,
        key: zonkey_types::ObservedKey,
        injection_origin: zonkey_types::InjectionOrigin,
    ) -> zonkey_types::ObservedInputEvent {
        zonkey_types::ObservedInputEvent {
            key,
            kind: zonkey_types::KeyEventKind::KeyDown,
            modifiers: zonkey_types::ModifierState::new(),
            injection_origin,
            sequence: zonkey_types::EventSequence::new(sequence).expect("sequence is non-zero"),
        }
    }

    fn live_letter(state: &SharedDecisionState, sequence: u64, character: char) {
        let key = zonkey_types::ObservedKey::letter(character).expect("ASCII letter");
        state.process(&live_event(
            sequence,
            key,
            zonkey_types::InjectionOrigin::PhysicalOrUnmarked,
        ));
    }

    fn live_space(state: &SharedDecisionState, sequence: u64) {
        state.process(&live_event(
            sequence,
            zonkey_types::ObservedKey::space(),
            zonkey_types::InjectionOrigin::PhysicalOrUnmarked,
        ));
    }

    #[test]
    fn live_wiring_maps_current_restore_candidate() {
        let state = std::sync::Arc::new(SharedDecisionState::new());
        let mut sequence = 1;
        for character in "resume".chars() {
            live_letter(&state, sequence, character);
            sequence += 1;
        }
        // Negative first: before the boundary there is no current handoff.
        assert_eq!(
            state.current_handoff_request(),
            Err(HandoffRequestError::NoCurrentPlan)
        );
        live_space(&state, sequence);
        let request = state.current_handoff_request().expect("live handoff");
        assert_eq!(request.request_id, "handoff-1");
        assert!(!request.rendered_token.is_empty());
        assert!(!request.replacement_token.is_empty());
        assert_eq!(
            request.rendered_units,
            request.rendered_token.chars().count()
        );
    }

    #[test]
    fn live_writing_second_candidate_advances_generation_deterministically() {
        let state = std::sync::Arc::new(SharedDecisionState::new());
        feed_live_token(&state, "resume", 1);
        let first = state.current_handoff_request().expect("first handoff");
        feed_live_token(&state, "config", 10);
        let second = state.current_handoff_request().expect("second handoff");
        assert_eq!(first.request_id, "handoff-1");
        assert_eq!(second.request_id, "handoff-2");
    }

    #[test]
    fn live_keep_and_ambiguous_produce_no_request() {
        for token in ["dungf", "hello"] {
            let state = std::sync::Arc::new(SharedDecisionState::new());
            feed_live_token(&state, token, 1);
            assert_eq!(
                state.current_handoff_request(),
                Err(HandoffRequestError::NoCurrentPlan),
                "token {token} must not produce a live request"
            );
        }
    }

    #[test]
    fn live_injected_events_do_not_produce_request() {
        let state = std::sync::Arc::new(SharedDecisionState::new());
        let mut sequence = 1;
        for character in "resume".chars() {
            let key = zonkey_types::ObservedKey::letter(character).expect("ASCII letter");
            state.process(&live_event(
                sequence,
                key,
                zonkey_types::InjectionOrigin::MarkedInjected,
            ));
            sequence += 1;
        }
        live_space(&state, sequence);
        assert_eq!(
            state.current_handoff_request(),
            Err(HandoffRequestError::NoCurrentPlan)
        );
    }

    #[test]
    fn live_shortcut_modifier_keys_do_not_mutate_token() {
        let state = std::sync::Arc::new(SharedDecisionState::new());
        live_letter(&state, 1, 'r');
        // A Ctrl+r shortcut event must be ignored by the decision processor.
        state.process(&zonkey_types::ObservedInputEvent {
            key: zonkey_types::ObservedKey::letter('r').expect("ASCII letter"),
            kind: zonkey_types::KeyEventKind::KeyDown,
            modifiers: zonkey_types::ModifierState::new().with_control(true),
            injection_origin: zonkey_types::InjectionOrigin::PhysicalOrUnmarked,
            sequence: zonkey_types::EventSequence::new(2).expect("sequence is non-zero"),
        });
        for (sequence, character) in (3..).zip("esume".chars()) {
            live_letter(&state, sequence, character);
        }
        live_space(&state, 8);
        // The Ctrl-modified event did not append to the token; the typed
        // token "resume" still yields the first generation.
        let request = state.current_handoff_request().expect("shortcut isolation");
        assert_eq!(request.request_id, "handoff-1");
    }

    #[test]
    fn live_discontinuity_resets_lifecycle() {
        let state = std::sync::Arc::new(SharedDecisionState::new());
        feed_live_token(&state, "resume", 1);
        assert!(state.current_handoff_request().is_ok());
        state.reset_after_discontinuity();
        assert_eq!(
            state.current_handoff_request(),
            Err(HandoffRequestError::NoCurrentPlan)
        );
    }

    fn feed_live_token(state: &std::sync::Arc<SharedDecisionState>, token: &str, start: u64) {
        let mut sequence = start;
        for character in token.chars() {
            live_letter(state, sequence, character);
            sequence += 1;
        }
        live_space(state, sequence);
    }

    #[test]
    fn recovery_registry_full_lifecycle() {
        let mut registry = RecoveryRegistry::new(4).expect("non-zero capacity");
        assert!(registry.list().is_empty());
        assert_eq!(
            registry.block("sess-1", "file:///a", "resume", "restored", (0, 6)),
            Ok(None)
        );
        assert_eq!(registry.list().len(), 1);
        // Ack before reconciliation is rejected.
        assert_eq!(
            registry.acknowledge("sess-1", "file:///a", "resume"),
            Err(RecoveryError::AckBeforeReconcile)
        );
        // All three deterministic verdicts.
        assert_eq!(
            registry.reconcile("sess-1", "file:///a", "resume", "restored"),
            Ok(RecoveryVerdict::AppliedAcknowledged)
        );
        // Duplicate reconciliation is idempotent.
        assert_eq!(
            registry.reconcile("sess-1", "file:///a", "resume", "restored"),
            Ok(RecoveryVerdict::AppliedAcknowledged)
        );
        assert_eq!(
            registry.acknowledge("sess-1", "file:///a", "resume"),
            Ok(())
        );
        assert!(registry.list().is_empty());
        assert_eq!(
            registry.acknowledge("sess-1", "file:///a", "resume"),
            Err(RecoveryError::UnknownTarget)
        );
    }

    #[test]
    fn recovery_registry_not_applied_and_conflict_verdicts() {
        let mut registry = RecoveryRegistry::new(4).expect("non-zero capacity");
        assert_eq!(
            registry.block("sess-1", "file:///a", "resume", "restored", (0, 6)),
            Ok(None)
        );
        assert_eq!(
            registry.reconcile("sess-1", "file:///a", "resume", "resume"),
            Ok(RecoveryVerdict::NotApplied)
        );
        assert_eq!(
            registry.acknowledge("sess-1", "file:///a", "resume"),
            Ok(())
        );
        assert_eq!(
            registry.block("sess-1", "file:///b", "resume", "restored", (0, 6)),
            Ok(None)
        );
        assert_eq!(
            registry.reconcile("sess-1", "file:///b", "resume", "mangled!"),
            Ok(RecoveryVerdict::ConflictHumanReview)
        );
        // Conflict still requires an explicit acknowledgement to unblock.
        assert_eq!(registry.list().len(), 1);
        assert_eq!(
            registry.acknowledge("sess-1", "file:///b", "resume"),
            Ok(())
        );
        assert!(registry.list().is_empty());
    }

    #[test]
    fn recovery_registry_rejects_wrong_session_and_unknown_targets() {
        let mut registry = RecoveryRegistry::new(4).expect("non-zero capacity");
        assert_eq!(
            registry.block("sess-1", "file:///a", "resume", "restored", (0, 6)),
            Ok(None)
        );
        assert_eq!(
            registry.reconcile("sess-2", "file:///a", "resume", "restored"),
            Err(RecoveryError::SessionMismatch)
        );
        assert_eq!(
            registry.acknowledge("sess-2", "file:///a", "resume"),
            Err(RecoveryError::SessionMismatch)
        );
        assert_eq!(
            registry.reconcile("sess-1", "file:///missing", "resume", "x"),
            Err(RecoveryError::UnknownTarget)
        );
    }

    #[test]
    fn recovery_registry_full_of_unresolved_rejects_new_blocks() {
        let mut registry = RecoveryRegistry::new(2).expect("non-zero capacity");
        assert_eq!(
            registry.block("sess-1", "file:///a", "t1", "r1", (0, 2)),
            Ok(None)
        );
        assert_eq!(
            registry.block("sess-1", "file:///b", "t2", "r2", (0, 2)),
            Ok(None)
        );
        // Unresolved blocks are never evicted: a full registry rejects.
        assert_eq!(
            registry.block("sess-1", "file:///c", "t3", "r3", (0, 2)),
            Err(RecoveryError::RegistryFull)
        );
        assert_eq!(registry.list().len(), 2);
        // Resolving and acknowledging one target frees capacity.
        assert_eq!(
            registry.reconcile("sess-1", "file:///a", "t1", "r1"),
            Ok(RecoveryVerdict::AppliedAcknowledged)
        );
        assert_eq!(registry.acknowledge("sess-1", "file:///a", "t1"), Ok(()));
        assert_eq!(
            registry.block("sess-1", "file:///c", "t3", "r3", (0, 2)),
            Ok(None)
        );
    }

    #[test]
    fn restored_targets_rebind_reconcile_by_hash_and_ack() {
        let salt = [0xABu8; recovery_codec::SALT_BYTES];
        let record = |verdict| recovery_codec::PersistedTarget {
            uri: "file:///a".to_owned(),
            range: (0, 6),
            expected_hash: recovery_codec::salted_hash(&salt, "resume"),
            replacement_hash: recovery_codec::salted_hash(&salt, "restored"),
            kind: recovery_codec::PersistedKind::Blocked { verdict },
            generation: 1,
            request_id: "operator".to_owned(),
        };
        for (verdict, live, expected) in [
            (None, "restored", RecoveryVerdict::AppliedAcknowledged),
            (None, "resume", RecoveryVerdict::NotApplied),
            (None, "mangled!", RecoveryVerdict::ConflictHumanReview),
        ] {
            let mut registry =
                RecoveryRegistry::restore(4, salt, vec![record(verdict)]).expect("restore");
            let listed = registry.list();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].session_id, "");
            assert_eq!(listed[0].expected.display(), "<hashed>");
            // The first valid operator action rebinds to the new session.
            assert_eq!(
                registry.reconcile("sess-new", "file:///a", "resume", live),
                Ok(expected)
            );
            assert_eq!(registry.list()[0].session_id, "sess-new");
            // After the rebind, other sessions are rejected again.
            assert_eq!(
                registry.reconcile("sess-other", "file:///a", "resume", live),
                Err(RecoveryError::SessionMismatch)
            );
            assert_eq!(
                registry.acknowledge("sess-new", "file:///a", "resume"),
                Ok(())
            );
            assert!(registry.list().is_empty());
        }
    }

    #[test]
    fn restored_verdict_survives_and_ack_rebinds_without_reconcile() {
        let salt = [1u8; recovery_codec::SALT_BYTES];
        let records = vec![recovery_codec::PersistedTarget {
            uri: "file:///a".to_owned(),
            range: (0, 6),
            expected_hash: recovery_codec::salted_hash(&salt, "resume"),
            replacement_hash: recovery_codec::salted_hash(&salt, "restored"),
            kind: recovery_codec::PersistedKind::Blocked {
                verdict: Some(RecoveryVerdict::ConflictHumanReview),
            },
            generation: 2,
            request_id: "operator".to_owned(),
        }];
        let mut registry = RecoveryRegistry::restore(4, salt, records).expect("restore");
        // A recorded verdict is replayed without re-evaluating, for any live
        // text, and the target acks straight away after rebind.
        assert_eq!(
            registry.reconcile("sess-new", "file:///a", "resume", "whatever"),
            Ok(RecoveryVerdict::ConflictHumanReview)
        );
        assert_eq!(
            registry.acknowledge("sess-new", "file:///a", "resume"),
            Ok(())
        );
        assert!(registry.list().is_empty());
    }

    #[test]
    fn hashed_text_without_salt_fails_closed() {
        let hashed = RecoveryText::Hashed([9u8; 32]);
        assert!(!hashed.matches(None, "anything"));
        let plain = RecoveryText::Plain("resume".to_owned());
        assert!(plain.matches(None, "resume"));
        assert!(!plain.matches(None, "other"));
    }

    #[test]
    fn ambiguous_ledger_entries_are_pinned_and_saturation_fails_closed() {
        let mut ledger = BoundedRequestLedger::new(2).expect("non-zero capacity");
        assert_eq!(
            ledger.record("amb-1", "c1", ambiguous_loss_outcome()),
            RecordOutcome::Inserted
        );
        assert_eq!(
            ledger.record("def-1", "c2", LedgerOutcome::Definite("applied".into())),
            RecordOutcome::Inserted
        );
        // Eviction skips the pinned ambiguous head and takes the definite.
        assert_eq!(
            ledger.record("def-2", "c3", LedgerOutcome::Definite("applied".into())),
            RecordOutcome::Evicted {
                evicted: "def-1".to_owned()
            }
        );
        assert_eq!(
            ledger.classify("amb-1", "c1"),
            RequestDisposition::Duplicate(ambiguous_loss_outcome())
        );
        // An all-ambiguous full ledger refuses to record: never forget.
        assert_eq!(
            ledger.record("amb-2", "c4", ambiguous_loss_outcome()),
            RecordOutcome::Evicted {
                evicted: "def-2".to_owned()
            }
        );
        assert_eq!(ledger.len(), 2);
        assert_eq!(
            ledger.record("amb-3", "c5", ambiguous_loss_outcome()),
            RecordOutcome::RejectedLedgerFull
        );
        assert_eq!(ledger.len(), 2);
        assert_eq!(
            ledger.classify("amb-1", "c1"),
            RequestDisposition::Duplicate(ambiguous_loss_outcome())
        );
        assert_eq!(ledger.classify("amb-3", "c5"), RequestDisposition::Fresh);
    }

    #[test]
    fn recovery_backed_ambiguous_entries_evict_while_unbacked_stay_pinned() {
        let mut ledger = BoundedRequestLedger::new(2).expect("non-zero capacity");
        // Backed ambiguous: the durable recovery record exists, so the
        // ledger may forget the result and stay bounded.
        assert_eq!(
            ledger.record_with_recovery("back-1", "c1", ambiguous_loss_outcome()),
            RecordOutcome::Inserted
        );
        assert_eq!(
            ledger.record_with_recovery("back-2", "c2", ambiguous_loss_outcome()),
            RecordOutcome::Inserted
        );
        // Both entries are ambiguous but recovery-backed: eviction proceeds.
        assert_eq!(
            ledger.record("def-1", "c3", LedgerOutcome::Definite("applied".into())),
            RecordOutcome::Evicted {
                evicted: "back-1".to_owned()
            }
        );
        assert_eq!(ledger.len(), 2);
        // Unbacked ambiguous entries at capacity still refuse to record.
        let mut pinned = BoundedRequestLedger::new(1).expect("non-zero capacity");
        assert_eq!(
            pinned.record("free-1", "c1", ambiguous_loss_outcome()),
            RecordOutcome::Inserted
        );
        assert_eq!(
            pinned.record("def-1", "c2", LedgerOutcome::Definite("x".into())),
            RecordOutcome::RejectedLedgerFull
        );
        assert_eq!(
            pinned.record_with_recovery("def-2", "c3", LedgerOutcome::Definite("x".into())),
            RecordOutcome::RejectedLedgerFull
        );
        assert_eq!(pinned.len(), 1);
    }
}

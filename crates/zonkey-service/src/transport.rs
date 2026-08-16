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
    /// Recorded; the oldest inserted id was evicted deterministically.
    Evicted { evicted: String },
    /// The id already existed; nothing changed.
    AlreadyPresent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LedgerEntry {
    canonical: String,
    outcome: LedgerOutcome,
}

/// A bounded request ledger with deterministic FIFO eviction.
///
/// Eviction removes the oldest *inserted* entry; lookups never refresh
/// order. All outcome kinds, including `Ambiguous`, are retained and replayed
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

    /// Records one outcome, evicting the oldest inserted entry when full.
    pub fn record(
        &mut self,
        request_id: &str,
        canonical: &str,
        outcome: LedgerOutcome,
    ) -> RecordOutcome {
        if self.entries.contains_key(request_id) {
            return RecordOutcome::AlreadyPresent;
        }
        let evicted = if self.order.len() == self.capacity {
            self.order.pop_front().inspect(|oldest| {
                self.entries.remove(oldest);
            })
        } else {
            None
        };
        self.order.push_back(request_id.to_owned());
        self.entries.insert(
            request_id.to_owned(),
            LedgerEntry {
                canonical: canonical.to_owned(),
                outcome,
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

    /// Read-only view of the bounded ledger.
    #[must_use]
    pub fn ledger(&self) -> &BoundedRequestLedger {
        &self.ledger
    }
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
}

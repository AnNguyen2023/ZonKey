#![allow(clippy::doc_markdown)]

//! M3D-20/M3D-29 hardened Windows named-pipe transport binding.
//!
//! This module binds the platform-neutral transport contract from
//! `zonkey-service::transport` to a localhost named pipe for Windows 11 x64.
//! One server instance hosts one dummy/test host; one client connects per
//! connection. Frames, the protocol id, the session-bound ledger, and
//! ambiguous-loss semantics all come from the existing contract; this module
//! adds only pipe I/O, a bounded read timeout, and clean teardown.
//!
//! Security posture (M3D-29), stated exactly: every created pipe instance
//! carries an explicit DACL granting the creating process's user SID full
//! access and nothing else — the default process DACL is never used, so
//! other interactive users, `Everyone`, administrators, and `LOCAL SYSTEM`
//! are denied by omission. A Windows administrator can still take ownership
//! of the pipe and rewrite the DACL; that is inherent administrative power
//! and a documented residual threat. `FILE_FLAG_FIRST_PIPE_INSTANCE` makes
//! creation fail if anything squatted the name first. Immediately after
//! connect the server impersonates the client at identification level,
//! compares the client token's user SID with its own, always reverts, and
//! drops the connection on any failure or mismatch — SID identity
//! inspection, not cryptographic authentication, and never claimed as such.
//! PID, window handle, and pipe name alone are never trusted identity.
//! Session ids and [`generate_pipe_name`] names embed a 128-bit
//! `BCryptGenRandom` nonce per server lifecycle, so a stale identity never
//! authorizes a new session. The only binding remains the server-issued
//! session id delivered by the `WELCOME` handshake, a correlation token.
//! Malformed or oversized frames fail closed by dropping the connection. A
//! request whose result is lost to a timeout or disconnect is the caller's
//! `Indeterminate`; re-sending the same request id replays the recorded
//! outcome without re-execution.
//!
//! This spike never mutates editor text and performs no VS Code `Applied`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use zonkey_service::transport::{
    Frame, FrameError, HostTransportEndpoint, LedgerOutcome, RequestDisposition,
    TRANSPORT_PROTOCOL_ID, decode_frame, encode_frame,
};

use windows::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_PIPE_CONNECTED, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_CREATION_DISPOSITION, FILE_FLAG_FIRST_PIPE_INSTANCE,
    FILE_SHARE_MODE, FlushFileBuffers, PIPE_ACCESS_DUPLEX, READ_CONTROL, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::IO::CancelSynchronousIo;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_WAIT, WaitNamedPipeW,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread};
use windows::core::{HRESULT, PCWSTR};

/// Fail-closed pipe transport errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipeError {
    /// The pipe did not appear before the connect deadline.
    ConnectTimeout,
    /// The hardened security boundary could not be established (explicit
    /// DACL, peer verification inputs, or nonce RNG). No listener was
    /// created with weaker security.
    PipeSecurity(String),
    /// The handshake was refused by the server.
    HandshakeRefused(String),
    /// The server reported a protocol id mismatch.
    ProtocolMismatch,
    /// The server reported a session mismatch.
    SessionMismatch,
    /// A bounded read timed out; the outcome is ambiguous.
    Timeout,
    /// The connection was lost; the outcome may be ambiguous.
    ConnectionLost,
    /// A frame violated framing rules and the connection was dropped.
    Frame(FrameError),
    /// A well-formed frame carried an invalid payload.
    InvalidPayload(String),
}

impl From<FrameError> for PipeError {
    fn from(error: FrameError) -> Self {
        PipeError::Frame(error)
    }
}

/// Handler invoked exactly once per fresh request id: `(request_id,
/// composition, canonical)`.
pub type RequestHandler = Arc<dyn Fn(&str, &str, &str) -> LedgerOutcome + Send + Sync>;

/// Query-only validation handler for the M3D-21 endpoint: fails closed on
/// composition and never executes anything.
#[must_use]
pub fn composition_gate_handler() -> RequestHandler {
    Arc::new(|_request_id, composition, _canonical| {
        zonkey_service::transport::composition_gate_outcome(composition)
    })
}

/// Wrapper letting a pipe handle cross into the reader thread; the handle is
/// used from exactly one thread at a time.
struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

const READ_CHUNK: usize = 4096;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// `HRESULT_FROM_WIN32(ERROR_PIPE_BUSY)` where `ERROR_PIPE_BUSY` is 231.
const HRESULT_PIPE_BUSY: i32 = -2_147_024_755;

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_session_id() -> Result<String, PipeError> {
    // Unpredictable per-lifecycle identity: a 128-bit system-RNG nonce so a
    // stale session id from a previous lifecycle authorizes nothing.
    let nonce = crate::pipe_security::random_nonce_hex(16)
        .map_err(|error| PipeError::PipeSecurity(format!("{error:?}")))?;
    Ok(format!(
        "sess-{}-{nonce}",
        SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Generates an unpredictable pipe name for one server lifecycle:
/// `\\.\pipe\zonkey-<prefix>-<128-bit BCrypt nonce hex>`. Each call yields a
/// fresh identity; callers must treat the name as opaque.
///
/// # Errors
///
/// Returns [`PipeError::InvalidPayload`] for an empty prefix or any
/// character outside ASCII alphanumerics and `-`, and
/// [`PipeError::PipeSecurity`] when the system RNG is unavailable.
pub fn generate_pipe_name(prefix: &str) -> Result<String, PipeError> {
    if prefix.is_empty()
        || !prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(PipeError::InvalidPayload(
            "pipe name prefix must be non-empty ASCII alphanumerics or dashes".to_owned(),
        ));
    }
    let nonce = crate::pipe_security::random_nonce_hex(16)
        .map_err(|error| PipeError::PipeSecurity(format!("{error:?}")))?;
    Ok(format!(r"\\.\pipe\zonkey-{prefix}-{nonce}"))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn write_all(handle: HANDLE, bytes: &[u8]) -> Result<(), PipeError> {
    unsafe { WriteFile(handle, Some(bytes), None, None) }.map_err(|_| PipeError::ConnectionLost)?;
    unsafe { FlushFileBuffers(handle) }.map_err(|_| PipeError::ConnectionLost)?;
    Ok(())
}

fn write_frame(handle: HANDLE, payload: &str) -> Result<(), PipeError> {
    let frame = encode_frame(payload)?;
    write_all(handle, &frame)
}

/// Blocking chunked read of exactly one frame; used by the server loop.
fn read_frame_blocking(handle: HANDLE, buffer: &mut Vec<u8>) -> Result<Frame, PipeError> {
    loop {
        let mut view: &[u8] = buffer;
        if let Some(frame) = decode_frame(&mut view)? {
            let consumed = buffer.len() - view.len();
            buffer.drain(..consumed);
            return Ok(frame);
        }
        let mut chunk = [0u8; READ_CHUNK];
        let mut read = 0u32;
        unsafe { ReadFile(handle, Some(&mut chunk), Some(&raw mut read), None) }
            .map_err(|_| PipeError::ConnectionLost)?;
        if read == 0 {
            return Err(PipeError::ConnectionLost);
        }
        buffer.extend_from_slice(&chunk[..read as usize]);
    }
}

/// A running dummy-host pipe server for one pipe name.
pub struct PipeServerHandle {
    pub pipe_name: String,
    pub session_id: String,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    ready: Receiver<()>,
}

impl PipeServerHandle {
    /// Waits until the listener thread has created the pipe instance.
    ///
    /// # Errors
    ///
    /// Returns [`PipeError::ConnectTimeout`] if creation does not signal in
    /// time (for example the thread panicked).
    pub fn wait_ready(&self, timeout: Duration) -> Result<(), PipeError> {
        match self.ready.recv_timeout(timeout) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => Ok(()),
            Err(RecvTimeoutError::Timeout) => Err(PipeError::ConnectTimeout),
        }
    }

    /// Stops the listener and waits for its thread to finish.
    pub fn shutdown(mut self) {
        self.stop_internal();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }

    fn stop_internal(&self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake a blocking ConnectNamedPipe with a throwaway connection.
        if let Ok(handle) = open_raw_handle(&self.pipe_name, Duration::from_millis(500)) {
            let _ = unsafe { CloseHandle(handle) };
        }
    }
}

impl Drop for PipeServerHandle {
    fn drop(&mut self) {
        self.stop_internal();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Spawns a dummy-host named-pipe server using the shared transport contract.
///
/// # Errors
///
/// Returns [`PipeError::ConnectTimeout`] when the listener cannot signal
/// readiness (for example the pipe name is already in use).
///
/// # Panics
///
/// Never panics on the caller's thread; invalid capacity silently stops the
/// listener thread and surfaces as the connect-timeout error.
pub fn spawn_dummy_host_server(
    pipe_name: &str,
    ledger_capacity: usize,
    handler: RequestHandler,
) -> Result<PipeServerHandle, PipeError> {
    spawn_dummy_host_server_with_handoff(pipe_name, ledger_capacity, handler, None, None)
}

/// Read-only provider of the current validated handoff request (M3D-22).
pub type HandoffProvider = Arc<
    dyn Fn() -> Result<zonkey_service::transport::HandoffRequest, HandoffRequestWireError>
        + Send
        + Sync,
>;

/// Wire form of a handoff provider failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffRequestWireError(pub String);

/// Encodes a handoff query answer as a transport result payload. Fields are
/// delimiter-separated and must not contain `|`.
#[must_use]
pub fn handoff_payload(request: &zonkey_service::transport::HandoffRequest) -> String {
    format!(
        "handoff:{}|{}|{}|{}|{}|{}",
        request.request_id,
        request.rendered_token,
        request.replacement_token,
        request.rendered_units,
        request.replacement_units,
        request.generation
    )
}

/// Spawns a dummy-host server that additionally answers read-only `HANDOFF`
/// queries from a [`HandoffProvider`] and operator `RECOVERY` commands from
/// an optional shared durable [`crate::recovery_store::DurableRecoveryStore`].
///
/// # Errors
///
/// Returns [`PipeError::ConnectTimeout`] when the listener cannot start.
///
/// # Panics
///
/// Never panics on the caller's thread; invalid capacity silently stops the
/// listener thread and surfaces as the connect-timeout error.
pub fn spawn_dummy_host_server_with_handoff(
    pipe_name: &str,
    ledger_capacity: usize,
    handler: RequestHandler,
    handoff: Option<HandoffProvider>,
    recovery: Option<Arc<std::sync::Mutex<crate::recovery_store::DurableRecoveryStore>>>,
) -> Result<PipeServerHandle, PipeError> {
    let pipe_name = pipe_name.to_owned();
    let session_id = next_session_id()?;
    // Fail closed before spawning anything if the explicit current-user-only
    // DACL cannot be built: no listener is ever created with weaker security.
    let security = crate::pipe_security::PipeSecurityAttributes::current_user_only()
        .map_err(|error| PipeError::PipeSecurity(format!("{error:?}")))?;
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let thread_stop = Arc::clone(&stop);
    let thread_session = session_id.clone();
    let listener_pipe_name = pipe_name.clone();
    let join = thread::spawn(move || {
        let Ok(mut endpoint) = HostTransportEndpoint::new(ledger_capacity) else {
            return;
        };
        let name = wide(&listener_pipe_name);
        let frame_buffer = u32::try_from(zonkey_service::transport::MAX_FRAME_BYTES + 4)
            .expect("frame bound fits u32");
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                frame_buffer,
                frame_buffer,
                1000,
                Some(security.as_ptr()),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let _ = ready_tx.send(());
            return;
        }
        let _ = ready_tx.send(());
        loop {
            let connected = unsafe { ConnectNamedPipe(handle, None) };
            if let Err(error) = connected {
                // ERROR_PIPE_CONNECTED means a client completed the connect
                // in the window between CreateNamedPipeW and this call; the
                // instance is connected and must be served, not dropped.
                if error.code() != HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) {
                    break;
                }
            }
            // Narrow fail-closed peer trust: the connected client must be the
            // current user (identification-level impersonation, SID compare,
            // always revert). Any failure drops the connection unserved.
            if crate::pipe_security::verify_peer_is_current_user(handle).is_err() {
                let _ = unsafe { DisconnectNamedPipe(handle) };
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
            serve_connection(
                handle,
                &mut endpoint,
                &thread_session,
                &handler,
                handoff.as_ref(),
                recovery.as_ref(),
            );
            let _ = unsafe { DisconnectNamedPipe(handle) };
            if thread_stop.load(Ordering::Relaxed) {
                break;
            }
        }
        let _ = unsafe { CloseHandle(handle) };
    });
    let server = PipeServerHandle {
        pipe_name,
        session_id,
        stop,
        join: Some(join),
        ready: ready_rx,
    };
    server.wait_ready(HANDSHAKE_TIMEOUT)?;
    Ok(server)
}

fn serve_connection(
    handle: HANDLE,
    endpoint: &mut HostTransportEndpoint,
    session_id: &str,
    handler: &RequestHandler,
    handoff: Option<&HandoffProvider>,
    recovery: Option<&Arc<std::sync::Mutex<crate::recovery_store::DurableRecoveryStore>>>,
) {
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let Ok(frame) = read_frame_blocking(handle, &mut buffer) else {
            return;
        };
        let payload = frame.payload.as_str();
        if let Some(protocol) = payload.strip_prefix("HELLO|") {
            if endpoint.accept_hello(protocol, session_id).is_ok() {
                if write_frame(handle, &format!("WELCOME|{session_id}")).is_err() {
                    return;
                }
            } else {
                let _ = write_frame(handle, "ERROR|protocol_mismatch");
                return;
            }
            continue;
        }
        if let Some(recovery_command) = payload.strip_prefix("RECOVERY|") {
            let text = handle_recovery_command(endpoint, recovery_command, recovery);
            if write_frame(handle, &format!("RESULT|DEFINITE|{text}")).is_err() {
                return;
            }
            continue;
        }
        if let Some(query_session) = payload.strip_prefix("HANDOFF|") {
            if endpoint.check_session(query_session).is_err() {
                if write_frame(handle, "ERROR|session_mismatch").is_err() {
                    return;
                }
                continue;
            }
            // Read-only query: it never touches the request ledger.
            let text = match handoff {
                Some(provider) => match provider() {
                    Ok(request) => handoff_payload(&request),
                    Err(error) => format!("handoff-rejected:{}", error.0),
                },
                None => "handoff-unavailable".to_owned(),
            };
            if write_frame(handle, &format!("RESULT|DEFINITE|{text}")).is_err() {
                return;
            }
            continue;
        }
        if let Some(request) = payload.strip_prefix("REQ|") {
            let Some((request_session, request_id, composition, canonical)) =
                parse_request(request)
            else {
                return;
            };
            if endpoint.check_session(request_session).is_err() {
                if write_frame(handle, "ERROR|session_mismatch").is_err() {
                    return;
                }
                continue;
            }
            let outcome = match endpoint.classify_request(request_id, canonical) {
                RequestDisposition::Duplicate(recorded) => recorded,
                RequestDisposition::Conflict => {
                    LedgerOutcome::Definite("request_id_reuse".to_owned())
                }
                RequestDisposition::Fresh => {
                    let outcome = handler(request_id, composition, canonical);
                    endpoint.record(request_id, canonical, outcome.clone());
                    outcome
                }
            };
            if write_frame(handle, &outcome_payload(&outcome)).is_err() {
                return;
            }
            continue;
        }
        if let Some(request) = payload.strip_prefix("REQX|") {
            if !serve_descriptor_request(handle, endpoint, recovery, session_id, request, handler) {
                return;
            }
            continue;
        }
        // Unknown well-formed payloads are ignored fail-closed: drop.
        return;
    }
}

/// Operator recovery commands (M3D-28/M3D-31), reusing the session-bound
/// framing: `RECOVERY|<session>|LIST`, `RECOVERY|<session>|BLOCK|<uri>|
/// <expected>|<replacement>|<start>|<end>`, `RECOVERY|<session>|RECONCILE|
/// <uri>|<expected>|<live-range-text>`, and `RECOVERY|<session>|ACK|<uri>|
/// <expected>`. Every answer is a definitive transport result; failures are
/// `recovery-error:<reason>` strings, never mutations, never retries.
/// Recovery state is the M3D-31 durable store: mutations are write-through
/// to the bounded state file, and an unreadable durable state answers
/// every command with a typed fail-closed error.
fn handle_recovery_command(
    endpoint: &HostTransportEndpoint,
    command: &str,
    recovery: Option<&Arc<std::sync::Mutex<crate::recovery_store::DurableRecoveryStore>>>,
) -> String {
    let mut parts = command.split('|');
    let Some(command_session) = parts.next() else {
        return "recovery-error:MalformedCommand".to_owned();
    };
    if endpoint.check_session(command_session).is_err() {
        return "recovery-error:SessionMismatch".to_owned();
    }
    let Some(store) = recovery else {
        return "recovery-error:RecoveryUnavailable".to_owned();
    };
    let Ok(mut store) = store.lock() else {
        return "recovery-error:RegistryLocked".to_owned();
    };
    let map_store_error = |error: crate::recovery_store::RecoveryStoreError| match error {
        crate::recovery_store::RecoveryStoreError::StateUnreadable(code) => {
            format!("recovery-error:StateUnreadable:{code:?}")
        }
        crate::recovery_store::RecoveryStoreError::Unavailable => {
            "recovery-error:StateUnavailable".to_owned()
        }
        crate::recovery_store::RecoveryStoreError::WriteFailed => {
            "recovery-error:StateWriteFailed".to_owned()
        }
        crate::recovery_store::RecoveryStoreError::Registry(reason) => {
            format!("recovery-error:{reason:?}")
        }
    };
    match parts.next() {
        Some("LIST") => match store.list() {
            Ok(targets) => {
                let mut text = format!("recovery-list|{}", targets.len());
                for target in targets {
                    let state = match target.state {
                        None => "awaiting".to_owned(),
                        Some((verdict, acknowledged)) => {
                            format!("{verdict:?}|acked={acknowledged}")
                        }
                    };
                    text.push('\u{1}');
                    text.push_str(&target.uri);
                    text.push('\u{1}');
                    text.push_str(&target.expected.display());
                    text.push('\u{1}');
                    text.push_str(&state);
                }
                text
            }
            Err(error) => map_store_error(error),
        },
        Some("BLOCK") => {
            let (Some(uri), Some(expected), Some(replacement), Some(start), Some(end)) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            ) else {
                return "recovery-error:MalformedCommand".to_owned();
            };
            let (Ok(start), Ok(end)) = (start.parse::<usize>(), end.parse::<usize>()) else {
                return "recovery-error:MalformedCommand".to_owned();
            };
            match store.block(command_session, uri, expected, replacement, (start, end)) {
                Ok(_) => "recovery-blocked".to_owned(),
                Err(error) => map_store_error(error),
            }
        }
        Some("RECONCILE") => {
            let (Some(uri), Some(expected), Some(live)) =
                (parts.next(), parts.next(), parts.next())
            else {
                return "recovery-error:MalformedCommand".to_owned();
            };
            // The live range text may itself contain '|': rejoin the tail.
            let tail: Vec<&str> = parts.collect();
            let live = if tail.is_empty() {
                live.to_owned()
            } else {
                format!("{live}|{}", tail.join("|"))
            };
            match store.reconcile(command_session, uri, expected, &live) {
                Ok(verdict) => format!("recovery-verdict:{verdict:?}"),
                Err(error) => map_store_error(error),
            }
        }
        Some("ACK") => {
            let (Some(uri), Some(expected)) = (parts.next(), parts.next()) else {
                return "recovery-error:MalformedCommand".to_owned();
            };
            match store.acknowledge(command_session, uri, expected) {
                Ok(()) => "recovery-acked".to_owned(),
                Err(error) => map_store_error(error),
            }
        }
        _ => "recovery-error:MalformedCommand".to_owned(),
    }
}

fn parse_request(payload: &str) -> Option<(&str, &str, &str, &str)> {
    let mut parts = payload.splitn(4, '|');
    let session = parts.next()?;
    let request_id = parts.next()?;
    let composition = parts.next()?;
    let canonical = parts.next()?;
    if session.is_empty() || request_id.is_empty() || canonical.is_empty() {
        return None;
    }
    Some((session, request_id, composition, canonical))
}

/// Parses one descriptor-carrying request body (everything after `REQX|`):
/// `<session>|<request_id>|<composition>|<uri>|<start>|<end>|<expected>|
/// <replacement>|<generation>|<canonical tail>`. The UTF-16 range is
/// host-owned snapshot data carried verbatim; it is never derived from
/// service token lengths.
fn parse_descriptor_request(
    payload: &str,
) -> Option<(
    &str,
    zonkey_service::transport::RecoveryDescriptor,
    &str,
    &str,
)> {
    let mut parts = payload.splitn(10, '|');
    let session = parts.next()?;
    let request_id = parts.next()?;
    let composition = parts.next()?;
    let uri = parts.next()?;
    let start = parts.next()?;
    let end = parts.next()?;
    let expected = parts.next()?;
    let replacement = parts.next()?;
    let generation = parts.next()?;
    let canonical = parts.next()?;
    let (Ok(start), Ok(end), Ok(generation)) = (
        start.parse::<usize>(),
        end.parse::<usize>(),
        generation.parse::<u64>(),
    ) else {
        return None;
    };
    if session.is_empty()
        || request_id.is_empty()
        || composition.is_empty()
        || uri.is_empty()
        || expected.is_empty()
        || replacement.is_empty()
        || canonical.is_empty()
        || start > end
    {
        return None;
    }
    Some((
        session,
        zonkey_service::transport::RecoveryDescriptor {
            request_id: request_id.to_owned(),
            uri: uri.to_owned(),
            range: (start, end),
            expected: expected.to_owned(),
            replacement: replacement.to_owned(),
            generation,
        },
        composition,
        canonical,
    ))
}

/// Serves one descriptor-carrying request (`REQX`) under the M3D-31 durable
/// preflight rule. Returns false when the connection must be dropped.
fn serve_descriptor_request(
    handle: HANDLE,
    endpoint: &mut HostTransportEndpoint,
    recovery: Option<&Arc<std::sync::Mutex<crate::recovery_store::DurableRecoveryStore>>>,
    _session_id: &str,
    request: &str,
    handler: &RequestHandler,
) -> bool {
    let Some((request_session, descriptor, composition, canonical)) =
        parse_descriptor_request(request)
    else {
        return false;
    };
    let request_id = descriptor.request_id.clone();
    if endpoint.check_session(request_session).is_err() {
        return write_frame(handle, "ERROR|session_mismatch").is_ok();
    }
    let mut executed_fresh = false;
    let outcome = match endpoint.classify_request(&request_id, canonical) {
        RequestDisposition::Duplicate(recorded) => recorded,
        RequestDisposition::Conflict => LedgerOutcome::Definite("request_id_reuse".to_owned()),
        RequestDisposition::Fresh => {
            executed_fresh = true;
            execute_descriptor_request(
                endpoint,
                recovery,
                request_session,
                &descriptor,
                composition,
                canonical,
                handler,
            )
        }
    };
    // Resolve the durable intent only after the delivery attempt: a
    // delivered definitive rejection clears it; a delivered uncertain
    // outcome promotes it; a failed delivery promotes it (ADR 0036
    // disconnect rule).
    if write_frame(handle, &outcome_payload(&outcome)).is_ok() {
        if executed_fresh {
            resolve_pending(recovery, request_session, &request_id, &outcome);
        }
        return true;
    }
    if executed_fresh {
        promote_after_loss(recovery, request_session, &request_id);
    }
    false
}

/// Executes one fresh descriptor-carrying request under the M3D-31 durable
/// preflight rule: a PendingRecovery record must exist durably before the
/// request may proceed. The durable intent is left open here; the caller
/// resolves it with [`resolve_pending`] or [`promote_after_loss`] after the
/// delivery attempt. No mutation path exists — the handler is the same
/// fail-closed request handler used for plain requests.
fn execute_descriptor_request(
    endpoint: &mut HostTransportEndpoint,
    recovery: Option<&Arc<std::sync::Mutex<crate::recovery_store::DurableRecoveryStore>>>,
    _session_id: &str,
    descriptor: &zonkey_service::transport::RecoveryDescriptor,
    composition: &str,
    canonical: &str,
    handler: &RequestHandler,
) -> LedgerOutcome {
    let Some(store) = recovery else {
        return LedgerOutcome::Definite("rejected:RecoveryUnavailable".to_owned());
    };
    let Ok(mut store) = store.lock() else {
        return LedgerOutcome::Definite("rejected:RecoveryUnavailable".to_owned());
    };
    // A blocked logical target never reaches execution, before or after a
    // restart, until reconciliation plus owner acknowledgement.
    if store.target_blocked(&descriptor.uri, &descriptor.expected) {
        return LedgerOutcome::Definite("rejected:TargetBlocked".to_owned());
    }
    // Durable preflight: only after durable success may the request proceed.
    if store.begin_pending(descriptor).is_err() {
        return LedgerOutcome::Definite("rejected:RecoveryPreflightFailed".to_owned());
    }
    let outcome = handler(&descriptor.request_id, composition, canonical);
    endpoint.record_with_recovery(&descriptor.request_id, canonical, outcome.clone());
    outcome
}

/// Resolves an open durable intent after a delivered outcome: only a
/// definitive rejection with no mutation possibility clears it; Applied,
/// ambiguous, and any other definite outcome promote it to a block.
fn resolve_pending(
    recovery: Option<&Arc<std::sync::Mutex<crate::recovery_store::DurableRecoveryStore>>>,
    session_id: &str,
    request_id: &str,
    outcome: &LedgerOutcome,
) {
    let uncertain = match outcome {
        LedgerOutcome::Ambiguous(_) => true,
        LedgerOutcome::Definite(text) => !text.starts_with("rejected:"),
    };
    if let Some(store) = recovery
        && let Ok(mut store) = store.lock()
    {
        if uncertain {
            let _ = store.promote_pending(session_id, request_id);
        } else {
            let _ = store.clear_pending(request_id);
        }
    }
}

/// Promotes a pending intent after the response could not be delivered.
fn promote_after_loss(
    recovery: Option<&Arc<std::sync::Mutex<crate::recovery_store::DurableRecoveryStore>>>,
    session_id: &str,
    request_id: &str,
) {
    if let Some(store) = recovery
        && let Ok(mut store) = store.lock()
    {
        let _ = store.promote_pending(session_id, request_id);
    }
}

fn outcome_payload(outcome: &LedgerOutcome) -> String {
    match outcome {
        LedgerOutcome::Definite(text) => format!("RESULT|DEFINITE|{text}"),
        LedgerOutcome::Ambiguous(text) => format!("RESULT|AMBIGUOUS|{text}"),
    }
}

fn parse_outcome(payload: &str) -> Result<LedgerOutcome, PipeError> {
    let mut parts = payload.splitn(3, '|');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("RESULT"), Some("DEFINITE"), Some(text)) => {
            Ok(LedgerOutcome::Definite(text.to_owned()))
        }
        (Some("RESULT"), Some("AMBIGUOUS"), Some(text)) => {
            Ok(LedgerOutcome::Ambiguous(text.to_owned()))
        }
        _ => Err(PipeError::InvalidPayload("malformed result".to_owned())),
    }
}

fn open_raw_handle(pipe_name: &str, timeout: Duration) -> Result<HANDLE, PipeError> {
    let name = wide(pipe_name);
    let deadline = Instant::now() + timeout;
    loop {
        let opened = unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                // READ_CONTROL lets hardened clients and tests inspect the
                // pipe DACL (query only; it grants no write and no access
                // to other users).
                windows::Win32::Foundation::GENERIC_READ.0
                    | windows::Win32::Foundation::GENERIC_WRITE.0
                    | READ_CONTROL.0,
                FILE_SHARE_MODE(0),
                None,
                FILE_CREATION_DISPOSITION(3), // OPEN_EXISTING
                // Identification-level SQOS: the server may identify this
                // client (read its token user SID) but never act as it.
                FILE_ATTRIBUTE_NORMAL | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        };
        match opened {
            Ok(handle) => return Ok(handle),
            Err(error) => {
                let busy = error.code() == HRESULT(HRESULT_PIPE_BUSY);
                if busy {
                    // A single short wait may miss a listener that is about
                    // to re-enter ConnectNamedPipe (for example right after
                    // a restart); keep honoring the caller's deadline
                    // instead of giving up on the first miss.
                    let _ = unsafe { WaitNamedPipeW(PCWSTR(name.as_ptr()), 50) };
                    if Instant::now() >= deadline {
                        return Err(PipeError::ConnectTimeout);
                    }
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err(PipeError::ConnectTimeout);
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

/// Client connection over one pipe instance.
pub struct PipeClient {
    handle: Option<HANDLE>,
    session_id: String,
    broken: bool,
}

impl PipeClient {
    /// Connects and completes the authenticated, session-bound handshake.
    ///
    /// # Errors
    ///
    /// Returns a [`PipeError`] on connect timeout, protocol mismatch, or a
    /// refused handshake.
    pub fn connect(pipe_name: &str, connect_timeout: Duration) -> Result<Self, PipeError> {
        let handle = open_raw_handle(pipe_name, connect_timeout)?;
        write_frame(handle, &format!("HELLO|{TRANSPORT_PROTOCOL_ID}"))?;
        let reply = read_frame_with_timeout(handle, HANDSHAKE_TIMEOUT)?;
        if let Some(reason) = reply.payload.strip_prefix("ERROR|") {
            let _ = unsafe { CloseHandle(handle) };
            return Err(match reason {
                "protocol_mismatch" => PipeError::ProtocolMismatch,
                other => PipeError::HandshakeRefused(other.to_owned()),
            });
        }
        let Some(session) = reply.payload.strip_prefix("WELCOME|") else {
            let _ = unsafe { CloseHandle(handle) };
            return Err(PipeError::InvalidPayload("expected welcome".to_owned()));
        };
        Ok(Self {
            handle: Some(handle),
            session_id: session.to_owned(),
            broken: false,
        })
    }

    /// The server-issued session id bound to this connection.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Sends one compare-and-replace style request and awaits its result.
    ///
    /// # Errors
    ///
    /// Transport failures return [`PipeError::Timeout`] or
    /// [`PipeError::ConnectionLost`]; callers must map both to
    /// `Indeterminate` via [`ambiguous_loss_outcome`] and never retry
    /// automatically.
    pub fn request(
        &mut self,
        request_id: &str,
        composition: &str,
        canonical: &str,
        timeout: Duration,
    ) -> Result<LedgerOutcome, PipeError> {
        if self.broken {
            return Err(PipeError::ConnectionLost);
        }
        let handle = self.handle.ok_or(PipeError::ConnectionLost)?;
        let session = self.session_id.clone();
        self.request_on(
            handle,
            &session,
            request_id,
            composition,
            canonical,
            timeout,
        )
    }

    /// Sends one descriptor-carrying request and awaits its result. The
    /// descriptor makes the request participate in the M3D-31 durable
    /// preflight lifecycle on the server; no mutation path exists and the
    /// same fail-closed handler answers.
    ///
    /// # Errors
    ///
    /// Returns the same transport errors as [`PipeClient::request`];
    /// callers must map loss to `Indeterminate` and never retry
    /// automatically.
    pub fn request_with_descriptor(
        &mut self,
        descriptor: &zonkey_service::transport::RecoveryDescriptor,
        composition: &str,
        canonical: &str,
        timeout: Duration,
    ) -> Result<LedgerOutcome, PipeError> {
        if self.broken {
            return Err(PipeError::ConnectionLost);
        }
        let handle = self.handle.ok_or(PipeError::ConnectionLost)?;
        let session = self.session_id.clone();
        let payload = format!(
            "REQX|{session}|{}|{composition}|{}|{}|{}|{}|{}|{}|{canonical}",
            descriptor.request_id,
            descriptor.uri,
            descriptor.range.0,
            descriptor.range.1,
            descriptor.expected,
            descriptor.replacement,
            descriptor.generation,
        );
        self.descriptor_request_on(handle, &payload, timeout)
    }

    fn descriptor_request_on(
        &mut self,
        handle: HANDLE,
        payload: &str,
        timeout: Duration,
    ) -> Result<LedgerOutcome, PipeError> {
        if write_frame(handle, payload).is_err() {
            self.broken = true;
            return Err(PipeError::ConnectionLost);
        }
        match read_frame_with_timeout(handle, timeout) {
            Ok(frame) => {
                if let Some(reason) = frame.payload.strip_prefix("ERROR|") {
                    return Err(match reason {
                        "session_mismatch" => PipeError::SessionMismatch,
                        other => PipeError::HandshakeRefused(other.to_owned()),
                    });
                }
                match parse_outcome(&frame.payload) {
                    Ok(outcome) => Ok(outcome),
                    Err(error) => {
                        self.broken = true;
                        Err(error)
                    }
                }
            }
            Err(PipeError::Timeout | PipeError::ConnectionLost) => {
                self.broken = true;
                Err(PipeError::Timeout)
            }
            Err(other) => {
                self.broken = true;
                Err(other)
            }
        }
    }

    fn request_on(
        &mut self,
        handle: HANDLE,
        session: &str,
        request_id: &str,
        composition: &str,
        canonical: &str,
        timeout: Duration,
    ) -> Result<LedgerOutcome, PipeError> {
        if write_frame(
            handle,
            &format!("REQ|{session}|{request_id}|{composition}|{canonical}"),
        )
        .is_err()
        {
            self.broken = true;
            return Err(PipeError::ConnectionLost);
        }
        match read_frame_with_timeout(handle, timeout) {
            Ok(frame) => {
                if let Some(reason) = frame.payload.strip_prefix("ERROR|") {
                    // Server-level errors keep the framed connection usable.
                    return Err(match reason {
                        "session_mismatch" => PipeError::SessionMismatch,
                        other => PipeError::HandshakeRefused(other.to_owned()),
                    });
                }
                match parse_outcome(&frame.payload) {
                    Ok(outcome) => Ok(outcome),
                    Err(error) => {
                        self.broken = true;
                        Err(error)
                    }
                }
            }
            Err(PipeError::Timeout | PipeError::ConnectionLost) => {
                self.broken = true;
                Err(PipeError::Timeout)
            }
            Err(other) => {
                self.broken = true;
                Err(other)
            }
        }
    }

    /// Sends one read-only `HANDOFF` query and returns the result payload
    /// text. The query never touches the request ledger.
    ///
    /// # Errors
    ///
    /// Returns the same transport errors as [`PipeClient::request`].
    pub fn handoff_query(&mut self, timeout: Duration) -> Result<String, PipeError> {
        if self.broken {
            return Err(PipeError::ConnectionLost);
        }
        let handle = self.handle.ok_or(PipeError::ConnectionLost)?;
        let session = self.session_id.clone();
        if write_frame(handle, &format!("HANDOFF|{session}")).is_err() {
            self.broken = true;
            return Err(PipeError::ConnectionLost);
        }
        match read_frame_with_timeout(handle, timeout) {
            Ok(frame) => {
                if let Some(reason) = frame.payload.strip_prefix("ERROR|") {
                    return Err(match reason {
                        "session_mismatch" => PipeError::SessionMismatch,
                        other => PipeError::HandshakeRefused(other.to_owned()),
                    });
                }
                if let Some(text) = frame.payload.strip_prefix("RESULT|DEFINITE|") {
                    Ok(text.to_owned())
                } else {
                    self.broken = true;
                    Err(PipeError::InvalidPayload(
                        "expected handoff result".to_owned(),
                    ))
                }
            }
            Err(PipeError::Timeout | PipeError::ConnectionLost) => {
                self.broken = true;
                Err(PipeError::Timeout)
            }
            Err(other) => {
                self.broken = true;
                Err(other)
            }
        }
    }

    /// Sends one operator `RECOVERY` command (without the `RECOVERY|`
    /// prefix or session, which this method adds) and returns the result
    /// payload text. Recovery commands never touch the request ledger and
    /// never mutate documents.
    ///
    /// # Errors
    ///
    /// Returns the same transport errors as [`PipeClient::request`].
    pub fn recovery_command(
        &mut self,
        command: &str,
        timeout: Duration,
    ) -> Result<String, PipeError> {
        if self.broken {
            return Err(PipeError::ConnectionLost);
        }
        let handle = self.handle.ok_or(PipeError::ConnectionLost)?;
        let session = self.session_id.clone();
        if write_frame(handle, &format!("RECOVERY|{session}|{command}")).is_err() {
            self.broken = true;
            return Err(PipeError::ConnectionLost);
        }
        match read_frame_with_timeout(handle, timeout) {
            Ok(frame) => {
                if let Some(reason) = frame.payload.strip_prefix("ERROR|") {
                    return Err(match reason {
                        "session_mismatch" => PipeError::SessionMismatch,
                        other => PipeError::HandshakeRefused(other.to_owned()),
                    });
                }
                if let Some(text) = frame.payload.strip_prefix("RESULT|DEFINITE|") {
                    Ok(text.to_owned())
                } else {
                    self.broken = true;
                    Err(PipeError::InvalidPayload(
                        "expected recovery result".to_owned(),
                    ))
                }
            }
            Err(PipeError::Timeout | PipeError::ConnectionLost) => {
                self.broken = true;
                Err(PipeError::Timeout)
            }
            Err(other) => {
                self.broken = true;
                Err(other)
            }
        }
    }

    /// Closes the connection without waiting for any result.
    pub fn close(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = unsafe { CloseHandle(handle) };
        }
    }
}

impl Drop for PipeClient {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = unsafe { CloseHandle(handle) };
        }
    }
}

/// Reader body kept as a function so the spawned closure captures the whole
/// `SendHandle` (a raw field access would capture the non-Send `HANDLE`).
fn run_frame_reader(
    reader: &SendHandle,
    handle_tx: &mpsc::Sender<SendHandle>,
    result_tx: &mpsc::Sender<Result<Frame, PipeError>>,
) {
    let handle = reader.0;
    let mut duplicated = HANDLE::default();
    let registered = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            GetCurrentThread(),
            GetCurrentProcess(),
            &raw mut duplicated,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }
    .is_ok();
    if registered {
        let _ = handle_tx.send(SendHandle(duplicated));
    }
    let mut buffer: Vec<u8> = Vec::new();
    let outcome = read_frame_blocking(handle, &mut buffer);
    let message = match outcome {
        Ok(frame) => Ok(frame),
        Err(PipeError::Frame(error)) => Err(PipeError::Frame(error)),
        Err(_) => Err(PipeError::ConnectionLost),
    };
    let _ = result_tx.send(message);
}

/// Reads one frame with a bounded timeout by cancelling the blocking read.
fn read_frame_with_timeout(handle: HANDLE, timeout: Duration) -> Result<Frame, PipeError> {
    let (handle_tx, handle_rx) = mpsc::channel::<SendHandle>();
    let (result_tx, result_rx) = mpsc::channel::<Result<Frame, PipeError>>();
    let reader = SendHandle(handle);
    thread::spawn(move || run_frame_reader(&reader, &handle_tx, &result_tx));
    let reader_thread = handle_rx
        .recv_timeout(Duration::from_millis(500))
        .map(|wrapped| wrapped.0);
    match result_rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => {
            if let Ok(thread_handle) = reader_thread {
                let _ = unsafe { CancelSynchronousIo(thread_handle) };
            }
            // Reap the reader result so the thread finishes deterministically.
            let _ = result_rx.recv_timeout(Duration::from_secs(1));
            Err(PipeError::Timeout)
        }
        Err(RecvTimeoutError::Disconnected) => Err(PipeError::ConnectionLost),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery_store::DurableRecoveryStore;
    use std::sync::atomic::AtomicUsize;
    use zonkey_service::transport::ambiguous_loss_outcome;

    fn pipe_name(tag: &str) -> String {
        format!(
            r"\\.\pipe\zonkey-m3d20-{tag}-{}",
            SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn recovery_store_dir(tag: &str) -> crate::recovery_store::DurableRecoveryStore {
        let unique = crate::pipe_security::random_nonce_hex(8).expect("nonce");
        let dir = std::env::temp_dir().join(format!("zonkey-m3d31-pipe-{tag}-{unique}"));
        crate::recovery_store::DurableRecoveryStore::open_in(&dir, 4)
    }

    fn counting_handler() -> (RequestHandler, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let handler: RequestHandler = Arc::new(move |_request_id, _composition, _canonical| {
            counter.fetch_add(1, Ordering::Relaxed);
            LedgerOutcome::Definite("applied".to_owned())
        });
        (handler, calls)
    }

    fn spawn(tag: &str, handler: RequestHandler) -> PipeServerHandle {
        spawn_dummy_host_server(&pipe_name(tag), 8, handler).expect("server spawns")
    }

    fn connect(server: &PipeServerHandle) -> PipeClient {
        PipeClient::connect(&server.pipe_name, Duration::from_secs(5)).expect("client connects")
    }

    #[test]
    fn connect_hello_success_binds_server_session() {
        let (handler, _calls) = counting_handler();
        let server = spawn("hello", handler);
        let client = connect(&server);
        assert!(!client.session_id().is_empty());
        assert_eq!(client.session_id(), server.session_id);
        client.close();
        server.shutdown();
    }

    #[test]
    fn request_result_roundtrip_with_dummy_host() {
        let (handler, calls) = counting_handler();
        let server = spawn("roundtrip", handler);
        let mut client = connect(&server);
        let outcome = client
            .request("req-1", "Inactive", "{canonical}", Duration::from_secs(5))
            .expect("roundtrip");
        assert_eq!(outcome, LedgerOutcome::Definite("applied".to_owned()));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        client.close();
        server.shutdown();
    }

    #[test]
    fn duplicate_request_replays_without_reexecution() {
        let (handler, calls) = counting_handler();
        let server = spawn("duplicate", handler);
        let mut client = connect(&server);
        let first = client
            .request("req-1", "Inactive", "{canonical}", Duration::from_secs(5))
            .expect("first");
        let second = client
            .request("req-1", "Inactive", "{canonical}", Duration::from_secs(5))
            .expect("replay");
        assert_eq!(first, second);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        client.close();
        server.shutdown();
    }

    #[test]
    fn protocol_mismatch_rejects_hello() {
        let (handler, _calls) = counting_handler();
        let server = spawn("protocol", handler);
        let handle = open_raw_handle(&server.pipe_name, Duration::from_secs(5)).expect("open");
        write_frame(handle, "HELLO|zonkey.other/1").expect("send hello");
        let reply = read_frame_with_timeout(handle, HANDSHAKE_TIMEOUT).expect("reply");
        assert_eq!(reply.payload, "ERROR|protocol_mismatch");
        let _ = unsafe { CloseHandle(handle) };
        server.shutdown();
    }

    #[test]
    fn session_mismatch_rejects_request_before_execution() {
        let (handler, calls) = counting_handler();
        let server = spawn("session", handler);
        let mut client = connect(&server);
        let handle = client.handle.expect("handle");
        let error = client
            .request_on(
                handle,
                "bogus-session",
                "req-1",
                "Inactive",
                "{canonical}",
                Duration::from_secs(5),
            )
            .expect_err("session mismatch");
        assert_eq!(error, PipeError::SessionMismatch);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        // The correct session still works afterwards on the same connection.
        let outcome = client
            .request("req-1", "Inactive", "{canonical}", Duration::from_secs(5))
            .expect("valid session works");
        assert_eq!(outcome, LedgerOutcome::Definite("applied".to_owned()));
        client.close();
        server.shutdown();
    }

    #[test]
    fn malformed_frame_fails_closed() {
        let (handler, _calls) = counting_handler();
        let server = spawn("malformed", handler);
        let handle = open_raw_handle(&server.pipe_name, Duration::from_secs(5)).expect("open");
        // Frame with invalid UTF-8 payload.
        let mut frame = 2u32.to_le_bytes().to_vec();
        frame.extend_from_slice(&[0xd8, 0x00]);
        write_all(handle, &frame).expect("send malformed");
        let error = read_frame_with_timeout(handle, Duration::from_secs(5)).expect_err("closed");
        assert_eq!(error, PipeError::ConnectionLost);
        let _ = unsafe { CloseHandle(handle) };
        server.shutdown();
    }

    #[test]
    fn oversized_frame_fails_closed() {
        let (handler, _calls) = counting_handler();
        let server = spawn("oversized", handler);
        let handle = open_raw_handle(&server.pipe_name, Duration::from_secs(5)).expect("open");
        let mut frame = u32::try_from(zonkey_service::transport::MAX_FRAME_BYTES + 1)
            .unwrap()
            .to_le_bytes()
            .to_vec();
        frame.extend_from_slice(&[0x78]);
        write_all(handle, &frame).expect("send oversized");
        let error = read_frame_with_timeout(handle, Duration::from_secs(5)).expect_err("closed");
        assert_eq!(error, PipeError::ConnectionLost);
        let _ = unsafe { CloseHandle(handle) };
        server.shutdown();
    }

    #[test]
    fn disconnect_after_request_is_indeterminate_then_replayable() {
        let (handler, calls) = counting_handler();
        let server = spawn("disconnect", handler);
        // Client 1 sends a request and drops the connection before reading.
        let dropped = open_raw_handle(&server.pipe_name, Duration::from_secs(5)).expect("open");
        write_frame(dropped, &format!("HELLO|{TRANSPORT_PROTOCOL_ID}")).expect("hello");
        let welcome = read_frame_with_timeout(dropped, HANDSHAKE_TIMEOUT).expect("welcome");
        let session = welcome.payload.strip_prefix("WELCOME|").expect("session");
        write_frame(
            dropped,
            &format!("REQ|{session}|req-1|Inactive|{{canonical}}"),
        )
        .expect("send request");
        let _ = unsafe { CloseHandle(dropped) };
        // From the dropped client's perspective the outcome is lost.
        let lost = ambiguous_loss_outcome();
        assert!(matches!(lost, LedgerOutcome::Ambiguous(_)));
        // A reconnecting client with the same request id replays the result.
        let mut client = connect(&server);
        let replay = client
            .request("req-1", "Inactive", "{canonical}", Duration::from_secs(5))
            .expect("replay resolves ambiguity");
        assert_eq!(replay, LedgerOutcome::Definite("applied".to_owned()));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        client.close();
        server.shutdown();
    }

    #[test]
    fn server_restart_invalidates_session_history() {
        let (handler, calls) = counting_handler();
        let name = pipe_name("restart");
        let first = spawn_dummy_host_server(&name, 8, Arc::clone(&handler)).expect("first server");
        let first_session = first.session_id.clone();
        {
            let mut client = PipeClient::connect(&name, Duration::from_secs(5)).expect("client");
            let outcome = client
                .request("req-1", "Inactive", "{canonical}", Duration::from_secs(5))
                .expect("first run");
            assert_eq!(outcome, LedgerOutcome::Definite("applied".to_owned()));
            client.close();
        }
        first.shutdown();
        let second = spawn_dummy_host_server(&name, 8, handler).expect("second server");
        assert_ne!(first_session, second.session_id);
        {
            let mut client = PipeClient::connect(&name, Duration::from_secs(5)).expect("client");
            let outcome = client
                .request("req-1", "Inactive", "{canonical}", Duration::from_secs(5))
                .expect("fresh execution after restart");
            assert_eq!(outcome, LedgerOutcome::Definite("applied".to_owned()));
            // History was invalidated: the handler ran again.
            assert_eq!(calls.load(Ordering::Relaxed), 2);
            client.close();
        }
        second.shutdown();
    }

    #[test]
    fn bounded_read_timeout_maps_to_indeterminate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let handler: RequestHandler = Arc::new(move |_id, _composition, _canonical| {
            counter.fetch_add(1, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(1200));
            LedgerOutcome::Definite("applied".to_owned())
        });
        let server = spawn("timeout", handler);
        let mut client = connect(&server);
        let error = client
            .request(
                "req-1",
                "Inactive",
                "{canonical}",
                Duration::from_millis(300),
            )
            .expect_err("timeout");
        assert_eq!(error, PipeError::Timeout);
        // Caller maps the lost outcome to Indeterminate; no retry happens.
        assert!(matches!(
            ambiguous_loss_outcome(),
            LedgerOutcome::Ambiguous(_)
        ));
        // A later request on the same broken client fails closed.
        let again = client
            .request("req-2", "Inactive", "{canonical}", Duration::from_secs(5))
            .expect_err("broken connection");
        assert_eq!(again, PipeError::ConnectionLost);
        client.close();
        server.shutdown();
    }

    #[test]
    fn composition_gate_endpoint_rejects_unknown_without_execution() {
        let name = pipe_name("gate");
        let server = spawn_dummy_host_server(&name, 8, composition_gate_handler()).expect("server");
        let mut client = connect(&server);
        let outcome = client
            .request("req-1", "Unknown", "{canonical}", Duration::from_secs(5))
            .expect("gated");
        assert_eq!(
            outcome,
            LedgerOutcome::Definite("rejected:CompositionUnknown".to_owned())
        );
        // Duplicate replay: same rejection, still no execution path.
        let replay = client
            .request("req-1", "Unknown", "{canonical}", Duration::from_secs(5))
            .expect("replay");
        assert_eq!(replay, outcome);
        // A proven-inactive request is still rejected in this milestone.
        let inactive = client
            .request("req-2", "Inactive", "{canonical}", Duration::from_secs(5))
            .expect("gated");
        assert_eq!(
            inactive,
            LedgerOutcome::Definite("rejected:ExecutionNotImplemented".to_owned())
        );
        client.close();
        server.shutdown();
    }

    #[test]
    fn handoff_query_returns_validated_request_over_pipe() {
        let name = pipe_name("handoff");
        let provider: HandoffProvider = Arc::new(|| {
            let mut processor = zonkey_service::DiagnosticDecisionProcessor::default();
            zonkey_service::transport::feed_token(&mut processor, "resume", 1);
            let handoff = processor
                .current_restore_handoff()
                .ok_or(HandoffRequestWireError("NoCurrentPlan".to_owned()))?;
            zonkey_service::transport::build_host_request(&processor, &handoff)
                .map_err(|error| HandoffRequestWireError(format!("{error:?}")))
        });
        let server = spawn_dummy_host_server_with_handoff(
            &name,
            8,
            composition_gate_handler(),
            Some(provider),
            None,
        )
        .expect("server");
        let mut client = connect(&server);
        let payload = client
            .handoff_query(Duration::from_secs(5))
            .expect("handoff query");
        assert!(
            payload.starts_with("handoff:handoff-1|"),
            "payload={payload}"
        );
        // Telex renders "resume" as "réume" (5 scalar units); the handoff
        // carries the rendered token and the dictionary replacement.
        assert!(payload.contains("|réume|resume|5|6|1"), "payload={payload}");
        client.close();
        server.shutdown();
    }

    #[test]
    fn handoff_query_reports_rejection_reason_over_pipe() {
        let name = pipe_name("handoff-reject");
        let provider: HandoffProvider =
            Arc::new(|| Err(HandoffRequestWireError("NoCurrentPlan".to_owned())));
        let server = spawn_dummy_host_server_with_handoff(
            &name,
            8,
            composition_gate_handler(),
            Some(provider),
            None,
        )
        .expect("server");
        let mut client = connect(&server);
        let payload = client
            .handoff_query(Duration::from_secs(5))
            .expect("query answers");
        assert_eq!(payload, "handoff-rejected:NoCurrentPlan");
        client.close();
        server.shutdown();
    }

    #[test]
    fn recovery_lifecycle_over_pipe() {
        let name = pipe_name("recovery");
        let registry = Arc::new(std::sync::Mutex::new(recovery_store_dir("lifecycle")));
        let (handler, _calls) = counting_handler();
        let server = spawn_dummy_host_server_with_handoff(
            &name,
            8,
            handler,
            None,
            Some(Arc::clone(&registry)),
        )
        .expect("server");
        let mut client = connect(&server);
        assert_eq!(
            client.recovery_command("LIST", Duration::from_secs(5)),
            Ok("recovery-list|0".to_owned())
        );
        assert_eq!(
            client.recovery_command(
                "BLOCK|file:///doc|resume|restored|0|6",
                Duration::from_secs(5)
            ),
            Ok("recovery-blocked".to_owned())
        );
        let listed = client
            .recovery_command("LIST", Duration::from_secs(5))
            .expect("list");
        assert!(listed.starts_with("recovery-list|1"), "list={listed}");
        assert_eq!(
            client.recovery_command("ACK|file:///doc|resume", Duration::from_secs(5)),
            Ok("recovery-error:AckBeforeReconcile".to_owned())
        );
        assert_eq!(
            client.recovery_command(
                "RECONCILE|file:///doc|resume|resume",
                Duration::from_secs(5)
            ),
            Ok("recovery-verdict:NotApplied".to_owned())
        );
        assert_eq!(
            client.recovery_command(
                "RECONCILE|file:///doc|resume|resume",
                Duration::from_secs(5)
            ),
            Ok("recovery-verdict:NotApplied".to_owned())
        );
        assert_eq!(
            client.recovery_command("ACK|file:///doc|resume", Duration::from_secs(5)),
            Ok("recovery-acked".to_owned())
        );
        assert_eq!(
            client.recovery_command("LIST", Duration::from_secs(5)),
            Ok("recovery-list|0".to_owned())
        );
        // Close the first client so the single-instance pipe can accept
        // the raw wrong-session connection.
        client.close();
        let handle = open_raw_handle(&server.pipe_name, Duration::from_secs(5)).expect("open");
        write_frame(handle, "HELLO|zonkey.host-transport/1").expect("hello");
        let welcome = read_frame_with_timeout(handle, HANDSHAKE_TIMEOUT).expect("welcome");
        drop(welcome);
        write_frame(handle, "RECOVERY|bogus-session|LIST").expect("send");
        let reply = read_frame_with_timeout(handle, Duration::from_secs(5)).expect("reply");
        assert_eq!(
            reply.payload,
            "RESULT|DEFINITE|recovery-error:SessionMismatch"
        );
        let _ = unsafe { CloseHandle(handle) };
        server.shutdown();
    }

    #[test]
    fn generated_pipe_names_are_unpredictable_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let name = generate_pipe_name("m3d29").expect("generated name");
            assert!(name.starts_with(r"\\.\pipe\zonkey-m3d29-"), "name={name}");
            let nonce = name.rsplit('-').next().expect("nonce tail");
            assert_eq!(nonce.len(), 32, "name={name}");
            assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(seen.insert(name), "duplicate generated name");
        }
        assert!(
            generate_pipe_name("").is_err(),
            "empty prefix must be rejected"
        );
        assert!(
            generate_pipe_name("bad prefix!").is_err(),
            "non-identifier prefix must be rejected"
        );
    }

    #[test]
    fn session_ids_carry_unpredictable_nonces_per_lifecycle() {
        let (handler, _calls) = counting_handler();
        let first = spawn("nonce-a", Arc::clone(&handler));
        let second = spawn("nonce-b", Arc::clone(&handler));
        assert_ne!(first.session_id, second.session_id);
        for session in [&first.session_id, &second.session_id] {
            let nonce = session.rsplit('-').next().expect("nonce tail");
            assert_eq!(nonce.len(), 32, "session={session}");
            assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()));
        }
        first.shutdown();
        second.shutdown();
    }

    #[test]
    fn hardened_pipe_dacl_grants_only_the_current_user() {
        let (handler, _calls) = counting_handler();
        let server = spawn("dacl", handler);
        let client = connect(&server);
        let handle = client.handle.expect("handle");
        let facts = crate::pipe_security::inspect_current_user_only_dacl(handle)
            .expect("inspect server pipe DACL");
        assert!(facts.ace_count >= 1);
        assert!(
            facts.all_allowed_aces_grant_current_user_only,
            "facts={facts:?}"
        );
        assert!(!facts.has_deny_ace);
        client.close();
        server.shutdown();
    }

    #[test]
    fn restart_with_generated_names_changes_identity_and_old_pipe_dies() {
        let (handler, calls) = counting_handler();
        let first_name = generate_pipe_name("m3d29-restart").expect("first name");
        let first =
            spawn_dummy_host_server(&first_name, 8, Arc::clone(&handler)).expect("first server");
        let stale_session = first.session_id.clone();
        {
            let mut client = connect(&first);
            let outcome = client
                .request("req-1", "Inactive", "{canonical}", Duration::from_secs(5))
                .expect("first lifecycle works");
            assert_eq!(outcome, LedgerOutcome::Definite("applied".to_owned()));
            client.close();
        }
        first.shutdown();
        let second_name = generate_pipe_name("m3d29-restart").expect("second name");
        assert_ne!(first_name, second_name);
        let second = spawn_dummy_host_server(&second_name, 8, handler).expect("second server");
        assert_ne!(stale_session, second.session_id);
        // The old pipe identity is gone: connecting to it fails closed.
        let stale = PipeClient::connect(&first_name, Duration::from_millis(300));
        assert!(matches!(stale, Err(PipeError::ConnectTimeout)));
        // A stale session authorizes nothing on the new lifecycle.
        let mut client = connect(&second);
        let handle = client.handle.expect("handle");
        let error = client
            .request_on(
                handle,
                &stale_session,
                "req-old",
                "Inactive",
                "{canonical}",
                Duration::from_secs(5),
            )
            .expect_err("stale session rejected");
        assert_eq!(error, PipeError::SessionMismatch);
        let outcome = client
            .request("req-1", "Inactive", "{canonical}", Duration::from_secs(5))
            .expect("new lifecycle works");
        assert_eq!(outcome, LedgerOutcome::Definite("applied".to_owned()));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        client.close();
        second.shutdown();
    }

    #[test]
    fn recovery_applied_and_conflict_verdicts_over_pipe() {
        let name = pipe_name("recovery-verdicts");
        let registry = Arc::new(std::sync::Mutex::new(recovery_store_dir("verdicts")));
        let (handler, _calls) = counting_handler();
        let server = spawn_dummy_host_server_with_handoff(
            &name,
            8,
            handler,
            None,
            Some(Arc::clone(&registry)),
        )
        .expect("server");
        let mut client = connect(&server);
        client
            .recovery_command("BLOCK|file:///a|t1|r1|0|2", Duration::from_secs(5))
            .expect("block");
        assert_eq!(
            client.recovery_command("RECONCILE|file:///a|t1|r1", Duration::from_secs(5)),
            Ok("recovery-verdict:AppliedAcknowledged".to_owned())
        );
        client
            .recovery_command("ACK|file:///a|t1", Duration::from_secs(5))
            .expect("ack");
        client
            .recovery_command("BLOCK|file:///b|t2|r2|0|2", Duration::from_secs(5))
            .expect("block");
        assert_eq!(
            client.recovery_command(
                "RECONCILE|file:///b|t2|zz|with|pipes",
                Duration::from_secs(5)
            ),
            Ok("recovery-verdict:ConflictHumanReview".to_owned())
        );
        client
            .recovery_command("ACK|file:///b|t2", Duration::from_secs(5))
            .expect("ack");
        client.close();
        server.shutdown();
    }

    fn test_descriptor(tag: &str) -> zonkey_service::transport::RecoveryDescriptor {
        zonkey_service::transport::RecoveryDescriptor {
            request_id: format!("reqx-{tag}"),
            uri: "file:///reqx-doc".to_owned(),
            range: (0, 6),
            expected: "resume".to_owned(),
            replacement: "restored".to_owned(),
            generation: 1,
        }
    }

    fn ambiguous_handler() -> RequestHandler {
        Arc::new(|_request_id: &str, _composition: &str, _canonical: &str| {
            LedgerOutcome::Ambiguous("lost_in_test".to_owned())
        })
    }

    #[test]
    fn reqx_definitive_rejection_clears_pending_durably() {
        let name = pipe_name("reqx-reject");
        let store = Arc::new(std::sync::Mutex::new(recovery_store_dir("reqx-reject")));
        let server = spawn_dummy_host_server_with_handoff(
            &name,
            8,
            composition_gate_handler(),
            None,
            Some(Arc::clone(&store)),
        )
        .expect("server");
        let mut client = connect(&server);
        let outcome = client
            .request_with_descriptor(
                &test_descriptor("reject"),
                "Unknown",
                "{canonical}",
                Duration::from_secs(5),
            )
            .expect("gated");
        assert_eq!(
            outcome,
            LedgerOutcome::Definite("rejected:CompositionUnknown".to_owned())
        );
        client.close();
        server.shutdown();
        // The definitive no-mutation rejection removed the pending intent:
        // a restart starts clean for this target.
        let reloaded =
            DurableRecoveryStore::open_in(store.lock().expect("store lock").directory(), 8);
        assert!(reloaded.list().expect("list").is_empty());
        assert!(!reloaded.target_blocked("file:///reqx-doc", "resume"));
    }

    #[test]
    fn reqx_ambiguous_outcome_promotes_block_and_restart_requires_ack() {
        let name = pipe_name("reqx-ambiguous");
        let dir = {
            let store = recovery_store_dir("reqx-ambiguous");
            let dir = store.directory().to_path_buf();
            drop(store);
            dir
        };
        let store = Arc::new(std::sync::Mutex::new(DurableRecoveryStore::open_in(
            &dir, 8,
        )));
        let server = spawn_dummy_host_server_with_handoff(
            &name,
            8,
            ambiguous_handler(),
            None,
            Some(Arc::clone(&store)),
        )
        .expect("server");
        let mut client = connect(&server);
        let outcome = client
            .request_with_descriptor(
                &test_descriptor("lost"),
                "Unknown",
                "{canonical}",
                Duration::from_secs(5),
            )
            .expect("ambiguous reply");
        assert!(matches!(outcome, LedgerOutcome::Ambiguous(_)));
        client.close();
        server.shutdown();
        // Restart: the promoted target must reload as blocked.
        let reloaded = DurableRecoveryStore::open_in(&dir, 8);
        let listed = reloaded.list().expect("list");
        assert_eq!(listed.len(), 1, "lost response => blocked target");
        assert_eq!(listed[0].uri, "file:///reqx-doc");
        // A new server over the reloaded state refuses the same target.
        let name_two = pipe_name("reqx-ambiguous-2");
        let store_two = Arc::new(std::sync::Mutex::new(reloaded));
        let server_two = spawn_dummy_host_server_with_handoff(
            &name_two,
            8,
            ambiguous_handler(),
            None,
            Some(Arc::clone(&store_two)),
        )
        .expect("second server");
        let mut client = connect(&server_two);
        let blocked = client
            .request_with_descriptor(
                &test_descriptor("retry"),
                "Unknown",
                "{canonical}",
                Duration::from_secs(5),
            )
            .expect("blocked reply");
        assert_eq!(
            blocked,
            LedgerOutcome::Definite("rejected:TargetBlocked".to_owned())
        );
        // Only reconcile + owner ack unblocks the target.
        assert_eq!(
            client.recovery_command(
                "RECONCILE|file:///reqx-doc|resume|restored",
                Duration::from_secs(5)
            ),
            Ok("recovery-verdict:AppliedAcknowledged".to_owned())
        );
        client
            .recovery_command("ACK|file:///reqx-doc|resume", Duration::from_secs(5))
            .expect("ack");
        let unblocked = client
            .request_with_descriptor(
                &test_descriptor("after-ack"),
                "Unknown",
                "{canonical}",
                Duration::from_secs(5),
            )
            .expect("reply after ack");
        assert_ne!(
            unblocked,
            LedgerOutcome::Definite("rejected:TargetBlocked".to_owned())
        );
        client.close();
        server_two.shutdown();
    }

    #[test]
    fn reqx_client_disconnect_after_send_promotes_blocked() {
        let name = pipe_name("reqx-disconnect");
        let store = Arc::new(std::sync::Mutex::new(recovery_store_dir("reqx-disconnect")));
        let server = spawn_dummy_host_server_with_handoff(
            &name,
            8,
            composition_gate_handler(),
            None,
            Some(Arc::clone(&store)),
        )
        .expect("server");
        // Raw client: complete the handshake, send REQX, vanish before the
        // result can be read.
        let dropped = open_raw_handle(&server.pipe_name, Duration::from_secs(5)).expect("open");
        write_frame(dropped, &format!("HELLO|{TRANSPORT_PROTOCOL_ID}")).expect("hello");
        let welcome = read_frame_with_timeout(dropped, HANDSHAKE_TIMEOUT).expect("welcome");
        let session = welcome.payload.strip_prefix("WELCOME|").expect("session");
        write_frame(
            dropped,
            &format!(
                "REQX|{session}|reqx-drop|Unknown|file:///reqx-doc|0|6|resume|restored|1|{{canonical}}"
            ),
        )
        .expect("send");
        let _ = unsafe { CloseHandle(dropped) };
        // The server promotes the pending intent after the failed write;
        // poll the shared store deterministically.
        let mut promoted = false;
        for _ in 0..200 {
            if let Ok(store) = store.lock()
                && store.list().map_or(0, |listed| listed.len()) == 1
            {
                promoted = true;
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(promoted, "disconnect after send must promote the block");
        server.shutdown();
        let reloaded =
            DurableRecoveryStore::open_in(store.lock().expect("store lock").directory(), 8);
        assert_eq!(reloaded.list().expect("list").len(), 1);
    }

    #[test]
    fn reqx_preflight_failure_never_executes() {
        let dir = {
            let store = recovery_store_dir("reqx-poison");
            let dir = store.directory().to_path_buf();
            drop(store);
            dir
        };
        // Corrupt the state file so the store is poisoned: preflight must
        // fail closed and the handler must never run.
        let path = dir.join("recovery-state.bin");
        let mut bytes = std::fs::read(&path).expect("state");
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&path, bytes).expect("corrupt");
        let store = Arc::new(std::sync::Mutex::new(DurableRecoveryStore::open_in(
            &dir, 8,
        )));
        assert!(store.lock().expect("lock").poison().is_some());
        let (handler, calls) = counting_handler();
        let name = pipe_name("reqx-poison");
        let server =
            spawn_dummy_host_server_with_handoff(&name, 8, handler, None, Some(Arc::clone(&store)))
                .expect("server");
        let mut client = connect(&server);
        let outcome = client
            .request_with_descriptor(
                &test_descriptor("poison"),
                "Inactive",
                "{canonical}",
                Duration::from_secs(5),
            )
            .expect("fail-closed reply");
        // A poisoned store is fail-closed both at the blocked-target gate
        // and at preflight; either typed rejection proves the request never
        // became mutation-eligible.
        assert!(
            matches!(
                &outcome,
                LedgerOutcome::Definite(text)
                    if text == "rejected:RecoveryPreflightFailed"
                        || text == "rejected:TargetBlocked"
            ),
            "outcome={outcome:?}"
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0, "never executed");
        client.close();
        server.shutdown();
    }
}

#![allow(clippy::doc_markdown)]

//! M3D-20 narrow Windows named-pipe transport binding.
//!
//! This module binds the platform-neutral transport contract from
//! `zonkey-service::transport` to a localhost named pipe for Windows 11 x64.
//! One server instance hosts one dummy/test host; one client connects per
//! connection. Frames, the protocol id, the session-bound ledger, and
//! ambiguous-loss semantics all come from the existing contract; this module
//! adds only pipe I/O, a bounded read timeout, and clean teardown.
//!
//! Security posture, stated exactly: the pipe is created with default
//! security attributes (the creating process's default DACL), which on a
//! standard interactive token restricts access to the creating user,
//! administrators, and local system. No explicit per-user DACL is built, no
//! client impersonation is performed, and no client identity (PID, window
//! handle, or pipe name) is trusted. The only binding is the server-issued
//! session id delivered by the `WELCOME` handshake; it is a correlation
//! token, not cryptographic authentication. Malformed or oversized frames
//! fail closed by dropping the connection. A request whose result is lost to
//! a timeout or disconnect is the caller's `Indeterminate`; re-sending the
//! same request id replays the recorded outcome without re-execution.
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
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_CREATION_DISPOSITION, FILE_SHARE_MODE,
    FlushFileBuffers, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
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

fn next_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0u128, |since| since.as_nanos());
    format!(
        "sess-{}-{nanos}",
        SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
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
    let pipe_name = pipe_name.to_owned();
    let session_id = next_session_id();
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
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                frame_buffer,
                frame_buffer,
                1000,
                None,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let _ = ready_tx.send(());
            return;
        }
        let _ = ready_tx.send(());
        loop {
            if unsafe { ConnectNamedPipe(handle, None) }.is_err() {
                break;
            }
            serve_connection(handle, &mut endpoint, &thread_session, &handler);
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
        // Unknown well-formed payloads are ignored fail-closed: drop.
        return;
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
                windows::Win32::Foundation::GENERIC_READ.0
                    | windows::Win32::Foundation::GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                FILE_CREATION_DISPOSITION(3), // OPEN_EXISTING
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        };
        match opened {
            Ok(handle) => return Ok(handle),
            Err(error) => {
                let busy = error.code() == HRESULT(HRESULT_PIPE_BUSY);
                if busy {
                    let waited = unsafe { WaitNamedPipeW(PCWSTR(name.as_ptr()), 50) };
                    if !waited.as_bool() {
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
    use std::sync::atomic::AtomicUsize;
    use zonkey_service::transport::ambiguous_loss_outcome;

    fn pipe_name(tag: &str) -> String {
        format!(
            r"\\.\pipe\zonkey-m3d20-{tag}-{}",
            SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
        )
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
}

#![allow(clippy::doc_markdown)]

//! Endpoint discovery record (M3D-33).
//!
//! The approved startup model for this milestone is explicit and manual:
//! the operator starts `zonkey-cli serve-host-validation --pipe auto`; the
//! CLI generates the per-lifecycle nonce pipe name (M3D-29), writes this
//! small discovery record under `%LOCALAPPDATA%\ZonKey\endpoint.txt`
//! (current-user-only ACL, no secrets, no document content), and removes
//! it again on clean shutdown — but only when the record still names this
//! endpoint, so a newer endpoint is never silently deregistered. A crash
//! leaves a stale record; consumers must treat the record as a hint and
//! verify by connecting: the pipe identity is a per-lifecycle nonce, so a
//! stale name never authorizes anything.
//!
//! Record format (key=value lines, ASCII):
//! `protocol=<zonkey.host-transport/1>`
//! `pipe=<\\.\pipe\zonkey-svc-...>`
//! `pid=<process id>`
//! `started_unix_ms=<wall clock>`

use std::path::PathBuf;

/// The endpoint protocol the discovery record advertises.
pub const ENDPOINT_PROTOCOL: &str = "zonkey.host-transport/1";
/// Discovery file name inside the state directory.
const DISCOVERY_FILE: &str = "endpoint.txt";

/// Parsed discovery record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointRecord {
    pub protocol: String,
    pub pipe: String,
    pub pid: u32,
    pub started_unix_ms: u64,
}

/// The discovery directory: `%LOCALAPPDATA%\ZonKey`, overridable with
/// `ZONKEY_ENDPOINT_DIR` for isolated tests.
#[must_use]
pub fn discovery_dir() -> Option<PathBuf> {
    if let Some(override_dir) = std::env::var_os("ZONKEY_ENDPOINT_DIR") {
        return Some(PathBuf::from(override_dir));
    }
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join("ZonKey"))
}

fn render(record: &EndpointRecord) -> String {
    format!(
        "protocol={}\r\npipe={}\r\npid={}\r\nstarted_unix_ms={}\r\n",
        record.protocol, record.pipe, record.pid, record.started_unix_ms
    )
}

/// Parses record text; any malformation or unknown protocol returns None
/// (fail closed).
fn parse(text: &str) -> Option<EndpointRecord> {
    let mut protocol: Option<String> = None;
    let mut pipe: Option<String> = None;
    let mut pid: Option<u32> = None;
    let mut started: Option<u64> = None;
    for line in text.lines() {
        let (key, value) = line.split_once('=')?;
        match key {
            "protocol" => protocol = Some(value.to_owned()),
            "pipe" => pipe = Some(value.to_owned()),
            "pid" => pid = value.parse::<u32>().ok(),
            "started_unix_ms" => started = value.parse::<u64>().ok(),
            _ => return None,
        }
    }
    let record = EndpointRecord {
        protocol: protocol?,
        pipe: pipe?,
        pid: pid?,
        started_unix_ms: started?,
    };
    // Protocol/schema mismatch fails closed.
    (record.protocol == ENDPOINT_PROTOCOL).then_some(record)
}

/// Writes the discovery record with the current-user-only ACL via the
/// durable replace flow; never falls back to weaker security.
#[must_use]
pub fn write_record(record: &EndpointRecord) -> bool {
    let Some(dir) = discovery_dir() else {
        return false;
    };
    write_record_in(&dir, record)
}

/// Directory-injectable core of [`write_record`].
#[must_use]
pub fn write_record_in(dir: &std::path::Path, record: &EndpointRecord) -> bool {
    let path = dir.join(DISCOVERY_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    crate::recovery_store::durable_write_user_only(&path, render(record).as_bytes())
}

/// Reads and validates the current discovery record.
#[must_use]
pub fn read_record() -> Option<EndpointRecord> {
    let dir = discovery_dir()?;
    read_record_in(&dir)
}

/// Directory-injectable core of [`read_record`].
#[must_use]
pub fn read_record_in(dir: &std::path::Path) -> Option<EndpointRecord> {
    let bytes = std::fs::read(dir.join(DISCOVERY_FILE)).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    parse(&text)
}

/// Removes the discovery record on clean shutdown, but only when it still
/// names `pipe` — a newer endpoint's record is never deregistered by an
/// older one.
#[must_use]
pub fn remove_record(pipe: &str) -> bool {
    let Some(dir) = discovery_dir() else {
        return false;
    };
    remove_record_in(&dir, pipe)
}

/// Directory-injectable core of [`remove_record`].
#[must_use]
pub fn remove_record_in(dir: &std::path::Path, pipe: &str) -> bool {
    match read_record_in(dir) {
        Some(record) if record.pipe == pipe => {
            std::fs::remove_file(dir.join(DISCOVERY_FILE)).is_ok()
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let unique = crate::pipe_security::random_nonce_hex(8).expect("nonce");
        let dir = std::env::temp_dir().join(format!("zonkey-m3d33-{tag}-{unique}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn sample(pipe: &str) -> EndpointRecord {
        EndpointRecord {
            protocol: ENDPOINT_PROTOCOL.to_owned(),
            pipe: pipe.to_owned(),
            pid: 4242,
            started_unix_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn record_roundtrip_and_removal() {
        let dir = temp_dir("roundtrip");
        let record = sample(r"\\.\pipe\zonkey-svc-abcd");
        assert!(write_record_in(&dir, &record));
        assert_eq!(read_record_in(&dir), Some(record.clone()));
        assert!(remove_record_in(&dir, &record.pipe));
        assert_eq!(read_record_in(&dir), None);
    }

    #[test]
    fn removal_never_deregisters_a_newer_endpoint() {
        let dir = temp_dir("newer");
        let older = sample(r"\\.\pipe\zonkey-svc-old");
        let newer = sample(r"\\.\pipe\zonkey-svc-new");
        assert!(write_record_in(&dir, &older));
        assert!(write_record_in(&dir, &newer));
        // The older lifecycle shutting down must not remove the newer one.
        assert!(remove_record_in(&dir, &older.pipe));
        assert_eq!(read_record_in(&dir), Some(newer.clone()));
        assert!(remove_record_in(&dir, &newer.pipe));
    }

    #[test]
    fn malformed_or_unknown_protocol_fails_closed() {
        let dir = temp_dir("malformed");
        assert_eq!(read_record_in(&dir), None);
        let record = sample(r"\\.\pipe\zonkey-svc-x");
        assert!(write_record_in(&dir, &record));
        let path = dir.join(DISCOVERY_FILE);
        let bytes = std::fs::read(&path).expect("read");
        // Unknown protocol version in the record must fail closed.
        let corrupted = String::from_utf8(bytes.clone())
            .expect("ascii")
            .replace(ENDPOINT_PROTOCOL, "zonkey.host-transport/9");
        std::fs::write(&path, corrupted).expect("write");
        assert_eq!(read_record_in(&dir), None);
        // Truncated/garbage records fail closed.
        std::fs::write(&path, b"pipe=only").expect("write");
        assert_eq!(read_record_in(&dir), None);
    }

    #[test]
    fn record_contains_no_secrets_or_document_content() {
        let dir = temp_dir("privacy");
        let record = sample(r"\\.\pipe\zonkey-svc-0123456789abcdef");
        assert!(write_record_in(&dir, &record));
        let bytes = std::fs::read(dir.join(DISCOVERY_FILE)).expect("read");
        let text = String::from_utf8(bytes).expect("ascii");
        assert!(text.contains("pipe="));
        assert!(!text.contains("resume"));
        assert!(!text.contains("expected"));
        assert!(!text.contains("replacement"));
    }

    #[test]
    fn discovery_file_dacl_grants_only_current_user() {
        let dir = temp_dir("acl");
        let record = sample(r"\\.\pipe\zonkey-svc-acl");
        assert!(write_record_in(&dir, &record));
        let handle =
            crate::recovery_store::probe_control_handle(&dir.join(DISCOVERY_FILE)).expect("probe");
        let facts = crate::pipe_security::inspect_current_user_only_dacl(handle).expect("inspect");
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
        assert!(facts.all_allowed_aces_grant_current_user_only);
    }
}

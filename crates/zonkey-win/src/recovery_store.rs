#![allow(clippy::doc_markdown)]

//! Durable recovery-state store (M3D-31 / ADR 0035).
//!
//! One bounded, versioned state file under a Zonkey subdirectory of
//! `%LOCALAPPDATA%` persists exactly the blocked-target recovery metadata
//! defined by [`zonkey_service::recovery_codec`]: URI, UTF-16 range, salted
//! SHA-256 hashes of the expected/replacement tokens, the reconciliation
//! verdict, and a generation marker. Plaintext document text is never
//! written. Every mutation is write-through: the next registry state is
//! encoded and durably replaced *before* it becomes visible in memory, so a
//! crash can only leave the previous durable state, never a forgotten
//! block. A corrupt, truncated, oversized, or unknown-version file poisons
//! the store: every recovery operation fails closed with a typed error and
//! the file is never interpreted as empty state.
//!
//! Durability uses temp file → `FlushFileBuffers` →
//! `MoveFileExW(MOVEFILE_WRITE_THROUGH | MOVEFILE_REPLACE_EXISTING)`. Both
//! the temp and final files carry the explicit current-user-only DACL from
//! M3D-29; if that DACL cannot be built, writes fail rather than silently
//! falling back to default security.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, FILE_CREATION_DISPOSITION, FILE_SHARE_MODE,
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
use windows::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, DeleteFileW, FlushFileBuffers, MoveFileExW,
};
#[cfg(test)]
use windows::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, READ_CONTROL};
use windows::core::PCWSTR;

use zonkey_service::recovery_codec::{
    self, MAX_RECOVERY_ENTRIES, MAX_RECOVERY_STATE_BYTES, PersistedKind, PersistedTarget,
    RecoveryCodecError, SALT_BYTES,
};
use zonkey_service::transport::{
    BlockedTarget, RecoveryDescriptor, RecoveryError, RecoveryRegistry, RecoveryVerdict,
};

/// State file name inside the store directory.
const STATE_FILE_NAME: &str = "recovery-state.bin";
/// Temp file used by the replace flow; never read as state.
const TEMP_FILE_NAME: &str = "recovery-state.bin.tmp";

/// Typed fail-closed store errors surfaced to operators.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryStoreError {
    /// The durable state exists but cannot be trusted (corruption,
    /// truncation, oversize, unknown version, bad checksum). The store is
    /// poisoned until the operator repairs or removes the file.
    StateUnreadable(RecoveryCodecError),
    /// The store directory or state file could not be accessed or created.
    Unavailable,
    /// The durable write failed; the mutation was not applied in memory.
    WriteFailed,
    /// The wrapped registry rejected the operation (for example the
    /// registry is full of unresolved targets).
    Registry(RecoveryError),
}

/// Durable recovery store: an in-memory [`RecoveryRegistry`] plus the
/// in-process pending preflight intents, both mirrored to a bounded state
/// file with write-through-before-commit ordering.
pub struct DurableRecoveryStore {
    registry: RecoveryRegistry,
    pending: Vec<RecoveryDescriptor>,
    directory: PathBuf,
    salt: [u8; SALT_BYTES],
    poison: Option<RecoveryStoreError>,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn write_all(handle: HANDLE, bytes: &[u8]) -> bool {
    let mut written = 0usize;
    while written < bytes.len() {
        let chunk = &bytes[written..];
        let mut transferred = 0u32;
        let result = unsafe {
            windows::Win32::Storage::FileSystem::WriteFile(
                handle,
                Some(chunk),
                Some(&raw mut transferred),
                None,
            )
        };
        if result.is_err() || transferred == 0 {
            return false;
        }
        written += transferred as usize;
    }
    unsafe { FlushFileBuffers(handle) }.is_ok()
}

/// Creates the directory when missing with the explicit current-user-only
/// DACL; an existing directory is used as-is (its ACL is never weakened).
fn ensure_directory(directory: &Path) -> bool {
    if directory.exists() {
        return true;
    }
    let Ok(security) = crate::pipe_security::PipeSecurityAttributes::current_user_only() else {
        return false;
    };
    let text = directory.to_string_lossy().into_owned();
    let name = wide(&text);
    let created =
        unsafe { CreateDirectoryW(PCWSTR(name.as_ptr()), Some(security.as_ptr().cast_mut())) };
    created.is_ok() || directory.exists()
}

/// Durable replace flow: ACL'd temp write → flush → write-through replace.
fn durable_write(path: &Path, bytes: &[u8]) -> bool {
    let Ok(security) = crate::pipe_security::PipeSecurityAttributes::current_user_only() else {
        return false;
    };
    let temp = path.with_file_name(TEMP_FILE_NAME);
    let temp_text = temp.to_string_lossy().into_owned();
    let temp_name = wide(&temp_text);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(temp_name.as_ptr()),
            GENERIC_WRITE.0,
            FILE_SHARE_MODE(0),
            Some(security.as_ptr().cast_mut()),
            FILE_CREATION_DISPOSITION(CREATE_ALWAYS.0),
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };
    let Ok(handle) = handle else {
        return false;
    };
    let written = write_all(handle, bytes);
    let closed = unsafe { CloseHandle(handle) }.is_ok();
    if !written || !closed {
        return false;
    }
    let target_text = path.to_string_lossy().into_owned();
    let target_name = wide(&target_text);
    let moved = unsafe {
        MoveFileExW(
            PCWSTR(temp_name.as_ptr()),
            PCWSTR(target_name.as_ptr()),
            MOVEFILE_WRITE_THROUGH | MOVEFILE_REPLACE_EXISTING,
        )
    };
    moved.is_ok()
}

impl DurableRecoveryStore {
    /// Opens (or creates) the store in `directory` with a custom capacity.
    /// A corrupt existing file poisons the store instead of failing here:
    /// the endpoint keeps serving non-recovery traffic while every
    /// recovery operation fails closed with the typed reason.
    ///
    /// # Panics
    ///
    /// Never panics on the caller's thread; an invalid zero capacity is
    /// clamped to one so a usable registry always exists behind the poison
    /// gate.
    #[must_use]
    pub fn open_in(directory: &Path, capacity: usize) -> Self {
        let poisoned = |reason: RecoveryStoreError| Self {
            registry: RecoveryRegistry::new(capacity.max(1)).expect("non-zero capacity"),
            pending: Vec::new(),
            directory: directory.to_owned(),
            salt: [0; SALT_BYTES],
            poison: Some(reason),
        };
        if !ensure_directory(directory) {
            return poisoned(RecoveryStoreError::Unavailable);
        }
        let path = directory.join(STATE_FILE_NAME);
        match std::fs::read(&path) {
            Err(_) => {
                // Fresh store: a new random salt and an eagerly created
                // empty state file prove writability up front.
                let Some(salt) = crate::pipe_security::random_bytes(SALT_BYTES)
                    .ok()
                    .and_then(|bytes| <[u8; SALT_BYTES]>::try_from(bytes).ok())
                else {
                    return poisoned(RecoveryStoreError::Unavailable);
                };
                let mut store = Self {
                    registry: RecoveryRegistry::new(capacity).expect("non-zero capacity"),
                    pending: Vec::new(),
                    directory: directory.to_owned(),
                    salt,
                    poison: None,
                };
                match recovery_codec::encode(&salt, &[]) {
                    Ok(bytes) if durable_write(&path, &bytes) => {}
                    _ => store.poison = Some(RecoveryStoreError::Unavailable),
                }
                store
            }
            Ok(bytes) => {
                if bytes.len() > MAX_RECOVERY_STATE_BYTES {
                    return poisoned(RecoveryStoreError::StateUnreadable(
                        RecoveryCodecError::OversizedFile,
                    ));
                }
                match recovery_codec::decode(&bytes) {
                    Err(error) => poisoned(RecoveryStoreError::StateUnreadable(error)),
                    Ok((salt, records)) => {
                        // Pending preflight intents that survived a restart
                        // were never definitively resolved: RecoveryRegistry
                        // reloads them as blocked (recovery-required), so a
                        // restart can never look silently clean.
                        match RecoveryRegistry::restore(capacity, salt, records) {
                            Ok(registry) => Self {
                                registry,
                                pending: Vec::new(),
                                directory: directory.to_owned(),
                                salt,
                                poison: None,
                            },
                            Err(_) => poisoned(RecoveryStoreError::StateUnreadable(
                                RecoveryCodecError::TooManyEntries,
                            )),
                        }
                    }
                }
            }
        }
    }

    /// Opens the default store under
    /// `%LOCALAPPDATA%\ZonKey` with the ADR 0035 capacity.
    #[must_use]
    pub fn open_default() -> Self {
        let Some(base) = std::env::var_os("LOCALAPPDATA") else {
            return Self::open_in(Path::new("/."), MAX_RECOVERY_ENTRIES);
        };
        Self::open_in(&Path::new(&base).join("ZonKey"), MAX_RECOVERY_ENTRIES)
    }

    /// The file salt in use (test/diagnostic surface).
    #[must_use]
    pub fn salt(&self) -> [u8; SALT_BYTES] {
        self.salt
    }

    /// The store directory (test/diagnostic surface).
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The typed poison reason when the durable state cannot be trusted.
    #[must_use]
    pub fn poison(&self) -> Option<&RecoveryStoreError> {
        self.poison.as_ref()
    }

    fn state_path(&self) -> PathBuf {
        self.directory.join(STATE_FILE_NAME)
    }

    /// Encodes the full durable state: blocked targets plus pending
    /// preflight intents, plaintext hashed with the file salt.
    fn snapshot_records(
        registry: &RecoveryRegistry,
        pending: &[RecoveryDescriptor],
        salt: &[u8; SALT_BYTES],
    ) -> Vec<PersistedTarget> {
        let mut records: Vec<PersistedTarget> = registry
            .list()
            .into_iter()
            .map(|target| into_persisted(&target, salt, OPERATOR_REQUEST_ID))
            .collect();
        records.extend(pending.iter().map(|descriptor| PersistedTarget {
            uri: descriptor.uri.clone(),
            range: descriptor.range,
            expected_hash: recovery_codec::salted_hash(salt, &descriptor.expected),
            replacement_hash: recovery_codec::salted_hash(salt, &descriptor.replacement),
            kind: PersistedKind::Pending,
            generation: descriptor.generation,
            request_id: descriptor.request_id.clone(),
        }));
        records
    }

    /// Runs one mutation write-through: clone → mutate → encode → durable
    /// replace → commit. A failed durable write leaves the live state
    /// untouched.
    fn mutate_state<T>(
        &mut self,
        operation: impl FnOnce(
            &mut RecoveryRegistry,
            &mut Vec<RecoveryDescriptor>,
        ) -> Result<T, RecoveryError>,
    ) -> Result<T, RecoveryStoreError> {
        if let Some(poison) = self.poison.clone() {
            return Err(poison);
        }
        let mut next_registry = self.registry.clone();
        let mut next_pending = self.pending.clone();
        let result = operation(&mut next_registry, &mut next_pending)
            .map_err(RecoveryStoreError::Registry)?;
        let salt = self.salt;
        let records = Self::snapshot_records(&next_registry, &next_pending, &salt);
        let Ok(bytes) = recovery_codec::encode(&salt, &records) else {
            return Err(RecoveryStoreError::WriteFailed);
        };
        if !durable_write(&self.state_path(), &bytes) {
            return Err(RecoveryStoreError::WriteFailed);
        }
        self.registry = next_registry;
        self.pending = next_pending;
        Ok(result)
    }

    /// Lists blocked targets; fails closed while poisoned.
    ///
    /// # Errors
    ///
    /// Returns the poison reason when the durable state is unreadable.
    pub fn list(&self) -> Result<Vec<BlockedTarget>, RecoveryStoreError> {
        match &self.poison {
            Some(poison) => Err(poison.clone()),
            None => Ok(self.registry.list()),
        }
    }

    /// Number of in-process pending preflight intents (bounded, shared
    /// capacity with blocked targets).
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// True when the logical target is blocked (no mutation request may
    /// proceed for it); fails closed (returns true) while poisoned or
    /// salt-less.
    #[must_use]
    pub fn target_blocked(&self, uri: &str, expected: &str) -> bool {
        match &self.poison {
            Some(_) => true,
            None => self.registry.is_blocked(uri, expected),
        }
    }

    /// Durable preflight (M3D-31 / ADR 0036): persists a PendingRecovery
    /// intent before the carrying request may proceed. Only after durable
    /// success may the caller execute; a definitive no-mutation rejection
    /// later removes it with [`Self::clear_pending`], and any uncertain
    /// outcome promotes it with [`Self::promote_pending`]. A crash at any
    /// point reloads the pending record as a blocked target.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryStoreError::Registry`] with
    /// [`RecoveryError::RegistryFull`] when the shared bound is reached, or
    /// the poison/write reasons.
    pub fn begin_pending(
        &mut self,
        descriptor: &RecoveryDescriptor,
    ) -> Result<(), RecoveryStoreError> {
        self.mutate_state(|registry, pending| {
            if pending
                .iter()
                .any(|existing| existing.request_id == descriptor.request_id)
            {
                return Ok(());
            }
            if pending.len() + registry.list().len() >= registry_capacity(registry) {
                return Err(RecoveryError::RegistryFull);
            }
            pending.push(descriptor.clone());
            Ok(())
        })
    }

    /// Removes a pending intent after a definitive rejection with no
    /// mutation possibility. Returns whether a record was removed.
    ///
    /// # Errors
    ///
    /// Returns the poison/write reasons.
    pub fn clear_pending(&mut self, request_id: &str) -> Result<bool, RecoveryStoreError> {
        self.mutate_state(|_registry, pending| {
            let before = pending.len();
            pending.retain(|existing| existing.request_id != request_id);
            Ok(before != pending.len())
        })
    }

    /// Promotes a pending intent to a recovery-required blocked target
    /// after an Applied, ambiguous, lost-response, disconnect, or otherwise
    /// uncertain outcome. Returns whether a record was promoted.
    ///
    /// # Errors
    ///
    /// Returns the poison/write reasons.
    pub fn promote_pending(
        &mut self,
        session_id: &str,
        request_id: &str,
    ) -> Result<bool, RecoveryStoreError> {
        self.mutate_state(|registry, pending| {
            let Some(index) = pending
                .iter()
                .position(|existing| existing.request_id == request_id)
            else {
                return Ok(false);
            };
            let descriptor = pending.remove(index);
            let blocked = registry
                .block(
                    session_id,
                    &descriptor.uri,
                    &descriptor.expected,
                    &descriptor.replacement,
                    descriptor.range,
                )
                .is_ok();
            Ok(blocked)
        })
    }

    /// Durable block; see [`RecoveryRegistry::block`].
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryStoreError::Registry`] with
    /// [`RecoveryError::RegistryFull`] when the registry is full of
    /// unresolved targets, or the poison/write reasons.
    pub fn block(
        &mut self,
        session_id: &str,
        uri: &str,
        expected: &str,
        replacement: &str,
        range: (usize, usize),
    ) -> Result<Option<String>, RecoveryStoreError> {
        self.mutate_state(|registry, _pending| {
            registry.block(session_id, uri, expected, replacement, range)
        })
    }

    /// Durable reconciliation; see [`RecoveryRegistry::reconcile`].
    ///
    /// # Errors
    ///
    /// Returns the registry or store reasons.
    pub fn reconcile(
        &mut self,
        session_id: &str,
        uri: &str,
        expected: &str,
        live_range_text: &str,
    ) -> Result<RecoveryVerdict, RecoveryStoreError> {
        self.mutate_state(|registry, _pending| {
            registry.reconcile(session_id, uri, expected, live_range_text)
        })
    }

    /// Durable owner acknowledgement; see [`RecoveryRegistry::acknowledge`].
    ///
    /// # Errors
    ///
    /// Returns the registry or store reasons.
    pub fn acknowledge(
        &mut self,
        session_id: &str,
        uri: &str,
        expected: &str,
    ) -> Result<(), RecoveryStoreError> {
        self.mutate_state(|registry, _pending| registry.acknowledge(session_id, uri, expected))
    }
}

/// Placeholder request id for operator-created blocked records; only
/// pending preflight intents carry a real request id.
const OPERATOR_REQUEST_ID: &str = "operator";

/// The registry's configured capacity for bound checks.
fn registry_capacity(registry: &RecoveryRegistry) -> usize {
    registry.capacity()
}

/// Converts one in-memory blocked target into its persisted metadata form;
/// plaintext tokens are salted-hashed, hashed entries are reused verbatim.
fn into_persisted(
    target: &BlockedTarget,
    salt: &[u8; SALT_BYTES],
    request_id: &str,
) -> PersistedTarget {
    let expected_hash = match &target.expected {
        zonkey_service::transport::RecoveryText::Plain(text) => {
            recovery_codec::salted_hash(salt, text)
        }
        zonkey_service::transport::RecoveryText::Hashed(hash) => *hash,
    };
    let replacement_hash = match &target.replacement {
        zonkey_service::transport::RecoveryText::Plain(text) => {
            recovery_codec::salted_hash(salt, text)
        }
        zonkey_service::transport::RecoveryText::Hashed(hash) => *hash,
    };
    PersistedTarget {
        uri: target.uri.clone(),
        range: target.range,
        expected_hash,
        replacement_hash,
        kind: PersistedKind::Blocked {
            verdict: target.state.map(|(verdict, _)| verdict),
        },
        generation: target.generation,
        request_id: request_id.to_owned(),
    }
}

/// Removes a leftover temp file left by a crash between write and replace;
/// never touches the state file itself.
#[must_use]
pub fn cleanup_temp_file(directory: &Path) -> bool {
    let temp = directory.join(TEMP_FILE_NAME);
    let text = temp.to_string_lossy().into_owned();
    let name = wide(&text);
    let removed = unsafe { DeleteFileW(PCWSTR(name.as_ptr())) };
    removed.is_ok() || !temp.exists()
}

/// Opens a READ_CONTROL probe handle for ACL inspection in tests.
#[cfg(test)]
pub(crate) fn probe_control_handle(path: &Path) -> Option<HANDLE> {
    let text = path.to_string_lossy().into_owned();
    let name = wide(&text);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            READ_CONTROL.0,
            FILE_SHARE_MODE(0),
            None,
            FILE_CREATION_DISPOSITION(3), // OPEN_EXISTING
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    };
    handle.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zonkey_service::transport::RecoveryText;

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = crate::pipe_security::random_nonce_hex(8).expect("nonce");
        let base = std::env::temp_dir();
        let dir = base.join(format!("zonkey-m3d31-{tag}-{unique}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn clean_create_then_reload_empty() {
        let dir = temp_dir("clean");
        let store = DurableRecoveryStore::open_in(&dir, 8);
        assert!(store.poison().is_none());
        assert!(store.list().expect("list").is_empty());
        assert!(dir.join(STATE_FILE_NAME).exists());
        let reloaded = DurableRecoveryStore::open_in(&dir, 8);
        assert!(reloaded.poison().is_none());
        assert!(reloaded.list().expect("list").is_empty());
        assert_ne!(store.salt(), [0u8; SALT_BYTES]);
    }

    #[test]
    fn new_file_salt_differs_per_directory() {
        let first = temp_dir("salt-a");
        let second = temp_dir("salt-b");
        let a = DurableRecoveryStore::open_in(&first, 8);
        let b = DurableRecoveryStore::open_in(&second, 8);
        assert_ne!(a.salt(), b.salt());
    }

    #[test]
    fn block_survives_restart_with_rebind_reconcile_and_ack() {
        let dir = temp_dir("restart");
        let mut store = DurableRecoveryStore::open_in(&dir, 8);
        store
            .block("sess-old", "file:///a", "resume", "restored", (0, 6))
            .expect("block");
        // Restart: a new store instance is a new runtime identity.
        let mut reloaded = DurableRecoveryStore::open_in(&dir, 8);
        assert!(reloaded.poison().is_none());
        let listed = reloaded.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "");
        assert_eq!(listed[0].expected.display(), "<hashed>");
        // The first valid operator action rebinds to the new session and
        // the verdict comes from the salted-hash comparison.
        assert_eq!(
            reloaded.reconcile("sess-new", "file:///a", "resume", "restored"),
            Ok(RecoveryVerdict::AppliedAcknowledged)
        );
        assert_eq!(
            reloaded.acknowledge("sess-new", "file:///a", "resume"),
            Ok(())
        );
        // The acknowledgement is durable: a further restart starts empty.
        let settled = DurableRecoveryStore::open_in(&dir, 8);
        assert!(settled.list().expect("list").is_empty());
    }

    #[test]
    fn every_verdict_and_state_persists() {
        for (live, verdict) in [
            ("restored", RecoveryVerdict::AppliedAcknowledged),
            ("resume", RecoveryVerdict::NotApplied),
            ("mangled!", RecoveryVerdict::ConflictHumanReview),
        ] {
            let dir = temp_dir("verdicts");
            let mut store = DurableRecoveryStore::open_in(&dir, 8);
            store
                .block("sess-1", "file:///a", "resume", "restored", (0, 6))
                .expect("block");
            assert_eq!(
                store.reconcile("sess-1", "file:///a", "resume", live),
                Ok(verdict)
            );
            let mut reloaded = DurableRecoveryStore::open_in(&dir, 8);
            let listed = reloaded.list().expect("list");
            assert_eq!(listed.len(), 1, "blocked target survives restart");
            // The recorded verdict replays without re-evaluating.
            assert_eq!(
                reloaded.reconcile("sess-9", "file:///a", "resume", "anything-else"),
                Ok(verdict)
            );
            assert_eq!(
                reloaded.acknowledge("sess-9", "file:///a", "resume"),
                Ok(())
            );
        }
    }

    #[test]
    fn corrupt_truncated_and_oversized_files_fail_closed() {
        let dir = temp_dir("corrupt");
        let mut store = DurableRecoveryStore::open_in(&dir, 8);
        store
            .block("sess-1", "file:///a", "resume", "restored", (0, 6))
            .expect("block");
        let path = dir.join(STATE_FILE_NAME);
        let original = std::fs::read(&path).expect("state bytes");

        // Single flipped byte => checksum mismatch, poisoned store.
        let mut corrupted = original.clone();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0x01;
        std::fs::write(&path, corrupted).expect("write corrupt");
        let mut poisoned = DurableRecoveryStore::open_in(&dir, 8);
        assert_eq!(
            poisoned.poison(),
            Some(&RecoveryStoreError::StateUnreadable(
                RecoveryCodecError::ChecksumMismatch
            ))
        );
        assert!(poisoned.list().is_err());
        assert!(poisoned.block("s", "file:///b", "t", "r", (0, 1)).is_err());
        // The corrupt file is never silently rewritten as empty.
        assert_eq!(
            std::fs::read(&path).expect("still there").len(),
            original.len()
        );

        // Truncation => fail closed.
        std::fs::write(&path, &original[..original.len() - 5]).expect("truncate");
        let truncated = DurableRecoveryStore::open_in(&dir, 8);
        assert!(matches!(
            truncated.poison(),
            Some(RecoveryStoreError::StateUnreadable(_))
        ));

        // Oversized file => fail closed.
        let mut big = original.clone();
        big.extend(std::iter::repeat_n(0u8, MAX_RECOVERY_STATE_BYTES + 1));
        std::fs::write(&path, big).expect("oversize");
        let oversized = DurableRecoveryStore::open_in(&dir, 8);
        assert_eq!(
            oversized.poison(),
            Some(&RecoveryStoreError::StateUnreadable(
                RecoveryCodecError::OversizedFile
            ))
        );
    }

    #[test]
    fn bad_magic_and_version_fail_closed() {
        let dir = temp_dir("magic");
        let mut store = DurableRecoveryStore::open_in(&dir, 8);
        store
            .block("sess-1", "file:///a", "resume", "restored", (0, 6))
            .expect("block");
        let path = dir.join(STATE_FILE_NAME);
        let original = std::fs::read(&path).expect("state bytes");

        let mut bad_magic = original.clone();
        bad_magic[0] = b'X';
        std::fs::write(&path, bad_magic).expect("write");
        assert_eq!(
            DurableRecoveryStore::open_in(&dir, 8).poison(),
            Some(&RecoveryStoreError::StateUnreadable(
                RecoveryCodecError::BadMagic
            ))
        );

        let mut bad_version = original.clone();
        bad_version[8] = 0x7F;
        std::fs::write(&path, bad_version).expect("write");
        assert_eq!(
            DurableRecoveryStore::open_in(&dir, 8).poison(),
            Some(&RecoveryStoreError::StateUnreadable(
                RecoveryCodecError::UnknownVersion
            ))
        );
    }

    #[test]
    fn no_plaintext_tokens_on_disk() {
        let dir = temp_dir("privacy");
        let mut store = DurableRecoveryStore::open_in(&dir, 8);
        store
            .block("sess-1", "file:///a", "resume", "restored", (0, 6))
            .expect("block");
        store
            .reconcile("sess-1", "file:///a", "resume", "restored")
            .expect("reconcile");
        let bytes = std::fs::read(dir.join(STATE_FILE_NAME)).expect("state");
        assert!(!bytes.windows(6).any(|window| window == b"resume"));
        assert!(!bytes.windows(8).any(|window| window == b"restored"));
        assert!(bytes.windows(9).any(|window| window == b"file:///a"));
    }

    #[test]
    fn encoding_is_deterministic_within_salt_across_restarts() {
        let dir = temp_dir("determinism");
        let mut store = DurableRecoveryStore::open_in(&dir, 8);
        store
            .block("sess-1", "file:///a", "resume", "restored", (0, 6))
            .expect("block");
        let path = dir.join(STATE_FILE_NAME);
        let first = std::fs::read(&path).expect("bytes");
        // Re-blocking the same target refreshes in memory and rewrites the
        // same logical state; within one salt the encoding is stable.
        store
            .block("sess-1", "file:///a", "resume", "restored", (0, 6))
            .expect("re-block");
        let second = std::fs::read(&path).expect("bytes");
        assert_eq!(first, second);
    }

    #[test]
    fn full_unresolved_registry_rejects_new_blocks_durably() {
        let dir = temp_dir("full");
        let mut store = DurableRecoveryStore::open_in(&dir, 2);
        store
            .block("s", "file:///a", "t1", "r1", (0, 2))
            .expect("block a");
        store
            .block("s", "file:///b", "t2", "r2", (0, 2))
            .expect("block b");
        assert_eq!(
            store.block("s", "file:///c", "t3", "r3", (0, 2)),
            Err(RecoveryStoreError::Registry(RecoveryError::RegistryFull))
        );
        // The rejection left the durable state untouched.
        let reloaded = DurableRecoveryStore::open_in(&dir, 2);
        assert_eq!(reloaded.list().expect("list").len(), 2);
    }

    #[test]
    fn leftover_temp_file_is_ignored_and_reused() {
        let dir = temp_dir("crash-tmp");
        // Simulate a crash between temp write and replace.
        std::fs::write(dir.join(TEMP_FILE_NAME), b"torn garbage").expect("junk temp");
        let mut store = DurableRecoveryStore::open_in(&dir, 8);
        assert!(store.poison().is_none());
        assert!(store.list().expect("list").is_empty());
        store
            .block("s", "file:///a", "t", "r", (0, 1))
            .expect("block");
        assert!(!dir.join(TEMP_FILE_NAME).exists(), "temp replaced");
        let reloaded = DurableRecoveryStore::open_in(&dir, 8);
        assert_eq!(reloaded.list().expect("list").len(), 1);
        assert!(cleanup_temp_file(&dir));
    }

    #[test]
    fn state_file_dacl_grants_only_current_user() {
        let dir = temp_dir("acl");
        let mut store = DurableRecoveryStore::open_in(&dir, 8);
        store
            .block("s", "file:///a", "t", "r", (0, 1))
            .expect("block");
        let handle = probe_control_handle(&dir.join(STATE_FILE_NAME)).expect("probe");
        let facts = crate::pipe_security::inspect_current_user_only_dacl(handle).expect("inspect");
        let _ = unsafe { CloseHandle(handle) };
        assert!(facts.ace_count >= 1);
        assert!(
            facts.all_allowed_aces_grant_current_user_only,
            "facts={facts:?}"
        );
        assert!(!facts.has_deny_ace);
    }

    #[test]
    fn restored_entries_use_hashed_text() {
        let dir = temp_dir("hashed");
        let mut store = DurableRecoveryStore::open_in(&dir, 8);
        store
            .block("s", "file:///a", "resume", "restored", (0, 6))
            .expect("block");
        let reloaded = DurableRecoveryStore::open_in(&dir, 8);
        let listed = reloaded.list().expect("list");
        assert!(matches!(listed[0].expected, RecoveryText::Hashed(_)));
        assert!(matches!(listed[0].replacement, RecoveryText::Hashed(_)));
    }

    fn descriptor(tag: &str) -> zonkey_service::transport::RecoveryDescriptor {
        zonkey_service::transport::RecoveryDescriptor {
            request_id: format!("req-{tag}"),
            uri: "file:///doc".to_owned(),
            range: (0, 6),
            expected: "resume".to_owned(),
            replacement: "restored".to_owned(),
            generation: 5,
        }
    }

    #[test]
    fn crash_after_preflight_reloads_pending_as_blocked() {
        let dir = temp_dir("preflight");
        let mut store = DurableRecoveryStore::open_in(&dir, 8);
        store.begin_pending(&descriptor("p1")).expect("preflight");
        assert_eq!(store.pending_len(), 1);
        // Simulated crash: a fresh store instance reloads from disk only.
        let reloaded = DurableRecoveryStore::open_in(&dir, 8);
        assert!(reloaded.poison().is_none());
        let listed = reloaded.list().expect("list");
        assert_eq!(listed.len(), 1, "pending must reload as recovery-required");
        assert_eq!(listed[0].uri, "file:///doc");
        assert_eq!(listed[0].session_id, "");
        // No new request for the same logical target may proceed.
        assert!(reloaded.target_blocked("file:///doc", "resume"));
    }

    #[test]
    fn clear_pending_after_definitive_rejection_leaves_clean_state() {
        let dir = temp_dir("clear");
        let mut store = DurableRecoveryStore::open_in(&dir, 8);
        store.begin_pending(&descriptor("c1")).expect("preflight");
        assert!(store.clear_pending("req-c1").expect("clear"));
        assert_eq!(store.pending_len(), 0);
        assert!(!store.target_blocked("file:///doc", "resume"));
        let reloaded = DurableRecoveryStore::open_in(&dir, 8);
        assert!(reloaded.list().expect("list").is_empty());
        assert!(!reloaded.target_blocked("file:///doc", "resume"));
    }

    #[test]
    fn promote_pending_blocks_target_durably() {
        let dir = temp_dir("promote");
        let mut store = DurableRecoveryStore::open_in(&dir, 8);
        store.begin_pending(&descriptor("m1")).expect("preflight");
        assert!(store.promote_pending("sess-1", "req-m1").expect("promote"));
        assert_eq!(store.pending_len(), 0);
        assert!(store.target_blocked("file:///doc", "resume"));
        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "sess-1");
        // Reconcile + ack unblocks, durably.
        assert_eq!(
            store.reconcile("sess-1", "file:///doc", "resume", "restored"),
            Ok(zonkey_service::transport::RecoveryVerdict::AppliedAcknowledged)
        );
        assert!(store.acknowledge("sess-1", "file:///doc", "resume").is_ok());
        let reloaded = DurableRecoveryStore::open_in(&dir, 8);
        assert!(reloaded.list().expect("list").is_empty());
        assert!(!reloaded.target_blocked("file:///doc", "resume"));
    }

    #[test]
    fn pending_shares_the_bounded_capacity() {
        let dir = temp_dir("pending-cap");
        let mut store = DurableRecoveryStore::open_in(&dir, 2);
        store.begin_pending(&descriptor("a")).expect("preflight a");
        store.begin_pending(&descriptor("b")).expect("preflight b");
        assert_eq!(
            store.begin_pending(&descriptor("c")),
            Err(RecoveryStoreError::Registry(RecoveryError::RegistryFull))
        );
        // The rejection left nothing durable for the third intent.
        let reloaded = DurableRecoveryStore::open_in(&dir, 2);
        assert_eq!(reloaded.list().expect("list").len(), 2);
    }

    #[test]
    fn poisoned_store_blocks_every_target_fail_closed() {
        let dir = temp_dir("poison");
        {
            let mut store = DurableRecoveryStore::open_in(&dir, 8);
            store.begin_pending(&descriptor("x")).expect("preflight");
        }
        let path = dir.join(STATE_FILE_NAME);
        let mut bytes = std::fs::read(&path).expect("state");
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&path, bytes).expect("corrupt");
        let mut poisoned = DurableRecoveryStore::open_in(&dir, 8);
        assert!(poisoned.poison().is_some());
        // Fail closed: unknown state must never look permissive.
        assert!(poisoned.target_blocked("file:///doc", "resume"));
        assert!(poisoned.list().is_err());
        assert!(poisoned.begin_pending(&descriptor("y")).is_err());
    }
}

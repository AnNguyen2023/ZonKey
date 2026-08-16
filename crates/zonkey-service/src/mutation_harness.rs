//! M3D-26 controlled-mutation contract harness — test evidence only.
//!
//! A deterministic dummy cooperating host exercising the ADR 0030 semantics:
//! the fixed-order eligibility validator, one atomic host-owned
//! compare-and-replace with in-transaction re-read and post-commit
//! verification, the bounded idempotent ledger (reused from
//! `zonkey-service::transport`), and the Indeterminate reconciliation
//! workflow with explicit owner acknowledgement. The synthetic composition
//! state and `CAP_COMPOSITION_PROOF` live only here; nothing in this module
//! is compiled into production or reaches the real VS Code binding, which
//! still reports `CompositionUnknown`.

use std::collections::HashMap;

use crate::transport::{BoundedRequestLedger, LedgerOutcome, RequestDisposition};

pub const CAP_SNAPSHOT: u8 = 0b0000_0001;
pub const CAP_COMPARE_AND_REPLACE: u8 = 0b0000_0010;
pub const CAP_UTF16_UNITS: u8 = 0b0000_0100;
pub const CAP_IDEMPOTENT_REQUESTS: u8 = 0b0000_1000;
/// Synthetic proof capability; test hosts only.
pub const CAP_COMPOSITION_PROOF: u8 = 0b0001_0000;
pub const CAP_ALL_WITH_COMPOSITION: u8 = CAP_SNAPSHOT
    | CAP_COMPARE_AND_REPLACE
    | CAP_UTF16_UNITS
    | CAP_IDEMPOTENT_REQUESTS
    | CAP_COMPOSITION_PROOF;

pub const MUTATION_PROTOCOL_ID: &str = "zonkey.mutation-harness/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HarnessComposition {
    Inactive,
    Active,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HarnessSecure {
    KnownNonSecure,
    Secure,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HarnessSession {
    SupportedLocal,
    UnsupportedRemote,
    Unknown,
}

/// Transaction behavior injection for the dummy host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionMode {
    Normal,
    /// The host refuses the transaction before applying anything.
    Refuse,
    /// The outcome is lost before the commit.
    Lost,
    /// The commit happens but the response is lost afterwards.
    LostAfterCommit,
}

/// Deterministic dummy cooperating host. UTF-16 offsets throughout.
pub struct HarnessHost {
    pub host_id: String,
    pub session_id: String,
    pub uri: String,
    pub epoch: u64,
    pub editor_id: u64,
    pub version: u64,
    pub caret: usize,
    /// Non-zero selection length rejects as `SelectionNotEmpty`.
    pub selection_span: usize,
    pub secure: HarnessSecure,
    pub session_state: HarnessSession,
    pub composition: HarnessComposition,
    pub capabilities: u8,
    pub transaction_mode: TransactionMode,
    /// Race injection: replace a UTF-16 range before the in-transaction
    /// re-read (bumps the version).
    pub pre_transaction_change: Option<(usize, usize, &'static str)>,
    /// Race injection: replace a UTF-16 range after the commit, before
    /// post-commit verification (bumps the version again).
    pub post_commit_change: Option<(usize, usize, &'static str)>,
    text: String,
}

impl HarnessHost {
    pub fn new(text: &str) -> Self {
        Self {
            host_id: "harness-host".to_owned(),
            session_id: "harness-session".to_owned(),
            uri: "file:///harness/doc.txt".to_owned(),
            epoch: 1,
            editor_id: 1,
            version: 1,
            caret: text.encode_utf16().count(),
            selection_span: 0,
            secure: HarnessSecure::KnownNonSecure,
            session_state: HarnessSession::SupportedLocal,
            composition: HarnessComposition::Inactive,
            capabilities: CAP_ALL_WITH_COMPOSITION,
            transaction_mode: TransactionMode::Normal,
            pre_transaction_change: None,
            post_commit_change: None,
            text: text.to_owned(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn utf16_length(&self) -> usize {
        self.text.encode_utf16().count()
    }

    fn utf16_slice(&self, start: usize, end: usize) -> String {
        String::from_utf16_lossy(
            &self
                .text
                .encode_utf16()
                .skip(start)
                .take(end.saturating_sub(start))
                .collect::<Vec<_>>(),
        )
    }

    fn replace_utf16(&mut self, start: usize, end: usize, replacement: &str) -> bool {
        let units: Vec<u16> = self.text.encode_utf16().collect();
        if start > end || end > units.len() {
            return false;
        }
        let mut next: Vec<u16> = Vec::with_capacity(units.len());
        next.extend_from_slice(&units[..start]);
        next.extend(replacement.encode_utf16());
        next.extend_from_slice(&units[end..]);
        match String::from_utf16(&next) {
            Ok(text) => {
                self.text = text;
                true
            }
            Err(_) => false,
        }
    }

    fn inject_change(&mut self, change: Option<(usize, usize, &'static str)>) {
        if let Some((start, end, text)) = change
            && self.replace_utf16(start, end, text)
        {
            self.version = self.version.saturating_add(1);
        }
    }
}

/// One compare-and-replace request against the harness host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutationRequest {
    pub protocol_id: String,
    pub host_id: String,
    pub session_id: String,
    pub request_id: String,
    pub uri: String,
    pub epoch: u64,
    pub editor_id: u64,
    pub revision: u64,
    pub range_start: usize,
    pub range_end: usize,
    pub expected_text: String,
    pub replacement: String,
    pub caret: usize,
    pub composition: HarnessComposition,
    pub capabilities: u8,
}

impl MutationRequest {
    /// Builds a request that matches the current host state: the expected
    /// text sits immediately before the caret.
    pub fn matching(host: &HarnessHost, expected: &str, replacement: &str) -> Self {
        let caret = host.caret;
        Self {
            protocol_id: MUTATION_PROTOCOL_ID.to_owned(),
            host_id: host.host_id.clone(),
            session_id: host.session_id.clone(),
            request_id: "req-1".to_owned(),
            uri: host.uri.clone(),
            epoch: host.epoch,
            editor_id: host.editor_id,
            revision: host.version,
            range_start: caret - expected.encode_utf16().count(),
            range_end: caret,
            expected_text: expected.to_owned(),
            replacement: replacement.to_owned(),
            caret,
            composition: HarnessComposition::Inactive,
            capabilities: host.capabilities,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    ProtocolMismatch,
    HostIdentityMismatch,
    SessionMismatch,
    TargetIdentityMismatch,
    RangeMismatch,
    CaretMismatch,
    SelectionNotEmpty,
    TextMismatch,
    RevisionMismatch,
    SecureTarget,
    SecureUnknown,
    CompositionActive,
    CompositionUnknown,
    UnsupportedSession,
    SessionUnknown,
    CapabilityMismatch,
    RequestIdInvalid,
    RequestIdReuse,
    EditTransactionRefused,
    RevisionOverflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndeterminateReason {
    EditOutcomeLost,
    AmbiguousCommit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MutationOutcome {
    Applied {
        new_revision: u64,
    },
    Rejected {
        reason: RejectReason,
    },
    Indeterminate {
        reason: IndeterminateReason,
    },
    /// A prior Indeterminate blocks this logical target until
    /// reconciliation plus explicit owner acknowledgement.
    BlockedPendingReconciliation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconciliationVerdict {
    AppliedAcknowledged,
    NotApplied,
    ConflictHumanReview,
}

enum TargetState {
    Indeterminate,
    Reconciled { acknowledged: bool },
}

/// The harness under test: one dummy host, one bounded ledger, one
/// blocked-target registry keyed by document URI + expected rendered text.
pub struct MutationHarness {
    host: HarnessHost,
    ledger: BoundedRequestLedger,
    targets: HashMap<String, TargetState>,
}

fn logical_target(uri: &str, expected: &str) -> String {
    format!("{uri}\u{0}{expected}")
}

fn canonical(request: &MutationRequest) -> String {
    [
        request.protocol_id.clone(),
        request.host_id.clone(),
        request.session_id.clone(),
        request.request_id.clone(),
        request.uri.clone(),
        request.epoch.to_string(),
        request.editor_id.to_string(),
        request.revision.to_string(),
        request.range_start.to_string(),
        request.range_end.to_string(),
        request.expected_text.clone(),
        request.replacement.clone(),
        request.caret.to_string(),
        format!("{:?}", request.composition),
        request.capabilities.to_string(),
    ]
    .join("\u{1}")
}

fn encode(outcome: &MutationOutcome) -> LedgerOutcome {
    match outcome {
        MutationOutcome::Applied { new_revision } => {
            LedgerOutcome::Definite(format!("applied:{new_revision}"))
        }
        MutationOutcome::Rejected { reason } => {
            LedgerOutcome::Definite(format!("rejected:{reason:?}"))
        }
        MutationOutcome::Indeterminate { reason } => {
            LedgerOutcome::Ambiguous(format!("indeterminate:{reason:?}"))
        }
        MutationOutcome::BlockedPendingReconciliation => {
            LedgerOutcome::Definite("blocked".to_owned())
        }
    }
}

fn decode(outcome: &LedgerOutcome) -> MutationOutcome {
    match outcome {
        LedgerOutcome::Definite(text) => {
            if let Some(value) = text.strip_prefix("applied:") {
                MutationOutcome::Applied {
                    new_revision: value.parse().unwrap_or(u64::MAX),
                }
            } else if let Some(value) = text.strip_prefix("rejected:") {
                MutationOutcome::Rejected {
                    reason: match value {
                        "ProtocolMismatch" => RejectReason::ProtocolMismatch,
                        "HostIdentityMismatch" => RejectReason::HostIdentityMismatch,
                        "SessionMismatch" => RejectReason::SessionMismatch,
                        "TargetIdentityMismatch" => RejectReason::TargetIdentityMismatch,
                        "RangeMismatch" => RejectReason::RangeMismatch,
                        "CaretMismatch" => RejectReason::CaretMismatch,
                        "SelectionNotEmpty" => RejectReason::SelectionNotEmpty,
                        "RevisionMismatch" => RejectReason::RevisionMismatch,
                        "SecureTarget" => RejectReason::SecureTarget,
                        "SecureUnknown" => RejectReason::SecureUnknown,
                        "CompositionActive" => RejectReason::CompositionActive,
                        "CompositionUnknown" => RejectReason::CompositionUnknown,
                        "UnsupportedSession" => RejectReason::UnsupportedSession,
                        "SessionUnknown" => RejectReason::SessionUnknown,
                        "CapabilityMismatch" => RejectReason::CapabilityMismatch,
                        "RequestIdInvalid" => RejectReason::RequestIdInvalid,
                        "RequestIdReuse" => RejectReason::RequestIdReuse,
                        "EditTransactionRefused" => RejectReason::EditTransactionRefused,
                        "RevisionOverflow" => RejectReason::RevisionOverflow,
                        // Unknown encodings fail closed as a text mismatch.
                        _ => RejectReason::TextMismatch,
                    },
                }
            } else if text == "blocked" {
                MutationOutcome::BlockedPendingReconciliation
            } else {
                MutationOutcome::Rejected {
                    reason: RejectReason::TextMismatch,
                }
            }
        }
        LedgerOutcome::Ambiguous(text) => MutationOutcome::Indeterminate {
            reason: match text.strip_prefix("indeterminate:") {
                Some("EditOutcomeLost") => IndeterminateReason::EditOutcomeLost,
                _ => IndeterminateReason::AmbiguousCommit,
            },
        },
    }
}

impl MutationHarness {
    pub fn new(host: HarnessHost) -> Self {
        Self {
            host,
            ledger: BoundedRequestLedger::new(64).expect("non-zero capacity"),
            targets: HashMap::new(),
        }
    }

    pub fn host(&self) -> &HarnessHost {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut HarnessHost {
        &mut self.host
    }

    /// Submits one compare-and-replace request through the full ADR 0030
    /// contract: ledger idempotency, blocked-target gate, fixed-order
    /// eligibility, one atomic transaction, and post-commit verification.
    pub fn submit(&mut self, request: &MutationRequest) -> MutationOutcome {
        if request.request_id.is_empty() {
            return MutationOutcome::Rejected {
                reason: RejectReason::RequestIdInvalid,
            };
        }
        let canonical = canonical(request);
        match self.ledger.classify(&request.request_id, &canonical) {
            RequestDisposition::Duplicate(recorded) => return decode(&recorded),
            RequestDisposition::Conflict => {
                return MutationOutcome::Rejected {
                    reason: RejectReason::RequestIdReuse,
                };
            }
            RequestDisposition::Fresh => {}
        }
        let target = logical_target(&request.uri, &request.expected_text);
        let blocked = matches!(
            self.targets.get(&target),
            Some(
                TargetState::Indeterminate
                    | TargetState::Reconciled {
                        acknowledged: false,
                    }
            )
        );
        if blocked {
            // A gate state, not a mutation result: deliberately not recorded.
            return MutationOutcome::BlockedPendingReconciliation;
        }
        let outcome = self.validate_and_mutate(request);
        if matches!(outcome, MutationOutcome::Indeterminate { .. }) {
            self.targets.insert(target, TargetState::Indeterminate);
        }
        self.ledger
            .record(&request.request_id, &canonical, encode(&outcome));
        outcome
    }

    /// Fixed-order eligibility (ADR 0030 items 1–14 minus the service-side
    /// handoff gate owned by M3D-22), then one atomic transaction.
    #[allow(clippy::too_many_lines)]
    fn validate_and_mutate(&mut self, request: &MutationRequest) -> MutationOutcome {
        let reject = |reason: RejectReason| MutationOutcome::Rejected { reason };
        if request.protocol_id != MUTATION_PROTOCOL_ID {
            return reject(RejectReason::ProtocolMismatch);
        }
        if request.host_id != self.host.host_id {
            return reject(RejectReason::HostIdentityMismatch);
        }
        if request.session_id != self.host.session_id {
            return reject(RejectReason::SessionMismatch);
        }
        if request.uri != self.host.uri
            || request.epoch != self.host.epoch
            || request.editor_id != self.host.editor_id
        {
            return reject(RejectReason::TargetIdentityMismatch);
        }
        if request.caret != self.host.caret || request.range_end != request.caret {
            return reject(RejectReason::CaretMismatch);
        }
        if self.host.selection_span != 0 {
            return reject(RejectReason::SelectionNotEmpty);
        }
        let length = self.host.utf16_length();
        let splits_surrogate = |index: usize| -> bool {
            let units: Vec<u16> = self.host.text().encode_utf16().collect();
            index > 0
                && index < units.len()
                && (0xd800..=0xdbff).contains(&units[index - 1])
                && (0xdc00..=0xdfff).contains(&units[index])
        };
        if request.range_start > request.range_end
            || request.range_end > length
            || splits_surrogate(request.range_start)
            || splits_surrogate(request.range_end)
        {
            return reject(RejectReason::RangeMismatch);
        }
        if self
            .host
            .utf16_slice(request.range_start, request.range_end)
            != request.expected_text
        {
            return reject(RejectReason::TextMismatch);
        }
        if request.revision != self.host.version {
            return reject(RejectReason::RevisionMismatch);
        }
        if request.revision == u64::MAX {
            return reject(RejectReason::RevisionOverflow);
        }
        match self.host.secure {
            HarnessSecure::KnownNonSecure => {}
            HarnessSecure::Secure => return reject(RejectReason::SecureTarget),
            HarnessSecure::Unknown => return reject(RejectReason::SecureUnknown),
        }
        match self.host.session_state {
            HarnessSession::SupportedLocal => {}
            HarnessSession::UnsupportedRemote => return reject(RejectReason::UnsupportedSession),
            HarnessSession::Unknown => return reject(RejectReason::SessionUnknown),
        }
        match self.host.composition {
            HarnessComposition::Active => return reject(RejectReason::CompositionActive),
            HarnessComposition::Unknown => return reject(RejectReason::CompositionUnknown),
            HarnessComposition::Inactive => {
                // Inactive is only believable with the synthetic proof bit.
                if self.host.capabilities & CAP_COMPOSITION_PROOF == 0 {
                    return reject(RejectReason::CompositionUnknown);
                }
                if request.composition != HarnessComposition::Inactive {
                    return reject(RejectReason::CompositionActive);
                }
            }
        }
        if request.capabilities != self.host.capabilities {
            return reject(RejectReason::CapabilityMismatch);
        }
        self.transaction(request)
    }

    /// One host-owned transaction: race injection, in-transaction re-read,
    /// a single replace, optional post-commit race, and exact verification.
    fn transaction(&mut self, request: &MutationRequest) -> MutationOutcome {
        let revision_before = self.host.version;
        let prefix = self.host.utf16_slice(0, request.range_start);
        let suffix = self
            .host
            .utf16_slice(request.range_end, self.host.utf16_length());
        let pre = self.host.pre_transaction_change;
        self.host.inject_change(pre);
        let version_changed_early = self.host.version != revision_before;

        // In-transaction re-read: the authorization and the mutation are
        // one step; there is no separate read-then-mutate window.
        if self
            .host
            .utf16_slice(request.range_start, request.range_end)
            != request.expected_text
        {
            return if version_changed_early {
                MutationOutcome::Indeterminate {
                    reason: IndeterminateReason::AmbiguousCommit,
                }
            } else {
                MutationOutcome::Rejected {
                    reason: RejectReason::TextMismatch,
                }
            };
        }
        match self.host.transaction_mode {
            TransactionMode::Refuse => {
                return MutationOutcome::Rejected {
                    reason: RejectReason::EditTransactionRefused,
                };
            }
            TransactionMode::Lost => {
                return MutationOutcome::Indeterminate {
                    reason: IndeterminateReason::EditOutcomeLost,
                };
            }
            TransactionMode::Normal | TransactionMode::LostAfterCommit => {}
        }
        if !self
            .host
            .replace_utf16(request.range_start, request.range_end, &request.replacement)
        {
            return MutationOutcome::Rejected {
                reason: RejectReason::RangeMismatch,
            };
        }
        self.host.version = self.host.version.saturating_add(1);
        let post = self.host.post_commit_change;
        self.host.inject_change(post);

        let version_after = self.host.version;
        let text_after = self.host.text().to_owned();
        let applied = version_after == revision_before + 1
            && text_after == format!("{prefix}{}{suffix}", request.replacement);
        if applied {
            if self.host.transaction_mode == TransactionMode::LostAfterCommit {
                // The commit happened, but the response never arrived.
                MutationOutcome::Indeterminate {
                    reason: IndeterminateReason::EditOutcomeLost,
                }
            } else {
                MutationOutcome::Applied {
                    new_revision: version_after,
                }
            }
        } else {
            MutationOutcome::Indeterminate {
                reason: IndeterminateReason::AmbiguousCommit,
            }
        }
    }

    /// Reconciliation readback for an Indeterminate target: compares the
    /// live range against the intended replacement and the original
    /// rendered text. The verdict is recorded but the target stays blocked
    /// until [`MutationHarness::owner_ack`].
    pub fn reconcile(
        &mut self,
        uri: &str,
        expected: &str,
        replacement: &str,
        range: (usize, usize),
    ) -> Option<ReconciliationVerdict> {
        let target = logical_target(uri, expected);
        if !matches!(self.targets.get(&target), Some(TargetState::Indeterminate)) {
            return None;
        }
        let live = self.host.utf16_slice(range.0, range.1);
        let verdict = if live == replacement {
            ReconciliationVerdict::AppliedAcknowledged
        } else if live == expected {
            ReconciliationVerdict::NotApplied
        } else {
            ReconciliationVerdict::ConflictHumanReview
        };
        self.targets.insert(
            target,
            TargetState::Reconciled {
                acknowledged: false,
            },
        );
        Some(verdict)
    }

    /// Explicit owner acknowledgement; only valid after reconciliation.
    /// Returns false when the target is missing or not yet reconciled.
    pub fn owner_ack(&mut self, uri: &str, expected: &str) -> bool {
        let target = logical_target(uri, expected);
        match self.targets.get_mut(&target) {
            Some(TargetState::Reconciled { acknowledged }) => {
                *acknowledged = true;
                true
            }
            _ => false,
        }
    }
}

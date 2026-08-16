//! M3D-26 harness tests: every ADR 0030 semantic on the deterministic
//! dummy host. Synthetic `Inactive` composition and `CAP_COMPOSITION_PROOF`
//! exist only here; the real VS Code binding is untouched.

use crate::mutation_harness::TransactionMode;
use crate::mutation_harness::*;

fn new_harness(text: &str) -> MutationHarness {
    MutationHarness::new(HarnessHost::new(text))
}

fn applied(new_revision: u64) -> MutationOutcome {
    MutationOutcome::Applied { new_revision }
}

fn rejected(reason: RejectReason) -> MutationOutcome {
    MutationOutcome::Rejected { reason }
}

fn indeterminate(reason: IndeterminateReason) -> MutationOutcome {
    MutationOutcome::Indeterminate { reason }
}

#[test]
fn happy_path_applies_exactly_once_with_exact_text() {
    let mut harness = new_harness("resume");
    let request = MutationRequest::matching(harness.host(), "resume", "restored");
    assert_eq!(harness.submit(&request), applied(2));
    assert_eq!(harness.host().text(), "restored");
    assert_eq!(harness.host().version, 2);
    // Surrogate-safe UTF-16 ranges work end to end.
    let mut emoji = new_harness("\u{1f600}resume");
    let request = MutationRequest::matching(emoji.host(), "resume", "restored");
    assert_eq!(emoji.submit(&request), applied(2));
    assert_eq!(emoji.host().text(), "\u{1f600}restored");
}

#[test]
fn eligibility_rejects_every_major_mismatch_without_mutation() {
    struct Case {
        name: &'static str,
        mutate_host: Option<Box<dyn Fn(&mut HarnessHost)>>,
        mutate_request: Option<Box<dyn Fn(&mut MutationRequest)>>,
        reason: RejectReason,
        expected_version: u64,
    }
    let case = |name: &'static str,
                mutate_host: Option<Box<dyn Fn(&mut HarnessHost)>>,
                mutate_request: Option<Box<dyn Fn(&mut MutationRequest)>>,
                reason,
                expected_version| Case {
        name,
        mutate_host,
        mutate_request,
        reason,
        expected_version,
    };
    let cases = vec![
        case(
            "protocol",
            None,
            Some(Box::new(|request: &mut MutationRequest| {
                request.protocol_id = "other/1".to_owned();
            })),
            RejectReason::ProtocolMismatch,
            1,
        ),
        case(
            "host",
            None,
            Some(Box::new(|request| {
                request.host_id = "other-host".to_owned()
            })),
            RejectReason::HostIdentityMismatch,
            1,
        ),
        case(
            "session",
            None,
            Some(Box::new(|request| request.session_id = "other".to_owned())),
            RejectReason::SessionMismatch,
            1,
        ),
        case(
            "identity",
            None,
            Some(Box::new(|request| request.epoch = 99)),
            RejectReason::TargetIdentityMismatch,
            1,
        ),
        case(
            "caret",
            None,
            Some(Box::new(|request| request.caret = 5)),
            RejectReason::CaretMismatch,
            1,
        ),
        case(
            "selection",
            Some(Box::new(|host: &mut HarnessHost| host.selection_span = 2)),
            None,
            RejectReason::SelectionNotEmpty,
            1,
        ),
        case(
            "range",
            None,
            Some(Box::new(|request| {
                request.range_start = request.range_end + 1
            })),
            RejectReason::RangeMismatch,
            1,
        ),
        case(
            "text",
            None,
            Some(Box::new(|request| {
                request.expected_text = "other".to_owned()
            })),
            RejectReason::TextMismatch,
            1,
        ),
        case(
            "stale-version",
            Some(Box::new(|host: &mut HarnessHost| host.version += 1)),
            None,
            RejectReason::RevisionMismatch,
            // the setup itself advanced the version to 2
            2,
        ),
        case(
            "secure",
            Some(Box::new(|host| host.secure = HarnessSecure::Secure)),
            None,
            RejectReason::SecureTarget,
            1,
        ),
        case(
            "secure-unknown",
            Some(Box::new(|host| host.secure = HarnessSecure::Unknown)),
            None,
            RejectReason::SecureUnknown,
            1,
        ),
        case(
            "remote-session",
            Some(Box::new(|host| {
                host.session_state = HarnessSession::UnsupportedRemote;
            })),
            None,
            RejectReason::UnsupportedSession,
            1,
        ),
        case(
            "unknown-session",
            Some(Box::new(|host| {
                host.session_state = HarnessSession::Unknown
            })),
            None,
            RejectReason::SessionUnknown,
            1,
        ),
        case(
            "capabilities",
            None,
            Some(Box::new(|request| request.capabilities &= !CAP_SNAPSHOT)),
            RejectReason::CapabilityMismatch,
            1,
        ),
    ];
    for case in cases {
        let mut harness = new_harness("resume");
        let mut request = MutationRequest::matching(harness.host(), "resume", "restored");
        if let Some(mutate) = case.mutate_host {
            mutate(harness.host_mut());
        }
        if let Some(mutate) = case.mutate_request {
            mutate(&mut request);
        }
        assert_eq!(
            harness.submit(&request),
            rejected(case.reason),
            "case {}",
            case.name
        );
        assert_eq!(harness.host().text(), "resume", "case {}", case.name);
        assert_eq!(
            harness.host().version,
            case.expected_version,
            "case {}",
            case.name
        );
    }
}

#[test]
fn composition_unknown_and_active_reject() {
    for (state, reason) in [
        (
            HarnessComposition::Unknown,
            RejectReason::CompositionUnknown,
        ),
        (HarnessComposition::Active, RejectReason::CompositionActive),
    ] {
        let mut harness = new_harness("resume");
        harness.host_mut().composition = state;
        let request = MutationRequest::matching(harness.host(), "resume", "restored");
        assert_eq!(harness.submit(&request), rejected(reason));
        assert_eq!(harness.host().text(), "resume");
    }
}

#[test]
fn inactive_without_composition_proof_capability_rejects() {
    let mut harness = new_harness("resume");
    harness.host_mut().composition = HarnessComposition::Inactive;
    harness.host_mut().capabilities = CAP_ALL_WITH_COMPOSITION & !CAP_COMPOSITION_PROOF;
    let request = MutationRequest::matching(harness.host(), "resume", "restored");
    assert_eq!(
        harness.submit(&request),
        rejected(RejectReason::CompositionUnknown)
    );
    assert_eq!(harness.host().text(), "resume");
}

#[test]
fn text_mismatch_inside_transaction_is_fail_closed() {
    // A pre-transaction change that also bumps the version makes the
    // in-transaction re-read mismatch ambiguous.
    let mut harness = new_harness("resume");
    harness.host_mut().pre_transaction_change = Some((0, 6, "other!"));
    let request = MutationRequest::matching(harness.host(), "resume", "restored");
    assert_eq!(
        harness.submit(&request),
        indeterminate(IndeterminateReason::AmbiguousCommit)
    );
    assert_eq!(harness.host().text(), "other!");
    assert_eq!(harness.host().version, 2);
}

#[test]
fn range_surrogate_split_rejects() {
    let mut harness = new_harness("\u{1f600}resume");
    let mut request = MutationRequest::matching(harness.host(), "resume", "restored");
    // Split the emoji surrogate pair while keeping the caret consistent.
    request.range_start = 1;
    assert_eq!(
        harness.submit(&request),
        rejected(RejectReason::RangeMismatch)
    );
    assert_eq!(harness.host().text(), "\u{1f600}resume");
}

#[test]
fn duplicate_applied_request_does_not_reapply() {
    let mut harness = new_harness("resume");
    let request = MutationRequest::matching(harness.host(), "resume", "restored");
    assert_eq!(harness.submit(&request), applied(2));
    assert_eq!(harness.submit(&request), applied(2));
    assert_eq!(harness.host().text(), "restored");
    assert_eq!(harness.host().version, 2);
}

#[test]
fn conflicting_request_id_reuse_rejects() {
    let mut harness = new_harness("resume");
    let request = MutationRequest::matching(harness.host(), "resume", "restored");
    assert_eq!(harness.submit(&request), applied(2));
    let mut conflict = request.clone();
    conflict.replacement = "other".to_owned();
    assert_eq!(
        harness.submit(&conflict),
        rejected(RejectReason::RequestIdReuse)
    );
}

#[test]
fn empty_request_id_is_invalid() {
    let mut harness = new_harness("resume");
    let mut request = MutationRequest::matching(harness.host(), "resume", "restored");
    request.request_id = String::new();
    assert_eq!(
        harness.submit(&request),
        rejected(RejectReason::RequestIdInvalid)
    );
}

#[test]
fn refused_transaction_rejects_without_mutation() {
    let mut harness = new_harness("resume");
    harness.host_mut().transaction_mode = TransactionMode::Refuse;
    let mut request = MutationRequest::matching(harness.host(), "resume", "restored");
    request.request_id = "req-2".to_owned();
    assert_eq!(
        harness.submit(&request),
        rejected(RejectReason::EditTransactionRefused)
    );
    assert_eq!(harness.host().text(), "resume");
    assert_eq!(harness.host().version, 1);
}

#[test]
fn lost_outcome_is_indeterminate_and_replays_as_recorded() {
    let mut harness = new_harness("resume");
    harness.host_mut().transaction_mode = TransactionMode::Lost;
    let request = MutationRequest::matching(harness.host(), "resume", "restored");
    assert_eq!(
        harness.submit(&request),
        indeterminate(IndeterminateReason::EditOutcomeLost)
    );
    // Nothing was applied.
    assert_eq!(harness.host().text(), "resume");
    // Duplicate replays the recorded Indeterminate, never retries.
    assert_eq!(
        harness.submit(&request),
        indeterminate(IndeterminateReason::EditOutcomeLost)
    );
    assert_eq!(harness.host().text(), "resume");
    assert_eq!(harness.host().version, 1);
}

#[test]
fn commit_then_lost_response_reconciles_as_applied_acknowledged() {
    let mut harness = new_harness("resume");
    harness.host_mut().transaction_mode = TransactionMode::LostAfterCommit;
    let request = MutationRequest::matching(harness.host(), "resume", "restored");
    assert_eq!(
        harness.submit(&request),
        indeterminate(IndeterminateReason::EditOutcomeLost)
    );
    assert_eq!(harness.host().text(), "restored");
    // The one-shot loss does not repeat on the next request.
    harness.host_mut().transaction_mode = TransactionMode::Normal;
    // A different request id for the SAME logical target (document +
    // rendered token "resume") is blocked first.
    harness.host_mut().caret = 8;
    let mut fresh = MutationRequest::matching(harness.host(), "resume", "other");
    fresh.request_id = "req-2".to_owned();
    assert_eq!(
        harness.submit(&fresh),
        MutationOutcome::BlockedPendingReconciliation
    );
    // Reconciliation readback finds the exact replacement.
    assert_eq!(
        harness.reconcile(&request.uri, "resume", "restored", (0, 8)),
        Some(ReconciliationVerdict::AppliedAcknowledged)
    );
    // Still blocked until the owner acknowledges.
    assert_eq!(
        harness.submit(&fresh),
        MutationOutcome::BlockedPendingReconciliation
    );
    assert!(harness.owner_ack(&request.uri, "resume"));
    // After acknowledgement the request is evaluated normally: the live
    // text no longer contains the rendered token, so it fails closed.
    assert_eq!(harness.submit(&fresh), rejected(RejectReason::TextMismatch));
}

#[test]
fn lost_before_commit_reconciles_as_not_applied() {
    let mut harness = new_harness("resume");
    harness.host_mut().transaction_mode = TransactionMode::Lost;
    let request = MutationRequest::matching(harness.host(), "resume", "restored");
    assert_eq!(
        harness.submit(&request),
        indeterminate(IndeterminateReason::EditOutcomeLost)
    );
    assert_eq!(
        harness.reconcile(&request.uri, "resume", "restored", (0, 6)),
        Some(ReconciliationVerdict::NotApplied)
    );
    assert!(harness.owner_ack(&request.uri, "resume"));
    harness.host_mut().transaction_mode = TransactionMode::Normal;
    let mut fresh = MutationRequest::matching(harness.host(), "resume", "restored");
    fresh.request_id = "req-2".to_owned();
    assert_eq!(harness.submit(&fresh), applied(2));
}

#[test]
fn contradictory_live_state_reconciles_as_conflict() {
    let mut harness = new_harness("resume");
    harness.host_mut().transaction_mode = TransactionMode::LostAfterCommit;
    harness.host_mut().post_commit_change = Some((0, 8, "mangled!"));
    let request = MutationRequest::matching(harness.host(), "resume", "restored");
    assert_eq!(
        harness.submit(&request),
        indeterminate(IndeterminateReason::AmbiguousCommit)
    );
    assert_eq!(
        harness.reconcile(&request.uri, "resume", "restored", (0, 8)),
        Some(ReconciliationVerdict::ConflictHumanReview)
    );
    // Conflict still unblocks only after explicit owner acknowledgement.
    assert!(harness.owner_ack(&request.uri, "resume"));
}

#[test]
fn blocked_target_requires_reconciliation_then_ack() {
    let mut harness = new_harness("resume");
    harness.host_mut().transaction_mode = TransactionMode::Lost;
    let request = MutationRequest::matching(harness.host(), "resume", "restored");
    assert_eq!(
        harness.submit(&request),
        indeterminate(IndeterminateReason::EditOutcomeLost)
    );
    let mut fresh = MutationRequest::matching(harness.host(), "resume", "restored");
    fresh.request_id = "req-2".to_owned();
    assert_eq!(
        harness.submit(&fresh),
        MutationOutcome::BlockedPendingReconciliation
    );
    // Ack before reconciliation is refused.
    assert!(!harness.owner_ack(&request.uri, "resume"));
    assert_eq!(
        harness.reconcile(&request.uri, "resume", "restored", (0, 6)),
        Some(ReconciliationVerdict::NotApplied)
    );
    assert_eq!(
        harness.submit(&fresh),
        MutationOutcome::BlockedPendingReconciliation
    );
    assert!(harness.owner_ack(&request.uri, "resume"));
    harness.host_mut().transaction_mode = TransactionMode::Normal;
    assert_eq!(harness.submit(&fresh), applied(2));
}

#[test]
fn revision_overflow_fails_closed() {
    let mut harness = new_harness("resume");
    harness.host_mut().version = u64::MAX;
    let mut request = MutationRequest::matching(harness.host(), "resume", "restored");
    request.revision = u64::MAX;
    assert_eq!(
        harness.submit(&request),
        rejected(RejectReason::RevisionOverflow)
    );
    assert_eq!(harness.host().text(), "resume");
}

#[test]
fn no_partial_mutation_happens_on_any_rejection() {
    let mut harness = new_harness("resume");
    // Validation-time rejection.
    let mut stale = MutationRequest::matching(harness.host(), "resume", "restored");
    stale.revision = 0;
    assert_eq!(
        harness.submit(&stale),
        rejected(RejectReason::RevisionMismatch)
    );
    // Transaction refusal.
    harness.host_mut().transaction_mode = TransactionMode::Refuse;
    let mut request = MutationRequest::matching(harness.host(), "resume", "restored");
    request.request_id = "req-2".to_owned();
    assert_eq!(
        harness.submit(&request),
        rejected(RejectReason::EditTransactionRefused)
    );
    assert_eq!(harness.host().text(), "resume");
    assert_eq!(harness.host().version, 1);
}

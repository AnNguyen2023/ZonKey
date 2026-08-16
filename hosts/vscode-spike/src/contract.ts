/**
 * JSON-shaped cooperating-host contract for the M3D-17 VS Code spike.
 *
 * Mirrors the `dummy_host` vocabulary in `crates/zonkey-service/src/lib.rs` so
 * the same request/result shapes can cross a future transport. Deltas from the
 * service contract: `EditTransactionRefused` and the two `Indeterminate`
 * reasons are VS Code-transaction-specific, identities are strings, and the
 * request id is a non-empty string.
 */

/** Protocol identity advertised by this host adapter. */
export const PROTOCOL_ID = "zonkey.vscode-host/1";

/**
 * Unit schema for ranges and carets. Schema 1 is the zonkey-service in-proc
 * dummy (Unicode scalars); schema 2 is UTF-16 code units, which is what VS
 * Code `Position`/`offsetAt` natively use.
 */
export const UNIT_SCHEMA_UTF16 = 2;

/** Capability flag: the host can produce evidence snapshots. */
export const CAP_SNAPSHOT = 0b0000_0001;
/** Capability flag: the host supports compare-and-replace apply. */
export const CAP_COMPARE_AND_REPLACE = 0b0000_0010;
/** Capability flag: ranges and carets are UTF-16 code units. */
export const CAP_UTF16_UNITS = 0b0000_0100;
/** Capability flag: exact duplicate request ids replay recorded results. */
export const CAP_IDEMPOTENT_REQUESTS = 0b0000_1000;
/** Capability flag: the host can prove IME composition inactivity. */
export const CAP_COMPOSITION_PROOF = 0b0001_0000;

export type SecureState = "KnownNonSecure" | "Secure" | "Unknown";
export type CompositionState = "Inactive" | "Active" | "Unknown";
export type SessionState = "SupportedLocal" | "UnsupportedRemote" | "Unknown";

export type SelectionState =
  | { kind: "Empty" }
  | { kind: "Range"; start: number; end: number };

/** Host-assigned identity of the targeted document/editor pair. */
export interface TargetIdentity {
  document_uri: string;
  document_epoch: number;
  editor_id: number;
}

/** Half-open UTF-16 range plus the unit schema it was measured in. */
export interface TextRange {
  unit_schema: number;
  start: number;
  end: number;
}

export interface Capabilities {
  flags: number;
  unit_schema: number;
}

/** Evidence snapshot captured by the host for one candidate restore. */
export interface HostSnapshot {
  protocol_id: string;
  host_id: string;
  session_id: string;
  identity: TargetIdentity;
  /** `TextDocument.version` at capture time. */
  revision: number;
  range: TextRange;
  expected_text: string;
  replacement: string;
  caret: number;
  selection: SelectionState;
  secure: SecureState;
  composition: CompositionState;
  session: SessionState;
  capabilities: Capabilities;
}

/** Compare-and-replace request; field order of checks mirrors the dummy host. */
export interface CompareAndReplaceRequest {
  protocol_id: string;
  authenticated: boolean;
  host_id: string;
  session_id: string;
  request_id: string;
  identity: TargetIdentity;
  revision: number;
  expected_range: TextRange;
  expected_text: string;
  replacement: string;
  caret: number;
  selection: SelectionState;
  secure: SecureState;
  composition: CompositionState;
  session: SessionState;
  capabilities: Capabilities;
}

/**
 * Rejection vocabulary of the service contract plus one spike-specific reason:
 * `EditTransactionRefused` covers `TextEditor.edit` resolving `false`, which
 * proves this transaction queued no mutation but does not explain why.
 */
export type RejectReason =
  | "MalformedRequest"
  | "ProtocolMismatch"
  | "AuthenticationFailed"
  | "HostIdentityMismatch"
  | "SessionMismatch"
  | "TargetIdentityMismatch"
  | "RevisionMismatch"
  | "TextMismatch"
  | "RangeMismatch"
  | "UnitMismatch"
  | "CaretMismatch"
  | "SelectionNotEmpty"
  | "SecureTarget"
  | "SecureUnknown"
  | "CompositionActive"
  | "CompositionUnknown"
  | "UnsupportedSession"
  | "SessionUnknown"
  | "CapabilityMismatch"
  | "RequestIdReuse"
  | "RevisionOverflow"
  | "EditTransactionRefused";

export type IndeterminateReason =
  /** The edit promise rejected; the outcome is lost. */
  | "EditOutcomeLost"
  /** The transaction reported success but the result is not exactly provable. */
  | "AmbiguousCommit";

export type HostResult =
  | { kind: "Applied"; new_revision: number }
  | { kind: "Rejected"; reason: RejectReason }
  | { kind: "Indeterminate"; reason: IndeterminateReason };

/** Fail-closed refusal reasons for evidence capture (spike-internal). */
export type SnapshotRefusalReason =
  | "NoActiveEditor"
  | "MultiSelection"
  | "SelectionNotEmpty"
  | "RangeInvalid"
  | "CandidateMismatch";

export type SnapshotOutcome =
  | { ok: true; snapshot: HostSnapshot }
  | { ok: false; reason: SnapshotRefusalReason };

/**
 * Deterministic JSON form used for exact-duplicate request comparison and
 * capability equality. Object keys are sorted; `undefined` fields are dropped.
 */
export function canonicalJson(value: unknown): string {
  if (value === null || typeof value !== "object") {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJson).join(",")}]`;
  }
  const entries = Object.entries(value as Record<string, unknown>)
    .filter(([, item]) => item !== undefined)
    .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0));
  return `{${entries
    .map(([key, item]) => `${JSON.stringify(key)}:${canonicalJson(item)}`)
    .join(",")}}`;
}

/** UTF-16 length of a JS string (its `.length`, named for intent). */
export function utf16Length(text: string): number {
  return text.length;
}

/** True when the UTF-16 index splits a surrogate pair in `text`. */
export function splitsSurrogate(text: string, index: number): boolean {
  if (index <= 0 || index >= text.length) {
    return false;
  }
  const previous = text.charCodeAt(index - 1);
  const current = text.charCodeAt(index);
  return (
    previous >= 0xd800 &&
    previous <= 0xdbff &&
    current >= 0xdc00 &&
    current <= 0xdfff
  );
}

/** VS Code document versions are 32-bit; refuse to count past that. */
export const MAX_REVISION = 0x7fffffff;

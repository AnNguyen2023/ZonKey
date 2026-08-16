/**
 * M3D-17 VS Code host adapter core.
 *
 * Platform-neutral: depends only on the ports in `ports.ts`. Snapshot capture
 * and compare-and-replace follow the cooperating-host contract validated by
 * the `dummy_host` tests in `crates/zonkey-service`. Every check is fail
 * closed; ambiguous outcomes are `Indeterminate` and are never retried here.
 *
 * Safety shape of the apply path: validation, the immediate pre-edit re-read,
 * and queuing the single replace all happen inside one synchronous run of the
 * edit transaction callback, so the extension-host event loop introduces no
 * interleaving between them. The awaited commit is verified afterwards by
 * requiring the next version to be exactly `revision + 1` and the resulting
 * document text to be exactly `prefix + replacement + suffix`; anything else
 * is `Indeterminate`.
 */
import type {
  CompareAndReplaceRequest,
  HostResult,
  HostSnapshot,
  SnapshotOutcome,
} from "./contract.ts";
import {
  MAX_REVISION,
  PROTOCOL_ID,
  UNIT_SCHEMA_UTF16,
  canonicalJson,
  splitsSurrogate,
  utf16Length,
} from "./contract.ts";
import type { HostBindingPort, TextEditorPort } from "./ports.ts";

interface LedgerEntry {
  canonical: string;
  result: HostResult;
}

export class VsCodeHostAdapter {
  private readonly binding: HostBindingPort;
  private readonly ledger = new Map<string, LedgerEntry>();

  constructor(binding: HostBindingPort) {
    this.binding = binding;
  }

  /**
   * Captures the required evidence snapshot for one candidate restore whose
   * exact `expected_text` sits immediately before the caret. Refuses, rather
   * than degrades, on any structural impossibility; composition/secure/session
   * states are recorded as evidence and enforced at apply time.
   */
  captureSnapshot(candidate: {
    expected_text: string;
    replacement: string;
  }): SnapshotOutcome {
    if (
      typeof candidate.expected_text !== "string" ||
      candidate.expected_text.length === 0 ||
      typeof candidate.replacement !== "string" ||
      candidate.replacement.length === 0
    ) {
      return { ok: false, reason: "RangeInvalid" };
    }
    const editor = this.binding.getActiveEditor();
    if (editor === undefined) {
      return { ok: false, reason: "NoActiveEditor" };
    }
    if (editor.selectionCount !== 1) {
      return { ok: false, reason: "MultiSelection" };
    }
    const start = editor.selectionStart;
    const caret = editor.selectionEnd;
    if (start !== caret) {
      return { ok: false, reason: "SelectionNotEmpty" };
    }
    const text = editor.document.getText();
    if (
      caret > text.length ||
      splitsSurrogate(text, caret) ||
      utf16Length(candidate.expected_text) > caret
    ) {
      return { ok: false, reason: "RangeInvalid" };
    }
    const rangeStart = caret - utf16Length(candidate.expected_text);
    if (splitsSurrogate(text, rangeStart)) {
      return { ok: false, reason: "RangeInvalid" };
    }
    if (text.slice(rangeStart, caret) !== candidate.expected_text) {
      return { ok: false, reason: "CandidateMismatch" };
    }
    const document = editor.document;
    const snapshot: HostSnapshot = {
      protocol_id: PROTOCOL_ID,
      host_id: this.binding.host_id,
      session_id: this.binding.session_id,
      identity: {
        document_uri: document.uri,
        document_epoch: this.binding.documentEpoch(document),
        editor_id: this.binding.editorId(editor),
      },
      revision: document.version,
      range: { unit_schema: UNIT_SCHEMA_UTF16, start: rangeStart, end: caret },
      expected_text: candidate.expected_text,
      replacement: candidate.replacement,
      caret,
      selection: { kind: "Empty" },
      secure: this.binding.secureStateFor(document.uri),
      composition: this.binding.composition,
      session: this.binding.session,
      capabilities: { ...this.binding.capabilities },
    };
    return { ok: true, snapshot };
  }

  /**
   * Applies one compare-and-replace request. Exact duplicates replay their
   * recorded result without reapplying; conflicting reuse of a request id
   * rejects. The ledger records every terminal outcome, including
   * `Indeterminate`, which is never retried by this adapter.
   */
  async apply(request: CompareAndReplaceRequest): Promise<HostResult> {
    if (
      typeof request !== "object" ||
      request === null ||
      typeof request.request_id !== "string" ||
      request.request_id.length === 0
    ) {
      return { kind: "Rejected", reason: "MalformedRequest" };
    }
    const canonical = canonicalJson(request);
    const recorded = this.ledger.get(request.request_id);
    if (recorded !== undefined) {
      return recorded.canonical === canonical
        ? recorded.result
        : { kind: "Rejected", reason: "RequestIdReuse" };
    }
    const result = await this.validateAndApply(request);
    this.ledger.set(request.request_id, { canonical, result });
    return result;
  }

  /**
   * Clears the request ledger. Call only when the session identity has
   * actually changed (extension host restart); old requests then fail closed
   * on the session check, mirroring the dummy host's restart.
   */
  invalidateSession(): void {
    this.ledger.clear();
  }

  private async validateAndApply(
    request: CompareAndReplaceRequest,
  ): Promise<HostResult> {
    const editor = this.binding.getActiveEditor();
    if (editor === undefined) {
      return { kind: "Rejected", reason: "TargetIdentityMismatch" };
    }
    const document = editor.document;
    const text = document.getText();
    const liveIdentity = {
      document_uri: document.uri,
      document_epoch: this.binding.documentEpoch(document),
      editor_id: this.binding.editorId(editor),
    };
    if (request.protocol_id !== PROTOCOL_ID) {
      return { kind: "Rejected", reason: "ProtocolMismatch" };
    }
    if (request.authenticated !== true) {
      return { kind: "Rejected", reason: "AuthenticationFailed" };
    }
    if (request.host_id !== this.binding.host_id) {
      return { kind: "Rejected", reason: "HostIdentityMismatch" };
    }
    if (request.session_id !== this.binding.session_id) {
      return { kind: "Rejected", reason: "SessionMismatch" };
    }
    if (canonicalJson(request.identity) !== canonicalJson(liveIdentity)) {
      return { kind: "Rejected", reason: "TargetIdentityMismatch" };
    }
    if (
      canonicalJson(request.capabilities) !==
      canonicalJson(this.binding.capabilities)
    ) {
      return { kind: "Rejected", reason: "CapabilityMismatch" };
    }
    if (request.expected_range.unit_schema !== UNIT_SCHEMA_UTF16) {
      return { kind: "Rejected", reason: "UnitMismatch" };
    }
    if (
      !Number.isSafeInteger(request.revision) ||
      request.revision < 0 ||
      request.revision >= MAX_REVISION
    ) {
      return { kind: "Rejected", reason: "RevisionOverflow" };
    }
    if (document.version !== request.revision) {
      return { kind: "Rejected", reason: "RevisionMismatch" };
    }
    const start = request.expected_range.start;
    const end = request.expected_range.end;
    if (
      !Number.isSafeInteger(start) ||
      !Number.isSafeInteger(end) ||
      start < 0 ||
      start > end ||
      end > utf16Length(text) ||
      splitsSurrogate(text, start) ||
      splitsSurrogate(text, end)
    ) {
      return { kind: "Rejected", reason: "RangeMismatch" };
    }
    if (request.caret !== editor.selectionEnd || end !== request.caret) {
      return { kind: "Rejected", reason: "CaretMismatch" };
    }
    if (
      request.selection.kind !== "Empty" ||
      editor.selectionStart !== editor.selectionEnd
    ) {
      return { kind: "Rejected", reason: "SelectionNotEmpty" };
    }
    const liveSecure = this.binding.secureStateFor(document.uri);
    if (request.secure !== liveSecure) {
      return { kind: "Rejected", reason: "SecureTarget" };
    }
    if (liveSecure === "Secure") {
      return { kind: "Rejected", reason: "SecureTarget" };
    }
    if (liveSecure === "Unknown") {
      return { kind: "Rejected", reason: "SecureUnknown" };
    }
    if (request.composition !== this.binding.composition) {
      return { kind: "Rejected", reason: "CompositionActive" };
    }
    if (this.binding.composition === "Active") {
      return { kind: "Rejected", reason: "CompositionActive" };
    }
    if (this.binding.composition === "Unknown") {
      return { kind: "Rejected", reason: "CompositionUnknown" };
    }
    if (request.session !== this.binding.session) {
      return { kind: "Rejected", reason: "SessionMismatch" };
    }
    if (this.binding.session === "UnsupportedRemote") {
      return { kind: "Rejected", reason: "UnsupportedSession" };
    }
    if (this.binding.session === "Unknown") {
      return { kind: "Rejected", reason: "SessionUnknown" };
    }
    if (text.slice(start, end) !== request.expected_text) {
      return { kind: "Rejected", reason: "TextMismatch" };
    }
    return this.commit(editor, request, start, end, text);
  }

  private async commit(
    editor: TextEditorPort,
    request: CompareAndReplaceRequest,
    start: number,
    end: number,
    validatedText: string,
  ): Promise<HostResult> {
    const revisionBefore = editor.document.version;
    const prefix = validatedText.slice(0, start);
    const suffix = validatedText.slice(end);
    let queued = false;
    let transaction: boolean;
    try {
      transaction = await editor.edit((builder) => {
        const immediate = editor.document.getText();
        if (immediate.slice(start, end) !== request.expected_text) {
          return;
        }
        builder.replace(start, end, request.replacement);
        queued = true;
      });
    } catch {
      return { kind: "Indeterminate", reason: "EditOutcomeLost" };
    }
    if (!transaction) {
      return { kind: "Rejected", reason: "EditTransactionRefused" };
    }
    const versionAfter = editor.document.version;
    const textAfter = editor.document.getText();
    if (!queued) {
      return versionAfter === revisionBefore
        ? { kind: "Rejected", reason: "TextMismatch" }
        : { kind: "Indeterminate", reason: "AmbiguousCommit" };
    }
    if (
      versionAfter !== revisionBefore + 1 ||
      textAfter !== prefix + request.replacement + suffix
    ) {
      return { kind: "Indeterminate", reason: "AmbiguousCommit" };
    }
    return { kind: "Applied", new_revision: versionAfter };
  }
}

/** Builds an authenticated request from a captured snapshot. */
export function requestFromSnapshot(
  snapshot: HostSnapshot,
  request_id: string,
): CompareAndReplaceRequest {
  return {
    protocol_id: snapshot.protocol_id,
    authenticated: true,
    host_id: snapshot.host_id,
    session_id: snapshot.session_id,
    request_id,
    identity: { ...snapshot.identity },
    revision: snapshot.revision,
    expected_range: { ...snapshot.range },
    expected_text: snapshot.expected_text,
    replacement: snapshot.replacement,
    caret: snapshot.caret,
    selection: snapshot.selection,
    secure: snapshot.secure,
    composition: snapshot.composition,
    session: snapshot.session,
    capabilities: { ...snapshot.capabilities },
  };
}

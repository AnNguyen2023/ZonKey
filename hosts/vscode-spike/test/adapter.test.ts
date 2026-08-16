/**
 * Contract tests for the M3D-17 VS Code host adapter core. Mirrors the
 * `dummy_host` suite in `crates/zonkey-service` plus spike-specific
 * transaction, race, and idempotency cases.
 */
import assert from "node:assert/strict";
import { test } from "node:test";
import { VsCodeHostAdapter, requestFromSnapshot } from "../src/adapter.ts";
import { canonicalJson } from "../src/contract.ts";
import type { CompareAndReplaceRequest, HostSnapshot } from "../src/contract.ts";
import { FakeDocument, FakeEditor, world } from "./harness.ts";
import type { FakeWorld } from "./harness.ts";

function capture(w: FakeWorld, expected = "resume", replacement = "restored"): HostSnapshot {
  const captured = w.adapter.captureSnapshot({ expected_text: expected, replacement });
  assert.ok(captured.ok, `snapshot refused: ${captured.ok ? "" : captured.reason}`);
  return captured.snapshot;
}

function request(w: FakeWorld, id = "req-1", expected = "resume", replacement = "restored"): CompareAndReplaceRequest {
  return requestFromSnapshot(capture(w, expected, replacement), id);
}

function rejected(result: { kind: string; reason?: string }, reason: string): void {
  assert.deepEqual(result, { kind: "Rejected", reason });
}

test("snapshot carries the full required evidence set", () => {
  const w = world("resume");
  const s = capture(w);
  assert.equal(s.protocol_id, "zonkey.vscode-host/1");
  assert.equal(s.host_id, "fake-host");
  assert.equal(s.session_id, "fake-session");
  assert.deepEqual(s.identity, {
    document_uri: "file:///d:/spike/doc.txt",
    document_epoch: 1,
    editor_id: 1,
  });
  assert.equal(s.revision, 1);
  assert.deepEqual(s.range, { unit_schema: 2, start: 0, end: 6 });
  assert.equal(s.expected_text, "resume");
  assert.equal(s.replacement, "restored");
  assert.equal(s.caret, 6);
  assert.deepEqual(s.selection, { kind: "Empty" });
  assert.equal(s.secure, "KnownNonSecure");
  assert.equal(s.composition, "Inactive");
  assert.equal(s.session, "SupportedLocal");
  assert.deepEqual(s.capabilities, { flags: 0b0001_1111, unit_schema: 2 });
});

test("snapshot range uses UTF-16 code units across a non-BMP prefix", () => {
  const w = world("😀resume");
  const s = capture(w);
  assert.deepEqual(s.range, { unit_schema: 2, start: 2, end: 8 });
  assert.equal(s.caret, 8);
});

test("snapshot refuses structural impossibilities fail-closed", () => {
  const w = world("resume");
  const good = w.adapter.captureSnapshot({ expected_text: "resume", replacement: "restored" });
  assert.ok(good.ok);

  w.binding.activeEditor = undefined;
  assert.deepEqual(w.adapter.captureSnapshot({ expected_text: "resume", replacement: "r" }), { ok: false, reason: "NoActiveEditor" });
  w.binding.activeEditor = w.editor;

  w.editor.selectionCount = 2;
  assert.deepEqual(w.adapter.captureSnapshot({ expected_text: "resume", replacement: "r" }), { ok: false, reason: "MultiSelection" });
  w.editor.selectionCount = 1;

  w.editor.selectionStart = 0;
  w.editor.selectionEnd = 2;
  assert.deepEqual(w.adapter.captureSnapshot({ expected_text: "resume", replacement: "r" }), { ok: false, reason: "SelectionNotEmpty" });
  w.editor.setCaret(6);

  assert.deepEqual(w.adapter.captureSnapshot({ expected_text: "resumf", replacement: "r" }), { ok: false, reason: "CandidateMismatch" });
  assert.deepEqual(w.adapter.captureSnapshot({ expected_text: "resume!!", replacement: "r" }), { ok: false, reason: "RangeInvalid" });
  assert.deepEqual(w.adapter.captureSnapshot({ expected_text: "", replacement: "r" }), { ok: false, reason: "RangeInvalid" });
});

test("apply replaces once, advances version by exactly one, returns Applied", async () => {
  const w = world("resume");
  const result = await w.adapter.apply(request(w));
  assert.deepEqual(result, { kind: "Applied", new_revision: 2 });
  assert.equal(w.doc.getText(), "restored");
  assert.equal(w.doc.version, 2);
  assert.equal(w.editor.editAttempts, 1);
});

test("duplicate exact request replays the recorded Applied without reapplying", async () => {
  const w = world("resume");
  const req = request(w);
  const first = await w.adapter.apply(req);
  const second = await w.adapter.apply(req);
  assert.deepEqual(second, first);
  assert.equal(w.doc.version, 2);
  assert.equal(w.editor.editAttempts, 1);
  assert.equal(w.doc.getText(), "restored");
});

test("conflicting request_id reuse rejects", async () => {
  const w = world("resume");
  const req = request(w);
  await w.adapter.apply(req);
  const conflict = structuredClone(req);
  conflict.replacement = "other";
  rejected(await w.adapter.apply(conflict), "RequestIdReuse");
  assert.equal(w.doc.getText(), "restored");
  assert.equal(w.editor.editAttempts, 1);
});

test("empty request_id is malformed", async () => {
  const w = world("resume");
  rejected(await w.adapter.apply(request(w, "")), "MalformedRequest");
  assert.equal(w.editor.editAttempts, 0);
});

test("identity-family mismatches reject without mutation", async () => {
  const cases: Array<[string, (r: CompareAndReplaceRequest) => void]> = [
    ["ProtocolMismatch", (r) => { r.protocol_id = "zonkey.other/1"; }],
    ["AuthenticationFailed", (r) => { r.authenticated = false; }],
    ["HostIdentityMismatch", (r) => { r.host_id = "other-host"; }],
    ["SessionMismatch", (r) => { r.session_id = "other-session"; }],
    ["TargetIdentityMismatch", (r) => { r.identity.document_epoch = 99; }],
    ["CapabilityMismatch", (r) => { r.capabilities.flags &= ~0b0000_0001; }],
    ["UnitMismatch", (r) => { r.expected_range.unit_schema = 1; }],
  ];
  for (const [reason, mutate] of cases) {
    const w = world("resume");
    const req = request(w);
    mutate(req);
    rejected(await w.adapter.apply(req), reason);
    assert.equal(w.editor.editAttempts, 0);
    assert.equal(w.doc.version, 1);
  }
});

test("stale document version rejects without mutation", async () => {
  const w = world("resume");
  const req = request(w);
  w.doc.replaceRange(6, 6, "!");
  rejected(await w.adapter.apply(req), "RevisionMismatch");
  assert.equal(w.doc.getText(), "resume!");
  assert.equal(w.doc.version, 2);
  assert.equal(w.editor.editAttempts, 0);
});

test("caret and selection mismatches reject without mutation", async () => {
  const w = world("resume");
  const movedCaret = request(w, "req-caret");
  movedCaret.caret = 5;
  rejected(await w.adapter.apply(movedCaret), "CaretMismatch");

  const movedLive = request(w, "req-caret-live");
  w.editor.setCaret(5);
  rejected(await w.adapter.apply(movedLive), "CaretMismatch");
  w.editor.setCaret(6);

  const liveSelection = request(w, "req-selection-live");
  w.editor.selectionStart = 0;
  w.editor.selectionEnd = 6;
  rejected(await w.adapter.apply(liveSelection), "SelectionNotEmpty");
  w.editor.setCaret(6);

  const requestSelection = request(w, "req-selection-request");
  requestSelection.selection = { kind: "Range", start: 0, end: 6 };
  rejected(await w.adapter.apply(requestSelection), "SelectionNotEmpty");
  assert.equal(w.editor.editAttempts, 0);
});

test("secure states fail closed", async () => {
  const w = world("resume");
  w.binding.secureState = "Secure";
  rejected(await w.adapter.apply(request(w)), "SecureTarget");

  const unknown = world("resume");
  unknown.binding.secureState = "Unknown";
  rejected(await unknown.adapter.apply(request(unknown)), "SecureUnknown");

  const disagreement = world("resume");
  const req = request(disagreement);
  req.secure = "Unknown";
  rejected(await disagreement.adapter.apply(req), "SecureTarget");
  assert.equal(w.editor.editAttempts, 0);
  assert.equal(unknown.editor.editAttempts, 0);
  assert.equal(disagreement.editor.editAttempts, 0);
});

test("composition states fail closed", async () => {
  const w = world("resume");
  w.binding.composition = "Active";
  rejected(await w.adapter.apply(request(w)), "CompositionActive");

  const unknown = world("resume");
  unknown.binding.composition = "Unknown";
  rejected(await unknown.adapter.apply(request(unknown)), "CompositionUnknown");

  const disagreement = world("resume");
  const req = request(disagreement);
  req.composition = "Unknown";
  rejected(await disagreement.adapter.apply(req), "CompositionActive");
  assert.equal(w.editor.editAttempts + unknown.editor.editAttempts + disagreement.editor.editAttempts, 0);
});

test("session states fail closed", async () => {
  const w = world("resume");
  w.binding.session = "UnsupportedRemote";
  rejected(await w.adapter.apply(request(w)), "UnsupportedSession");

  const unknown = world("resume");
  unknown.binding.session = "Unknown";
  rejected(await unknown.adapter.apply(request(unknown)), "SessionUnknown");

  const disagreement = world("resume");
  const req = request(disagreement);
  req.session = "UnsupportedRemote";
  rejected(await disagreement.adapter.apply(req), "SessionMismatch");
});

test("text mismatch at validation rejects without mutation", async () => {
  const w = world("resume");
  const req = request(w);
  req.expected_text = "resume!";
  rejected(await w.adapter.apply(req), "TextMismatch");
  assert.equal(w.editor.editAttempts, 0);
  assert.equal(w.doc.getText(), "resume");
});

test("text changed at immediate re-read without version bump rejects TextMismatch", async () => {
  const w = world("resume");
  w.editor.beforeBuild = (doc) => doc.setTextSilently("resumf");
  rejected(await w.adapter.apply(request(w)), "TextMismatch");
  assert.equal(w.editor.editAttempts, 1);
  assert.equal(w.doc.version, 1);
});

test("text changed at immediate re-read with version bump is AmbiguousCommit", async () => {
  const w = world("resume");
  w.editor.beforeBuild = (doc) => doc.replaceRange(0, 6, "resumf");
  assert.deepEqual(await w.adapter.apply(request(w)), { kind: "Indeterminate", reason: "AmbiguousCommit" });
  assert.equal(w.doc.version, 2);
});

test("interleaved change after queuing is AmbiguousCommit", async () => {
  const w = world("resume");
  w.editor.afterBuild = (doc) => doc.replaceRange(6, 6, "!");
  assert.deepEqual(await w.adapter.apply(request(w)), { kind: "Indeterminate", reason: "AmbiguousCommit" });
  assert.equal(w.doc.version, 3);
  assert.equal(w.doc.getText(), "restored!");
});

test("edit refusal maps to Rejected and replays from the ledger", async () => {
  const w = world("resume");
  w.editor.editResultOverride = "false";
  const req = request(w);
  rejected(await w.adapter.apply(req), "EditTransactionRefused");
  rejected(await w.adapter.apply(req), "EditTransactionRefused");
  assert.equal(w.editor.editAttempts, 1);
  assert.equal(w.doc.version, 1);
});

test("edit loss maps to Indeterminate and is never auto-retried", async () => {
  const w = world("resume");
  w.editor.editResultOverride = "throw";
  const req = request(w);
  const first = await w.adapter.apply(req);
  assert.deepEqual(first, { kind: "Indeterminate", reason: "EditOutcomeLost" });
  assert.deepEqual(await w.adapter.apply(req), first);
  assert.equal(w.editor.editAttempts, 1);
});

test("reopened document gets a new epoch and rejects old identity", async () => {
  const w = world("resume");
  const reqDoc = request(w, "req-doc");

  const reopened = new FakeDocument("file:///d:/spike/doc.txt", "resume");
  const reopenedEditor = new FakeEditor(reopened);
  reopenedEditor.setCaret(6);
  w.binding.activeEditor = reopenedEditor;
  rejected(await w.adapter.apply(reqDoc), "TargetIdentityMismatch");
  assert.equal(reopened.version, 1);

  const otherEditor = new FakeEditor(w.doc);
  otherEditor.setCaret(6);
  w.binding.activeEditor = otherEditor;
  const reqEditor = request(w, "req-editor");
  assert.notEqual(reqEditor.identity.editor_id, reqDoc.identity.editor_id);
  w.binding.activeEditor = w.editor;
  rejected(await w.adapter.apply(reqEditor), "TargetIdentityMismatch");
  assert.equal(w.editor.editAttempts, 0);
});

test("session restart clears the ledger so old requests hit SessionMismatch", async () => {
  const w = world("resume");
  const req = request(w);
  assert.deepEqual(await w.adapter.apply(req), { kind: "Applied", new_revision: 2 });
  w.binding.session_id = "fake-session-2";
  w.adapter.invalidateSession();
  rejected(await w.adapter.apply(req), "SessionMismatch");
});

test("surrogate-split range rejects as RangeMismatch", async () => {
  const w = world("😀resume");
  const req = request(w);
  req.expected_range.start = 1;
  req.expected_text = "😀resume".slice(1, 8);
  rejected(await w.adapter.apply(req), "RangeMismatch");
  assert.equal(w.editor.editAttempts, 0);
});

test("real-host policy: composition Unknown makes real applies fail closed", async () => {
  const w = world("resume");
  w.binding.composition = "Unknown";
  w.binding.capabilities = {
    flags: w.binding.capabilities.flags & ~0b0001_0000,
    unit_schema: 2,
  };
  const captured = w.adapter.captureSnapshot({ expected_text: "resume", replacement: "restored" });
  assert.ok(captured.ok);
  assert.equal(captured.snapshot.composition, "Unknown");
  rejected(await w.adapter.apply(requestFromSnapshot(captured.snapshot, "req-1")), "CompositionUnknown");
  assert.equal(w.editor.editAttempts, 0);
  assert.equal(w.doc.getText(), "resume");
});

test("canonical JSON is key-order independent and drops undefined", () => {
  assert.equal(
    canonicalJson({ b: 1, a: { d: 2, c: 3 } }),
    canonicalJson({ a: { c: 3, d: 2 }, b: 1 }),
  );
  assert.equal(canonicalJson({ a: 1, b: undefined }), '{"a":1}');
});

test("bounded ledger evicts oldest request id deterministically", async () => {
  const w = world("resume");
  w.adapter = new VsCodeHostAdapter(w.binding, 2);
  const first = request(w, "req-a");
  assert.deepEqual(await w.adapter.apply(first), { kind: "Applied", new_revision: 2 });
  w.editor.setCaret(w.doc.getText().length);
  const second = request(w, "req-b", "restored", "resume");
  assert.deepEqual(await w.adapter.apply(second), { kind: "Applied", new_revision: 3 });
  w.editor.setCaret(w.doc.getText().length);
  const third = request(w, "req-c", "resume", "restored");
  assert.deepEqual(await w.adapter.apply(third), { kind: "Applied", new_revision: 4 });

  // req-b and req-c are retained: duplicates replay recorded results even
  // though the live document has moved on.
  assert.deepEqual(await w.adapter.apply(second), { kind: "Applied", new_revision: 3 });
  assert.deepEqual(await w.adapter.apply(third), { kind: "Applied", new_revision: 4 });
  // req-a was evicted: its duplicate no longer replays the recorded Applied
  // and instead fails live validation against the current version.
  rejected(await w.adapter.apply(first), "RevisionMismatch");
  assert.equal(w.editor.editAttempts, 3);
});

test("ledger capacity must be a positive integer", () => {
  const w = world("resume");
  assert.throws(() => new VsCodeHostAdapter(w.binding, 0), RangeError);
  assert.throws(() => new VsCodeHostAdapter(w.binding, -1), RangeError);
  assert.throws(() => new VsCodeHostAdapter(w.binding, 1.5), RangeError);
});

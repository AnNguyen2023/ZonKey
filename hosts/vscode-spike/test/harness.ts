/**
 * In-memory dummy host harness: deterministic documents, editors, and
 * binding state with explicit race-injection hooks. Mirrors how the real
 * binding reads live state and applies one transaction.
 */
import type {
  Capabilities,
  CompositionState,
  SecureState,
  SessionState,
} from "../src/contract.ts";
import {
  CAP_COMPARE_AND_REPLACE,
  CAP_COMPOSITION_PROOF,
  CAP_IDEMPOTENT_REQUESTS,
  CAP_SNAPSHOT,
  CAP_UTF16_UNITS,
  UNIT_SCHEMA_UTF16,
} from "../src/contract.ts";
import type {
  HostBindingPort,
  TextDocumentPort,
  TextEditBuilderPort,
  TextEditorPort,
} from "../src/ports.ts";
import { VsCodeHostAdapter } from "../src/adapter.ts";

/** The harness can control composition state, so it may advertise that bit. */
export const HARNESS_CAPABILITIES: Capabilities = {
  flags:
    CAP_SNAPSHOT |
    CAP_COMPARE_AND_REPLACE |
    CAP_UTF16_UNITS |
    CAP_IDEMPOTENT_REQUESTS |
    CAP_COMPOSITION_PROOF,
  unit_schema: UNIT_SCHEMA_UTF16,
};

export class FakeDocument implements TextDocumentPort {
  readonly uri: string;
  private currentText: string;
  private currentVersion: number;

  constructor(uri: string, text: string, version = 1) {
    this.uri = uri;
    this.currentText = text;
    this.currentVersion = version;
  }

  get version(): number {
    return this.currentVersion;
  }

  getText(): string {
    return this.currentText;
  }

  /** One applied edit: exact-range replace plus exactly one version bump. */
  replaceRange(start: number, end: number, text: string): void {
    this.currentText = this.currentText.slice(0, start) + text + this.currentText.slice(end);
    this.currentVersion += 1;
  }

  /** Host-side anomaly injection: text change without a version bump. */
  setTextSilently(text: string): void {
    this.currentText = text;
  }
}

export class FakeEditor implements TextEditorPort {
  readonly document: FakeDocument;
  selectionCount = 1;
  selectionStart = 0;
  selectionEnd = 0;
  editAttempts = 0;
  /** Runs inside the transaction, before the build callback. */
  beforeBuild?: (document: FakeDocument) => void;
  /** Runs inside the transaction, after queuing, before applying. */
  afterBuild?: (document: FakeDocument) => void;
  editResultOverride?: "false" | "throw";

  constructor(document: FakeDocument) {
    this.document = document;
  }

  setCaret(offset: number): void {
    this.selectionStart = offset;
    this.selectionEnd = offset;
  }

  async edit(build: (builder: TextEditBuilderPort) => void): Promise<boolean> {
    this.editAttempts += 1;
    this.beforeBuild?.(this.document);
    const ops: Array<{ start: number; end: number; text: string }> = [];
    build({
      replace: (start, end, text) => ops.push({ start, end, text }),
    });
    this.afterBuild?.(this.document);
    if (this.editResultOverride === "throw") {
      throw new Error("simulated edit loss");
    }
    if (this.editResultOverride === "false") {
      return false;
    }
    for (const op of ops) {
      this.document.replaceRange(op.start, op.end, op.text);
    }
    return true;
  }
}

export class FakeBinding implements HostBindingPort {
  host_id = "fake-host";
  session_id = "fake-session";
  session: SessionState = "SupportedLocal";
  composition: CompositionState = "Inactive";
  secureState: SecureState = "KnownNonSecure";
  activeEditor: TextEditorPort | undefined;
  capabilities: Capabilities = { ...HARNESS_CAPABILITIES };

  private readonly epochs = new WeakMap<object, number>();
  private readonly epochCounters = new Map<string, number>();
  private readonly editorIdMap = new WeakMap<object, number>();
  private nextEditorId = 1;

  getActiveEditor(): TextEditorPort | undefined {
    return this.activeEditor;
  }

  documentEpoch(document: TextDocumentPort): number {
    const cached = this.epochs.get(document);
    if (cached !== undefined) {
      return cached;
    }
    const next = (this.epochCounters.get(document.uri) ?? 0) + 1;
    this.epochCounters.set(document.uri, next);
    this.epochs.set(document, next);
    return next;
  }

  editorId(editor: TextEditorPort): number {
    const cached = this.editorIdMap.get(editor);
    if (cached !== undefined) {
      return cached;
    }
    const id = this.nextEditorId;
    this.nextEditorId += 1;
    this.editorIdMap.set(editor, id);
    return id;
  }

  secureStateFor(): SecureState {
    return this.secureState;
  }
}

export interface FakeWorld {
  doc: FakeDocument;
  editor: FakeEditor;
  binding: FakeBinding;
  adapter: VsCodeHostAdapter;
}

export function world(text = "resume", uri = "file:///d:/spike/doc.txt"): FakeWorld {
  const doc = new FakeDocument(uri, text);
  const editor = new FakeEditor(doc);
  editor.setCaret(text.length);
  const binding = new FakeBinding();
  binding.activeEditor = editor;
  return { doc, editor, binding, adapter: new VsCodeHostAdapter(binding) };
}

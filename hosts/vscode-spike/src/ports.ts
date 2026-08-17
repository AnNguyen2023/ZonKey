/**
 * Narrow host ports the adapter core depends on. The core never imports the
 * `vscode` module; `vscode-binding.ts` implements these ports against real
 * VS Code APIs and the test harness implements them in memory.
 *
 * All offsets, ranges, and carets are UTF-16 code-unit offsets into the full
 * document text, matching `TextDocument.offsetAt`.
 */
import type {
  Capabilities,
  CompositionState,
  SecureState,
  SessionState,
} from "./contract.ts";

export interface TextDocumentPort {
  readonly uri: string;
  /** Live `TextDocument.version`; read fresh, never cached. */
  readonly version: number;
  /** Live full document text; read fresh, never cached. */
  getText(): string;
}

export interface TextEditorPort {
  readonly document: TextDocumentPort;
  /** Number of selections; the contract supports exactly one. */
  readonly selectionCount: number;
  /** UTF-16 offset of the single selection's start. */
  readonly selectionStart: number;
  /** UTF-16 offset of the single selection's end (the caret when empty). */
  readonly selectionEnd: number;
  /**
   * Runs one edit transaction. `build` executes synchronously inside the
   * transaction and may queue exactly one replace using UTF-16 offsets; it
   * may also queue nothing to abort. Resolves `true` when the transaction
   * was applied, `false` when it was refused.
   */
  edit(build: (builder: TextEditBuilderPort) => void): Promise<boolean>;
}

export interface TextEditBuilderPort {
  replace(start: number, end: number, text: string): void;
}

/**
 * Host-side evidence source: identity, environment state, and the current
 * active editor. Real composition state is something VS Code cannot provide;
 * see `host-policy.ts`.
 */
export interface HostBindingPort {
  readonly host_id: string;
  readonly session_id: string;
  readonly session: SessionState;
  readonly composition: CompositionState;
  readonly capabilities: Capabilities;
  getActiveEditor(): TextEditorPort | undefined;
  /**
   * Stable epoch for one open document instance. A closed and reopened
   * document must yield a new epoch even for the same URI. An in-place
   * reload (same document object) keeps the epoch; the revision then
   * advances and stale snapshots fail the revision check.
   */
  documentEpoch(document: TextDocumentPort): number;
  /** Stable identity for one editor instance. */
  editorId(editor: TextEditorPort): number;
  /**
   * Number of currently visible editors showing this document. More than
   * one (split view / same document in multiple tabs) is ambiguous for the
   * one-active-editor scope and must reject; bindings that cannot answer
   * default to one only when they already guarantee a single editor.
   */
  visibleEditorCount(document: TextDocumentPort): number;
  secureStateFor(documentUri: string): SecureState;
}

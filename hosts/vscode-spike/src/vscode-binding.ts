/**
 * Real VS Code binding for the host ports. Only this file and `extension.ts`
 * import the `vscode` module; the Node test harness never loads them.
 *
 * Honest host policy (see `host-policy.ts`): composition is always `Unknown`
 * because VS Code exposes no IME state; sessions are local-desktop only;
 * secure state is proven non-secure only for `file:` documents. Port objects
 * are cached per underlying `TextDocument`/`TextEditor`, so identity maps key
 * stably and a closed-then-reopened document (new VS Code object, same URI)
 * receives a new epoch.
 */
import * as vscode from "vscode";
import type { SecureState } from "./contract.ts";
import {
  REAL_HOST_COMPOSITION,
  deriveSecureState,
  deriveSessionState,
  realCapabilities,
} from "./host-policy.ts";
import type {
  HostBindingPort,
  TextDocumentPort,
  TextEditorPort,
} from "./ports.ts";

export function createRealBinding(): HostBindingPort {
  const documentPorts = new WeakMap<vscode.TextDocument, TextDocumentPort>();
  const editorPorts = new WeakMap<vscode.TextEditor, TextEditorPort>();
  const documentEpochs = new WeakMap<TextDocumentPort, number>();
  const editorIds = new WeakMap<TextEditorPort, number>();
  const uriEpochCounters = new Map<string, number>();
  let nextEditorId = 1;

  function documentPortFor(document: vscode.TextDocument): TextDocumentPort {
    const cached = documentPorts.get(document);
    if (cached !== undefined) {
      return cached;
    }
    const port: TextDocumentPort = {
      uri: document.uri.toString(),
      get version() {
        return document.version;
      },
      getText: () => document.getText(),
    };
    documentPorts.set(document, port);
    const key = document.uri.toString();
    const epoch = (uriEpochCounters.get(key) ?? 0) + 1;
    uriEpochCounters.set(key, epoch);
    documentEpochs.set(port, epoch);
    return port;
  }

  return {
    host_id: `vscode:${vscode.env.appName}:${vscode.env.appHost}:${vscode.version}`,
    session_id: vscode.env.sessionId,
    session: deriveSessionState(vscode.env.appHost, vscode.env.remoteName),
    composition: REAL_HOST_COMPOSITION,
    capabilities: realCapabilities(),

    getActiveEditor(): TextEditorPort | undefined {
      const editor = vscode.window.activeTextEditor;
      if (editor === undefined) {
        return undefined;
      }
      if (editor.document.uri.scheme !== "file") {
        return undefined;
      }
      const cached = editorPorts.get(editor);
      if (cached !== undefined) {
        return cached;
      }
      const documentPort = documentPortFor(editor.document);
      const document = editor.document;
      const port: TextEditorPort = {
        document: documentPort,
        get selectionCount() {
          return editor.selections.length;
        },
        get selectionStart() {
          return document.offsetAt(editor.selection.start);
        },
        get selectionEnd() {
          return document.offsetAt(editor.selection.end);
        },
        edit: (build) =>
          Promise.resolve(
            editor.edit(
              (builder) => {
                build({
                  replace: (start, end, text) =>
                    builder.replace(
                      new vscode.Range(
                        document.positionAt(start),
                        document.positionAt(end),
                      ),
                      text,
                    ),
                });
              },
              { undoStopBefore: true, undoStopAfter: true },
            ),
          ),
      };
      editorPorts.set(editor, port);
      editorIds.set(port, nextEditorId);
      nextEditorId += 1;
      return port;
    },

    documentEpoch(document: TextDocumentPort): number {
      return documentEpochs.get(document) ?? Number.NaN;
    },

    visibleEditorCount(document: TextDocumentPort): number {
      // The active editor's document is compared by URI across every
      // visible editor: a split view or the same file in multiple tabs is
      // ambiguous for the one-active-editor scope and must reject.
      return vscode.window.visibleTextEditors.filter(
        (editor) =>
          editor.document.uri.scheme === "file" &&
          editor.document.uri.toString() === document.uri,
      ).length;
    },

    editorId(editor: TextEditorPort): number {
      return editorIds.get(editor) ?? Number.NaN;
    },

    secureStateFor(documentUri: string): SecureState {
      const separator = documentUri.indexOf(":");
      const scheme =
        separator === -1 ? documentUri : documentUri.slice(0, separator);
      return deriveSecureState(scheme);
    },
  };
}

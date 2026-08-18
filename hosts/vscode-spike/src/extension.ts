/**
 * Extension entry. Two surfaces only:
 *
 * 1. The manual cooperating-host probe command (dummy-harness and real
 *    hosts; on real VS Code the honest outcome is
 *    `Rejected(CompositionUnknown)` — no TextEditor.edit is wired).
 * 2. M3D-33 endpoint discovery: on activation the extension reads the
 *    current-user discovery record written by `zonkey-cli --pipe auto`
 *    and connects once. No watchers, no auto-retry, no restore wiring;
 *    the "endpoint connect" command is the explicit reconnect action.
 */
import * as vscode from "vscode";
import { VsCodeHostAdapter, requestFromSnapshot } from "./adapter.ts";
import { canonicalJson } from "./contract.ts";
import { createRealBinding } from "./vscode-binding.ts";
import { describeOperatorResult, parseHandoffPayload } from "./handoff.ts";
import {
  connectDiscoveredEndpoint,
  describeEndpointState,
  type EndpointState,
} from "./endpoint.ts";
import type { NamedPipeClient } from "./pipe-client.ts";

const SENTINEL = "zonkey-spike-target";
const REPLACEMENT = "zonkey-spike-applied";

export const endpointState: { client?: NamedPipeClient; last: EndpointState } = {
  last: { status: "no-record" },
};

async function connect(): Promise<NamedPipeClient | undefined> {
  // Explicit reconnect is allowed to replace the current single-active
  // client; otherwise the server correctly rejects our own second socket.
  endpointState.client?.destroy();
  endpointState.client = undefined;
  const { state, client } = await connectDiscoveredEndpoint(3000);
  endpointState.last = state;
  endpointState.client = client;
  return client;
}

async function checkCurrentHandoff(): Promise<string | undefined> {
  const client = endpointState.client ?? (await connect());
  if (client === undefined) {
    void vscode.window.showWarningMessage(
      `Zonkey endpoint: ${describeEndpointState(endpointState.last)}`,
    );
    return undefined;
  }

  let handoffPayload: string;
  try {
    handoffPayload = await client.handoffQuery(10_000);
  } catch {
    void vscode.window.showWarningMessage("Zonkey: current handoff is unavailable");
    return "CurrentHandoffUnavailable";
  }
  const queried = parseHandoffPayload(handoffPayload);
  if (queried.kind === "none") {
    void vscode.window.showInformationMessage("Zonkey: no current handoff");
    return "NoCurrentHandoff";
  }
  if (queried.kind === "unavailable") {
    void vscode.window.showWarningMessage("Zonkey: current handoff is unavailable");
    return "CurrentHandoffUnavailable";
  }

  const binding = createRealBinding();
  const editor = binding.getActiveEditor();
  const beforeText = editor?.document.getText();
  const beforeVersion = editor?.document.version;
  const adapter = new VsCodeHostAdapter(binding);
  const captured = adapter.captureSnapshot({
    expected_text: queried.handoff.renderedToken,
    replacement: queried.handoff.replacementToken,
  });
  if (!captured.ok) {
    void vscode.window.showWarningMessage(`Zonkey: snapshot refused (${captured.reason})`);
    return `SnapshotRefused(${captured.reason})`;
  }

  const request = requestFromSnapshot(captured.snapshot, queried.handoff.requestId);
  let result: string;
  try {
    result = await client.request(
      queried.handoff.requestId,
      captured.snapshot.composition,
      canonicalJson(request),
      10_000,
    );
  } catch {
    void vscode.window.showWarningMessage("Zonkey: request result unavailable");
    return "RequestResultUnavailable";
  }

  if (
    editor !== undefined &&
    (editor.document.getText() !== beforeText || editor.document.version !== beforeVersion)
  ) {
    void vscode.window.showWarningMessage("Zonkey: document changed during check");
    return "DocumentChanged";
  }
  const label = describeOperatorResult(result);
  if (label === "Rejected(CompositionUnknown)") {
    void vscode.window.showWarningMessage(`Zonkey: ${label}`);
  } else {
    void vscode.window.showInformationMessage(`Zonkey: ${label}`);
  }
  return label;
}

export function activate(context: vscode.ExtensionContext): { endpointState: typeof endpointState } {
  const binding = createRealBinding();
  const adapter = new VsCodeHostAdapter(binding);
  let counter = 0;

  const probe = vscode.commands.registerCommand(
    "zonkeySpike.probe",
    async () => {
      const captured = adapter.captureSnapshot({
        expected_text: SENTINEL,
        replacement: REPLACEMENT,
      });
      if (!captured.ok) {
        void vscode.window.showWarningMessage(
          `Zonkey spike: snapshot refused (${captured.reason})`,
        );
        return;
      }
      counter += 1;
      const request = requestFromSnapshot(
        captured.snapshot,
        `${binding.session_id}:${counter}`,
      );
      const result = await adapter.apply(request);
      if (result.kind === "Applied") {
        void vscode.window.showInformationMessage(
          `Zonkey spike: Applied (new version ${result.new_revision})`,
        );
      } else if (result.kind === "Rejected") {
        void vscode.window.showWarningMessage(
          `Zonkey spike: Rejected (${result.reason})`,
        );
      } else {
        void vscode.window.showWarningMessage(
          `Zonkey spike: Indeterminate (${result.reason}); never auto-retried`,
        );
      }
    },
  );

  const status = vscode.commands.registerCommand("zonkeySpike.endpointStatus", () => {
    void vscode.window
      .showInformationMessage(`Zonkey endpoint: ${describeEndpointState(endpointState.last)}`)
      .then();
  });

  const connectCommand = vscode.commands.registerCommand(
    "zonkeySpike.endpointConnect",
    async () => {
      await connect();
      void vscode.window.showInformationMessage(
        `Zonkey endpoint: ${describeEndpointState(endpointState.last)}`,
      );
    },
  );

  const checkHandoff = vscode.commands.registerCommand(
    "zonkeySpike.checkCurrentHandoff",
    () => checkCurrentHandoff(),
  );

  context.subscriptions.push(probe, status, connectCommand, checkHandoff);
  void connect();
  return { endpointState };
}

export function deactivate(): void {
  endpointState.client?.destroy();
  endpointState.client = undefined;
}

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
import { createRealBinding } from "./vscode-binding.ts";
import {
  connectDiscoveredEndpoint,
  type EndpointState,
} from "./endpoint.ts";
import type { NamedPipeClient } from "./pipe-client.ts";

const SENTINEL = "zonkey-spike-target";
const REPLACEMENT = "zonkey-spike-applied";

export const endpointState: { client?: NamedPipeClient; last: EndpointState } = {
  last: { status: "no-record" },
};

function describe(state: EndpointState): string {
  switch (state.status) {
    case "connected":
      return `connected (pipe ${state.pipe}, session ${state.session})`;
    case "connect-failed":
      return `endpoint record found but connect failed (${state.pipe})`;
    case "no-record":
      return "no endpoint discovery record";
    case "disabled":
      return "disabled";
  }
}

async function connect(): Promise<void> {
  const { state, client } = await connectDiscoveredEndpoint(3000);
  endpointState.last = state;
  endpointState.client = client;
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
      .showInformationMessage(`Zonkey endpoint: ${describe(endpointState.last)}`)
      .then();
  });

  const connectCommand = vscode.commands.registerCommand(
    "zonkeySpike.endpointConnect",
    async () => {
      await connect();
      void vscode.window.showInformationMessage(
        `Zonkey endpoint: ${describe(endpointState.last)}`,
      );
    },
  );

  context.subscriptions.push(probe, status, connectCommand);
  void connect();
  return { endpointState };
}

export function deactivate(): void {
  endpointState.client?.destroy();
  endpointState.client = undefined;
}

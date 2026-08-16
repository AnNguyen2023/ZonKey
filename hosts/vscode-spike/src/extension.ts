/**
 * Manual dummy-harness extension entry. Registers exactly one command; no
 * watchers, no hooks, no automatic restore wiring of any kind.
 *
 * Manual evidence procedure: open one ordinary local file, place the single
 * caret immediately after the sentinel token `zonkey-spike-target`, then run
 * "Zonkey spike: probe cooperating-host apply". On real VS Code the expected
 * outcome is `Rejected(CompositionUnknown)` — the spike's honest evidence that
 * VS Code cannot prove IME composition inactivity.
 */
import * as vscode from "vscode";
import { VsCodeHostAdapter, requestFromSnapshot } from "./adapter.ts";
import { createRealBinding } from "./vscode-binding.ts";

const SENTINEL = "zonkey-spike-target";
const REPLACEMENT = "zonkey-spike-applied";

export function activate(context: vscode.ExtensionContext): void {
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
  context.subscriptions.push(probe);
}

export function deactivate(): void {
  // Nothing to dispose beyond subscriptions.
}

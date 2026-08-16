/**
 * Pure, transport-free policy for the real VS Code binding. Kept separate so
 * tests can prove what the real host honestly reports without loading the
 * `vscode` module.
 */
import type { Capabilities, CompositionState, SecureState, SessionState } from "./contract.ts";
import {
  CAP_COMPARE_AND_REPLACE,
  CAP_IDEMPOTENT_REQUESTS,
  CAP_SNAPSHOT,
  CAP_UTF16_UNITS,
  UNIT_SCHEMA_UTF16,
} from "./contract.ts";

/**
 * VS Code exposes no IME composition state to extensions. The real binding
 * always reports `Unknown`, never claims inactivity, and therefore fails
 * closed on every real-VS-Code apply.
 */
export const REAL_HOST_COMPOSITION: CompositionState = "Unknown";

/** Local desktop sessions only; remotes and unknown hosts fail closed. */
export function deriveSessionState(
  appHost: string | undefined,
  remoteName: string | undefined,
): SessionState {
  if (remoteName !== undefined) {
    return "UnsupportedRemote";
  }
  return appHost === "desktop" ? "SupportedLocal" : "Unknown";
}

/** Ordinary local file documents only; every other scheme fails closed. */
export function deriveSecureState(uriScheme: string): SecureState {
  return uriScheme === "file" ? "KnownNonSecure" : "Unknown";
}

/** Real-host capabilities; note the absent `CAP_COMPOSITION_PROOF` bit. */
export function realCapabilities(): Capabilities {
  return {
    flags:
      CAP_SNAPSHOT |
      CAP_COMPARE_AND_REPLACE |
      CAP_UTF16_UNITS |
      CAP_IDEMPOTENT_REQUESTS,
    unit_schema: UNIT_SCHEMA_UTF16,
  };
}

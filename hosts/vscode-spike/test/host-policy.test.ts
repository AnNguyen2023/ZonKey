/**
 * Tests for the pure real-host policy: what the real VS Code binding honestly
 * reports. This is where the spike's headline limitation is pinned down.
 */
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  CAP_COMPARE_AND_REPLACE,
  CAP_COMPOSITION_PROOF,
  CAP_IDEMPOTENT_REQUESTS,
  CAP_SNAPSHOT,
  CAP_UTF16_UNITS,
  UNIT_SCHEMA_UTF16,
} from "../src/contract.ts";
import {
  REAL_HOST_COMPOSITION,
  deriveSecureState,
  deriveSessionState,
  realCapabilities,
} from "../src/host-policy.ts";

test("sessions: local desktop only; remote and web fail closed", () => {
  assert.equal(deriveSessionState("desktop", undefined), "SupportedLocal");
  assert.equal(deriveSessionState("desktop", "ssh-remote"), "UnsupportedRemote");
  assert.equal(deriveSessionState("web", undefined), "Unknown");
});

test("secure: only file documents are proven non-secure", () => {
  assert.equal(deriveSecureState("file"), "KnownNonSecure");
  assert.equal(deriveSecureState("untitled"), "Unknown");
  assert.equal(deriveSecureState("vscode-userdata"), "Unknown");
});

test("composition is honestly Unknown; no proof is claimed", () => {
  assert.equal(REAL_HOST_COMPOSITION, "Unknown");
});

test("real capabilities advertise UTF-16 idempotent apply without composition proof", () => {
  const caps = realCapabilities();
  assert.ok(caps.flags & CAP_SNAPSHOT);
  assert.ok(caps.flags & CAP_COMPARE_AND_REPLACE);
  assert.ok(caps.flags & CAP_UTF16_UNITS);
  assert.ok(caps.flags & CAP_IDEMPOTENT_REQUESTS);
  assert.equal(caps.flags & CAP_COMPOSITION_PROOF, 0);
  assert.equal(caps.unit_schema, UNIT_SCHEMA_UTF16);
});

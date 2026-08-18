/**
 * M3D-33 endpoint discovery unit tests: record parsing/validation and the
 * fail-closed rules for malformed or unknown-protocol records.
 */
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
  ENDPOINT_PROTOCOL,
  describeEndpointState,
  parseRecord,
  readRecord,
} from "../src/endpoint.ts";

const SAMPLE_PIPE = "\\\\.\\pipe\\zonkey-svc-0123456789abcdef";

function recordText(overrides: Record<string, string> = {}): string {
  const fields: Record<string, string> = {
    protocol: ENDPOINT_PROTOCOL,
    pipe: SAMPLE_PIPE,
    pid: "4242",
    started_unix_ms: "1700000000000",
    ...overrides,
  };
  return (
    Object.entries(fields)
      .map(([key, value]) => `${key}=${value}`)
      .join("\r\n") + "\r\n"
  );
}

test("valid record parses", () => {
  const record = parseRecord(recordText());
  assert.ok(record !== undefined);
  assert.equal(record.pipe, SAMPLE_PIPE);
  assert.equal(record.pid, 4242);
  assert.equal(record.startedUnixMs, 1_700_000_000_000);
});

test("unknown protocol schema fails closed", () => {
  assert.equal(parseRecord(recordText({ protocol: "zonkey.host-transport/9" })), undefined);
});

test("malformed fields fail closed", () => {
  assert.equal(parseRecord("pipe=only"), undefined);
  assert.equal(parseRecord(recordText({ pid: "zero" })), undefined);
  assert.equal(parseRecord(recordText({ pipe: "not-a-pipe" })), undefined);
  assert.equal(parseRecord(""), undefined);
});

test("readRecord reads and missing files return none", () => {
  const dir = mkdtempSync(join(tmpdir(), "zonkey-endpoint-"));
  try {
    assert.equal(readRecord(dir), undefined);
    writeFileSync(join(dir, "endpoint.txt"), recordText(), "utf8");
    const record = readRecord(dir);
    assert.ok(record !== undefined);
    assert.equal(record.protocol, ENDPOINT_PROTOCOL);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("endpoint diagnostics omit pipe and session identities", () => {
  const connected = describeEndpointState({
    status: "connected",
    pipe: SAMPLE_PIPE,
    session: "sess-secret-session",
  });
  const failed = describeEndpointState({ status: "connect-failed", pipe: SAMPLE_PIPE });
  assert.equal(connected, "connected");
  assert.equal(failed, "endpoint record found but connection failed");
  assert.ok(!connected.includes(SAMPLE_PIPE));
  assert.ok(!connected.includes("sess-secret-session"));
  assert.ok(!failed.includes(SAMPLE_PIPE));
});

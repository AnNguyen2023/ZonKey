/**
 * M3D-33 endpoint discovery (read side). The CLI writes a small
 * current-user-only record at `%LOCALAPPDATA%\ZonKey\endpoint.txt` when
 * started with `--pipe auto`; this module reads, validates, and connects.
 * The record is only a hint: the pipe identity is a per-lifecycle nonce,
 * so a stale record simply fails to connect and never authorizes
 * anything. Unknown protocol/schema fails closed.
 */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { NamedPipeClient } from "./pipe-client.ts";

export const ENDPOINT_PROTOCOL = "zonkey.host-transport/1";

export interface EndpointRecord {
  protocol: string;
  pipe: string;
  pid: number;
  startedUnixMs: number;
}

/** Resolves the discovery directory (env override for isolated profiles). */
export function discoveryDir(override?: string): string | undefined {
  if (override !== undefined) {
    return override;
  }
  const envOverride = process.env.ZONKEY_ENDPOINT_DIR;
  if (envOverride !== undefined && envOverride.length > 0) {
    return envOverride;
  }
  const base = process.env.LOCALAPPDATA;
  return base === undefined || base.length === 0 ? undefined : join(base, "ZonKey");
}

/** Parses key=value record text; malformed or unknown protocol => none. */
export function parseRecord(text: string): EndpointRecord | undefined {
  const fields = new Map<string, string>();
  for (const line of text.split(/\r?\n/)) {
    if (line.length === 0) {
      continue;
    }
    const separator = line.indexOf("=");
    if (separator <= 0) {
      return undefined;
    }
    fields.set(line.slice(0, separator), line.slice(separator + 1));
  }
  const protocol = fields.get("protocol");
  const pipe = fields.get("pipe");
  const pid = Number(fields.get("pid"));
  const startedUnixMs = Number(fields.get("started_unix_ms"));
  if (
    protocol !== ENDPOINT_PROTOCOL ||
    pipe === undefined ||
    !pipe.startsWith("\\\\.\\pipe\\") ||
    !Number.isSafeInteger(pid) ||
    pid <= 0 ||
    !Number.isSafeInteger(startedUnixMs) ||
    startedUnixMs <= 0
  ) {
    return undefined;
  }
  return { protocol, pipe, pid, startedUnixMs };
}

/** Reads the current discovery record, if present and valid. */
export function readRecord(dirOverride?: string): EndpointRecord | undefined {
  const dir = discoveryDir(dirOverride);
  if (dir === undefined) {
    return undefined;
  }
  const path = join(dir, "endpoint.txt");
  if (!existsSync(path)) {
    return undefined;
  }
  try {
    return parseRecord(readFileSync(path, "utf8"));
  } catch {
    return undefined;
  }
}

export type EndpointState =
  | { status: "disabled" }
  | { status: "no-record" }
  | { status: "connect-failed"; pipe: string }
  | { status: "connected"; pipe: string; session: string };

/**
 * Reads the discovery record and connects once. No watchers, no retry
 * loops: reconnection is an explicit operator action (reconnect within a
 * live endpoint lifecycle keeps the session; a restarted endpoint yields
 * a new session identity).
 */
export async function connectDiscoveredEndpoint(
  timeoutMs: number,
  dirOverride?: string,
): Promise<{ state: EndpointState; client?: NamedPipeClient }> {
  const record = readRecord(dirOverride);
  if (record === undefined) {
    return { state: { status: "no-record" } };
  }
  try {
    const client = await NamedPipeClient.connect(record.pipe, timeoutMs);
    return {
      state: { status: "connected", pipe: record.pipe, session: client.sessionId() },
      client,
    };
  } catch {
    return { state: { status: "connect-failed", pipe: record.pipe } };
  }
}

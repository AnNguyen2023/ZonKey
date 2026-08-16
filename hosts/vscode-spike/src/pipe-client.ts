/**
 * M3D-21 named-pipe client for the VS Code extension side.
 *
 * Speaks the exact wire protocol of `zonkey-win::pipe_transport`: little-endian
 * u32 length-prefixed UTF-8 frames (64 KiB bound, fail-closed), a
 * `HELLO|<protocol>` → `WELCOME|<session>` handshake binding one server-issued
 * session id, and `REQ|<session>|<request_id>|<composition>|<canonical>`
 * requests. Timeouts and disconnects reject; the caller maps them to
 * `Indeterminate` and never retries automatically.
 */
import net from "node:net";

export const TRANSPORT_PROTOCOL_ID = "zonkey.host-transport/1";
export const MAX_FRAME_BYTES = 64 * 1024;

export type PipeClientError =
  | { kind: "ConnectTimeout"; detail?: string }
  | { kind: "ProtocolMismatch" }
  | { kind: "SessionMismatch" }
  | { kind: "Timeout" }
  | { kind: "ConnectionLost" }
  | { kind: "InvalidPayload"; detail: string };

export function encodeFrame(payload: string): Buffer {
  if (payload.length === 0) {
    throw new Error("empty frame payload");
  }
  const body = Buffer.from(payload, "utf8");
  if (body.length > MAX_FRAME_BYTES) {
    throw new Error("frame payload exceeds bound");
  }
  const header = Buffer.alloc(4);
  header.writeUInt32LE(body.length, 0);
  return Buffer.concat([header, body]);
}

export function decodeFrames(buffer: Buffer): { frames: string[]; rest: Buffer } {
  const frames: string[] = [];
  let offset = 0;
  while (offset + 4 <= buffer.length) {
    const length = buffer.readUInt32LE(offset);
    if (length === 0 || length > MAX_FRAME_BYTES) {
      throw new Error("frame length out of bounds");
    }
    if (offset + 4 + length > buffer.length) {
      break;
    }
    frames.push(buffer.subarray(offset + 4, offset + 4 + length).toString("utf8"));
    offset += 4 + length;
  }
  return { frames, rest: buffer.subarray(offset) };
}

export class NamedPipeClient {
  private socket: net.Socket;
  private receiveBuffer: Buffer = Buffer.alloc(0);
  private pending: Array<(payload: string | null) => void> = [];
  private closed = false;
  private constructor(socket: net.Socket, private session: string) {
    this.socket = socket;
    socket.on("data", (chunk: Buffer) => {
      this.receiveBuffer = Buffer.concat([this.receiveBuffer, chunk]);
      try {
        const { frames, rest } = decodeFrames(this.receiveBuffer);
        this.receiveBuffer = rest;
        for (const frame of frames) {
          this.pending.shift()?.(frame);
        }
      } catch {
        this.failPending();
        this.destroy();
      }
    });
    socket.on("error", () => this.failPending());
    socket.on("close", () => this.failPending());
  }

  /** The server-issued session id bound by the handshake. */
  sessionId(): string {
    return this.session;
  }

  /** Connects to the pipe and completes the HELLO/WELCOME handshake. */
  static async connect(pipeName: string, timeoutMs: number): Promise<NamedPipeClient> {
    const socket = await new Promise<net.Socket>((resolve, reject) => {
      const socket = net.connect(pipeName);
      const timer = setTimeout(() => {
        socket.destroy();
        reject({ kind: "ConnectTimeout" } as PipeClientError);
      }, timeoutMs);
      socket.once("connect", () => {
        clearTimeout(timer);
        resolve(socket);
      });
      socket.once("error", (error: Error) => {
        clearTimeout(timer);
        socket.destroy();
        reject({
          kind: "ConnectTimeout",
          detail: `${error?.name ?? "Error"}: ${error?.message ?? String(error)}`,
        } as PipeClientError);
      });
    });
    const client = new NamedPipeClient(socket, "");
    try {
      socket.write(encodeFrame(`HELLO|${TRANSPORT_PROTOCOL_ID}`));
      const welcome = await client.nextFrame(timeoutMs);
      if (welcome.startsWith("ERROR|")) {
        const reason = welcome.slice("ERROR|".length);
        client.destroy();
        if (reason === "protocol_mismatch") {
          throw { kind: "ProtocolMismatch" } as PipeClientError;
        }
        throw { kind: "SessionMismatch" } as PipeClientError;
      }
      const session = welcome.slice("WELCOME|".length);
      if (session.length === 0) {
        client.destroy();
        throw { kind: "InvalidPayload", detail: "empty session" } as PipeClientError;
      }
      client.session = session;
      return client;
    } catch (error) {
      client.destroy();
      throw error;
    }
  }

  /** Sends one request and awaits its RESULT payload. */
  async request(
    requestId: string,
    composition: string,
    canonical: string,
    timeoutMs: number,
  ): Promise<string> {
    if (this.closed) {
      throw { kind: "ConnectionLost" } as PipeClientError;
    }
    this.socket.write(
      encodeFrame(`REQ|${this.session}|${requestId}|${composition}|${canonical}`),
    );
    const payload = await this.nextFrame(timeoutMs);
    if (payload.startsWith("ERROR|")) {
      const reason = payload.slice("ERROR|".length);
      if (reason === "session_mismatch") {
        throw { kind: "SessionMismatch" } as PipeClientError;
      }
      throw { kind: "InvalidPayload", detail: reason } as PipeClientError;
    }
    if (!payload.startsWith("RESULT|")) {
      throw { kind: "InvalidPayload", detail: "expected result" } as PipeClientError;
    }
    return payload.slice("RESULT|".length);
  }

  /** Sends one read-only HANDOFF query; returns the result payload text. */
  async handoffQuery(timeoutMs: number): Promise<string> {
    if (this.closed) {
      throw { kind: "ConnectionLost" } as PipeClientError;
    }
    this.socket.write(encodeFrame(`HANDOFF|${this.session}`));
    const payload = await this.nextFrame(timeoutMs);
    if (payload.startsWith("ERROR|")) {
      const reason = payload.slice("ERROR|".length);
      if (reason === "session_mismatch") {
        throw { kind: "SessionMismatch" } as PipeClientError;
      }
      throw { kind: "InvalidPayload", detail: reason } as PipeClientError;
    }
    const prefix = "RESULT|DEFINITE|";
    if (!payload.startsWith(prefix)) {
      throw { kind: "InvalidPayload", detail: "expected handoff result" } as PipeClientError;
    }
    return payload.slice(prefix.length);
  }

  /** Sends one operator RECOVERY command (session is added by this method). */
  async recoveryCommand(command: string, timeoutMs: number): Promise<string> {
    if (this.closed) {
      throw { kind: "ConnectionLost" } as PipeClientError;
    }
    this.socket.write(encodeFrame(`RECOVERY|${this.session}|${command}`));
    const payload = await this.nextFrame(timeoutMs);
    if (payload.startsWith("ERROR|")) {
      const reason = payload.slice("ERROR|".length);
      if (reason === "session_mismatch") {
        throw { kind: "SessionMismatch" } as PipeClientError;
      }
      throw { kind: "InvalidPayload", detail: reason } as PipeClientError;
    }
    const prefix = "RESULT|DEFINITE|";
    if (!payload.startsWith(prefix)) {
      throw { kind: "InvalidPayload", detail: "expected recovery result" } as PipeClientError;
    }
    return payload.slice(prefix.length);
  }

  /** Drops the connection immediately; pending awaits reject. */
  destroy(): void {
    this.closed = true;
    this.socket.destroy();
  }

  private nextFrame(timeoutMs: number): Promise<string> {
    return new Promise<string>((resolve, reject) => {
      const waiter = (payload: string | null) => {
        clearTimeout(timer);
        if (payload === null) {
          reject({ kind: "ConnectionLost" } as PipeClientError);
        } else {
          resolve(payload);
        }
      };
      const timer = setTimeout(() => {
        const index = this.pending.indexOf(waiter);
        if (index !== -1) {
          this.pending.splice(index, 1);
        }
        reject({ kind: "Timeout" } as PipeClientError);
      }, timeoutMs);
      this.pending.push(waiter);
    });
  }

  private failPending(): void {
    this.closed = true;
    for (const waiter of this.pending.splice(0)) {
      waiter(null);
    }
  }
}

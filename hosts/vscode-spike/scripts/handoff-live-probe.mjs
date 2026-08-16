/**
 * M3D-23 LIVE handoff probe (manual evidence tool, clearly labeled).
 *
 * Connects to a running `zonkey-cli handoff-live` endpoint and polls the
 * read-only HANDOFF query every 2 seconds, printing each answer. The owner
 * types the test tokens on a real keyboard while this probe runs; the
 * printed transitions are the live evidence. This probe sends no requests
 * and mutates nothing.
 *
 * Usage: node scripts/handoff-live-probe.mjs --pipe \\\\.\\pipe\\zonkey-m3d23 [--seconds 120]
 */
import net from "node:net";

const args = process.argv.slice(2);
function argValue(name) {
  const index = args.indexOf(name);
  return index !== -1 ? args[index + 1] : undefined;
}
const pipe = argValue("--pipe");
const seconds = Number(argValue("--seconds") ?? "120");
if (!pipe) {
  console.error("usage: handoff-live-probe.mjs --pipe <name> [--seconds <n>]");
  process.exit(2);
}

const PROTOCOL = "zonkey.host-transport/1";
const MAX_FRAME = 64 * 1024;

function frame(payload) {
  const body = Buffer.from(payload, "utf8");
  const head = Buffer.alloc(4);
  head.writeUInt32LE(body.length, 0);
  return Buffer.concat([head, body]);
}

const variants = [pipe, pipe.replace(/^\\\\\.\\/, "\\\\?\\")];
let socket = null;
for (const variant of variants) {
  try {
    socket = await new Promise((resolve, reject) => {
      const socket = net.connect(variant);
      const timer = setTimeout(() => {
        socket.destroy();
        reject(new Error("connect timeout"));
      }, 8000);
      socket.once("connect", () => {
        clearTimeout(timer);
        resolve(socket);
      });
      socket.once("error", (error) => {
        clearTimeout(timer);
        reject(error);
      });
    });
    break;
  } catch {
    // Try the next pipe-path form.
  }
}
if (socket === null) {
  console.error("LIVE_PROBE connect failed for both pipe path forms");
  process.exit(1);
}

let buffer = Buffer.alloc(0);
let session = null;
const pending = [];
socket.on("data", (chunk) => {
  buffer = Buffer.concat([buffer, chunk]);
  while (buffer.length >= 4) {
    const length = buffer.readUInt32LE(0);
    if (buffer.length < 4 + length) break;
    const payload = buffer.subarray(4, 4 + length).toString("utf8");
    buffer = buffer.subarray(4 + length);
    pending.shift()?.(payload);
  }
});
socket.on("error", () => {});
socket.on("close", () => {
  const waiter = pending.shift();
  if (waiter) waiter("__closed__");
});

function nextPayload(timeoutMs) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      const index = pending.indexOf(waiter);
      if (index !== -1) pending.splice(index, 1);
      reject(new Error("timeout"));
    }, timeoutMs);
    const waiter = (payload) => {
      clearTimeout(timer);
      if (payload === "__closed__") reject(new Error("closed"));
      else resolve(payload);
    };
    pending.push(waiter);
  });
}

socket.write(frame(`HELLO|${PROTOCOL}`));
const welcome = await nextPayload(8000);
session = welcome.slice("WELCOME|".length);
console.log(`LIVE_PROBE session=${session} pipe=${pipe} seconds=${seconds}`);
console.log(
  "Type 'resume' + Space (RestoreCandidate), then 'dungf' or 'hello' + Space (negative), on a real keyboard.",
);

const deadline = Date.now() + seconds * 1000;
let poll = 0;
let observed = false;
while (Date.now() < deadline) {
  socket.write(frame(`HANDOFF|${session}`));
  try {
    const payload = await nextPayload(4000);
    poll += 1;
    const text = payload.replace(/^RESULT\|DEFINITE\|/, "");
    const marker = text.startsWith("handoff:") ? "HANDOFF_OBSERVED" : "no-handoff";
    console.log(`LIVE_PROBE poll=${poll} ${marker} ${text}`);
    if (text.startsWith("handoff:")) {
      observed = true;
    }
  } catch (error) {
    console.error(`LIVE_PROBE poll error: ${error.message}`);
    break;
  }
  await new Promise((resolve) => setTimeout(resolve, 2000));
}
console.log(`LIVE_PROBE done polls=${poll} handoff_observed=${observed}`);
socket.destroy();
process.exit(0);

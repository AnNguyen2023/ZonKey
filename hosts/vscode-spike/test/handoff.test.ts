import assert from "node:assert/strict";
import { test } from "node:test";
import { describeOperatorResult, parseHandoffPayload } from "../src/handoff.ts";

test("handoff payload parses scalar counts without changing the payload", () => {
  const result = parseHandoffPayload("handoff:handoff-7|réume|resume|5|6|7");
  assert.deepEqual(result, {
    kind: "current",
    handoff: {
      requestId: "handoff-7",
      renderedToken: "réume",
      replacementToken: "resume",
      renderedUnits: 5,
      replacementUnits: 6,
      generation: 7,
    },
  });
});

test("missing handoff is a sanitized operator state", () => {
  assert.deepEqual(parseHandoffPayload("handoff-rejected:NoCurrentPlan"), { kind: "none" });
  assert.deepEqual(parseHandoffPayload("handoff-rejected:NoCurrentPlan|secret"), {
    kind: "unavailable",
  });
});

test("malformed handoff fails closed", () => {
  assert.deepEqual(parseHandoffPayload("handoff:handoff-1|resume|restored|99|8|1"), {
    kind: "unavailable",
  });
});

test("transport result display is bounded and excludes payload text", () => {
  assert.equal(
    describeOperatorResult("DEFINITE|rejected:CompositionUnknown"),
    "Rejected(CompositionUnknown)",
  );
  assert.equal(describeOperatorResult("AMBIGUOUS|private-document-text"), "Indeterminate");
  assert.equal(
    describeOperatorResult("DEFINITE|private-document-text"),
    "Unexpected transport result",
  );
});

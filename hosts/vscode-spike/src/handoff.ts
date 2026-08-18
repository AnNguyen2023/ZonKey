/** Production-safe parsing and display helpers for the read-only HANDOFF path. */

export interface HandoffAnswer {
  requestId: string;
  renderedToken: string;
  replacementToken: string;
  renderedUnits: number;
  replacementUnits: number;
  generation: number;
}

export type HandoffQuery =
  | { kind: "current"; handoff: HandoffAnswer }
  | { kind: "none" }
  | { kind: "unavailable" };

function positiveSafeInteger(value: string): number | undefined {
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : undefined;
}

/** Parses the six-field transport handoff without exposing payloads on error. */
export function parseHandoffPayload(payload: string): HandoffQuery {
  if (payload === "handoff-rejected:NoCurrentPlan") {
    return { kind: "none" };
  }
  if (payload.startsWith("handoff-rejected:")) {
    return { kind: "unavailable" };
  }
  if (!payload.startsWith("handoff:")) {
    return { kind: "unavailable" };
  }
  const parts = payload.slice("handoff:".length).split("|");
  if (parts.length !== 6) {
    return { kind: "unavailable" };
  }
  const [requestId, renderedToken, replacementToken, renderedUnitsText, replacementUnitsText, generationText] = parts;
  const renderedUnits = positiveSafeInteger(renderedUnitsText);
  const replacementUnits = positiveSafeInteger(replacementUnitsText);
  const generation = positiveSafeInteger(generationText);
  if (
    !/^handoff-[1-9]\d*$/.test(requestId) ||
    renderedToken.length === 0 ||
    replacementToken.length === 0 ||
    renderedUnits === undefined ||
    replacementUnits === undefined ||
    generation === undefined ||
    [...renderedToken].length !== renderedUnits ||
    [...replacementToken].length !== replacementUnits
  ) {
    return { kind: "unavailable" };
  }
  return {
    kind: "current",
    handoff: {
      requestId,
      renderedToken,
      replacementToken,
      renderedUnits,
      replacementUnits,
      generation,
    },
  };
}

/** Converts a transport result to a bounded operator label with no payload. */
export function describeOperatorResult(result: string): string {
  const rejected = /^DEFINITE\|rejected:([A-Za-z][A-Za-z0-9]*)$/.exec(result);
  if (rejected !== null) {
    return `Rejected(${rejected[1]})`;
  }
  if (result.startsWith("AMBIGUOUS|")) {
    return "Indeterminate";
  }
  return "Unexpected transport result";
}

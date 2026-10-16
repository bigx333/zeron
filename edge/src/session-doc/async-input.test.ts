import { expect, it } from "vitest";
import { fromDocParts, toDocParts } from "./messages";

it("preserves nonblocking questions through document conversion", () => {
  const open = {
    kind: "input" as const,
    requestId: "codex-async-1",
    questions: [],
    resolved: false,
    nonBlocking: true,
  };
  // Closed unanswered: resolved with an empty answer list.
  const closed = { ...open, requestId: "codex-async-2", resolved: true, answers: [] };
  expect(fromDocParts(toDocParts([open, closed]))).toEqual([open, closed]);
});

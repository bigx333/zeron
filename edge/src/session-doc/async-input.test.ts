import { expect, it } from "vitest";
import { fromDocParts, toDocParts } from "./messages";

it("preserves nonblocking questions through document conversion", () => {
  const input = {
    kind: "input" as const,
    requestId: "codex-async-1",
    questions: [],
    resolved: false,
    nonBlocking: true,
  };
  expect(fromDocParts(toDocParts([input]))).toEqual([input]);
});

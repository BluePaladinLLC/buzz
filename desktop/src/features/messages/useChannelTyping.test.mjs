import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { getTypingCompletionStateKeys } from "./useChannelTyping.ts";

const AGENT =
  "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";

describe("getTypingCompletionStateKeys", () => {
  it("clears conversation and branch state when a focused DM reply lands", () => {
    assert.deepEqual(getTypingCompletionStateKeys(AGENT, "thread-1", "dm"), [
      `${AGENT}:thread-1`,
      `${AGENT}:channel`,
    ]);
  });

  it("keeps room-thread completion scoped to that thread", () => {
    assert.deepEqual(
      getTypingCompletionStateKeys(AGENT, "thread-1", "stream"),
      [`${AGENT}:thread-1`],
    );
  });
});

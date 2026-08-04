import assert from "node:assert/strict";
import test from "node:test";

import {
  coalesceAgentAutocompleteCandidates,
  getMentionableAgentPubkeys,
  getSharedChannelIds,
  isAgentIdentityEligibleForRoomInvite,
  isAgentIdentityEligibleForMentions,
  relayAgentCanBeInvitedToRoom,
  relayAgentIsSharedWithUser,
  shouldHideAgentFromMentions,
} from "./agentAutocompleteEligibility.ts";

const CURRENT_PUBKEY = "a".repeat(64);
const OWNER_PUBKEY = "b".repeat(64);
const OTHER_OWNER_PUBKEY = "c".repeat(64);
const PUB_A = "1".repeat(64);
const PUB_B = "2".repeat(64);
const PUB_C = "3".repeat(64);
const PUB_D = "4".repeat(64);

function coalesce(candidates, options = {}) {
  return coalesceAgentAutocompleteCandidates(candidates, {
    currentPubkey: CURRENT_PUBKEY,
    getLabel: (candidate) => candidate.displayName,
    ...options,
  });
}

function makeAgent(overrides = {}) {
  return {
    pubkey: PUB_A,
    displayName: "Pinky",
    isAgent: true,
    isMember: false,
    ...overrides,
  };
}

test("getSharedChannelIds: includes only active joined channels", () => {
  assert.deepEqual(
    getSharedChannelIds([
      { id: "joined", isMember: true, archivedAt: null },
      { id: "not-joined", isMember: false, archivedAt: null },
      { id: "archived", isMember: true, archivedAt: "2026-01-01T00:00:00Z" },
    ]),
    new Set(["joined"]),
  );
});

test("relayAgentIsSharedWithUser: accepts shared anyone agents and rejects unshared ones", () => {
  const sharedChannelIds = new Set(["general"]);

  assert.equal(
    relayAgentIsSharedWithUser(
      { respondTo: "anyone", respondToAllowlist: [], channelIds: ["general"] },
      sharedChannelIds,
    ),
    true,
  );
  assert.equal(
    relayAgentIsSharedWithUser(
      {
        respondTo: "owner-only",
        respondToAllowlist: [],
        channelIds: ["general"],
      },
      sharedChannelIds,
    ),
    false,
  );
  assert.equal(
    relayAgentIsSharedWithUser(
      { respondTo: "anyone", respondToAllowlist: [], channelIds: ["other"] },
      sharedChannelIds,
    ),
    false,
  );
});

test("relayAgentIsSharedWithUser: accepts allowlist agents for the current user", () => {
  const sharedChannelIds = new Set(["general"]);

  assert.equal(
    relayAgentIsSharedWithUser(
      {
        respondTo: "allowlist",
        respondToAllowlist: [OTHER_OWNER_PUBKEY, CURRENT_PUBKEY.toUpperCase()],
        channelIds: ["other"],
      },
      sharedChannelIds,
      CURRENT_PUBKEY,
    ),
    true,
  );
  assert.equal(
    relayAgentIsSharedWithUser(
      {
        respondTo: "allowlist",
        respondToAllowlist: [OTHER_OWNER_PUBKEY],
        channelIds: ["general"],
      },
      sharedChannelIds,
      CURRENT_PUBKEY,
    ),
    false,
  );
});

test("room invitation policy does not expand mention eligibility", () => {
  const inviteableOwnerOnlyAgent = {
    respondTo: "owner-only",
    respondToAllowlist: [],
    channelIds: [],
    channelAddPolicy: "anyone",
  };
  assert.equal(
    relayAgentIsSharedWithUser(
      inviteableOwnerOnlyAgent,
      new Set(),
      CURRENT_PUBKEY,
    ),
    false,
  );
  assert.equal(relayAgentCanBeInvitedToRoom(inviteableOwnerOnlyAgent), true);
});

test("room invitation policy fails closed for restricted or missing values", () => {
  assert.equal(
    relayAgentCanBeInvitedToRoom({ channelAddPolicy: "owner_only" }),
    false,
  );
  assert.equal(
    relayAgentCanBeInvitedToRoom({ channelAddPolicy: "nobody" }),
    false,
  );
  assert.equal(relayAgentCanBeInvitedToRoom({}), false);
});

test("getMentionableAgentPubkeys: keeps managed agents and shared relay agents", () => {
  const result = getMentionableAgentPubkeys({
    managedAgentPubkeys: [PUB_A],
    currentPubkey: CURRENT_PUBKEY,
    relayAgents: [
      {
        pubkey: PUB_B,
        respondTo: "anyone",
        respondToAllowlist: [],
        channelIds: ["general"],
      },
      {
        pubkey: PUB_C,
        respondTo: "allowlist",
        respondToAllowlist: [CURRENT_PUBKEY],
        channelIds: ["other"],
      },
      {
        pubkey: PUB_D,
        respondTo: "anyone",
        respondToAllowlist: [],
        channelIds: ["other"],
      },
    ],
    sharedChannelIds: new Set(["general"]),
  });

  assert.deepEqual(result, new Set([PUB_A, PUB_B, PUB_C]));
});

test("getMentionableAgentPubkeys: keeps owner-bound relay agents without advertised rooms", () => {
  const result = getMentionableAgentPubkeys({
    managedAgentPubkeys: [],
    currentPubkey: CURRENT_PUBKEY,
    relayAgents: [
      {
        pubkey: PUB_D,
        ownerPubkey: CURRENT_PUBKEY.toUpperCase(),
        respondTo: "anyone",
        respondToAllowlist: [],
        channelIds: [],
      },
    ],
    sharedChannelIds: new Set(),
  });

  assert.deepEqual(result, new Set([PUB_D]));
});

test("isAgentIdentityEligibleForRoomInvite: keeps people, local agents, and inviteable relay agents", () => {
  const managedAgentPubkeys = new Set([PUB_A]);
  const inviteableRelayAgentPubkeys = new Set([PUB_B]);

  assert.equal(
    isAgentIdentityEligibleForRoomInvite(
      { isAgent: false, pubkey: PUB_C },
      managedAgentPubkeys,
      inviteableRelayAgentPubkeys,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityEligibleForRoomInvite(
      { isAgent: true, pubkey: PUB_A.toUpperCase() },
      managedAgentPubkeys,
      inviteableRelayAgentPubkeys,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityEligibleForRoomInvite(
      { isAgent: true, pubkey: PUB_B.toUpperCase() },
      managedAgentPubkeys,
      inviteableRelayAgentPubkeys,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityEligibleForRoomInvite(
      { isAgent: true, pubkey: PUB_D },
      managedAgentPubkeys,
      inviteableRelayAgentPubkeys,
    ),
    false,
  );
});

test("isAgentIdentityEligibleForMentions: keeps people, managed, member, and current-owner agents", () => {
  const managedAgentPubkeys = new Set([PUB_A]);

  assert.equal(
    isAgentIdentityEligibleForMentions(
      { isAgent: false, pubkey: PUB_B },
      managedAgentPubkeys,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityEligibleForMentions(
      { isAgent: true, pubkey: PUB_A.toUpperCase() },
      managedAgentPubkeys,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityEligibleForMentions(
      { isAgent: true, isMember: true, pubkey: PUB_B },
      managedAgentPubkeys,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityEligibleForMentions(
      { isAgent: true, ownerPubkey: CURRENT_PUBKEY, pubkey: PUB_C },
      managedAgentPubkeys,
      CURRENT_PUBKEY,
    ),
    true,
  );
  assert.equal(
    isAgentIdentityEligibleForMentions(
      { isAgent: true, ownerPubkey: OTHER_OWNER_PUBKEY, pubkey: PUB_D },
      managedAgentPubkeys,
      CURRENT_PUBKEY,
    ),
    false,
  );
});

test("shouldHideAgentFromMentions: never hides non-agents", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: false,
      isMember: false,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set([PUB_A]),
    }),
    false,
  );
});

test("shouldHideAgentFromMentions: shows invocable agents even when non-member", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: false,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set([PUB_A]),
      directoryAgentPubkeys: new Set([PUB_A]),
    }),
    false,
  );
});

test("shouldHideAgentFromMentions: hides non-member non-invocable agents", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: false,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set(),
    }),
    true,
  );
});

test("shouldHideAgentFromMentions: hides member agents with an explicit not-invocable directory entry (Fizz)", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set([PUB_A]),
    }),
    true,
  );
});

test("shouldHideAgentFromMentions: shows member agents with unknown invocability (not in directory)", () => {
  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: PUB_A,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set(),
    }),
    false,
  );
});

test("shouldHideAgentFromMentions: normalizes the pubkey before lookup", () => {
  const mixedCase = "Ab".repeat(32);
  const normalized = mixedCase.toLowerCase();

  assert.equal(
    shouldHideAgentFromMentions({
      isAgent: true,
      isMember: true,
      pubkey: mixedCase,
      mentionableAgentPubkeys: new Set(),
      directoryAgentPubkeys: new Set([normalized]),
    }),
    true,
  );
});

test("coalesceAgentAutocompleteCandidates: merges agents with the same persona id", () => {
  const first = makeAgent({ pubkey: PUB_A, personaId: "pinky" });
  const second = makeAgent({
    pubkey: PUB_B,
    personaId: "pinky",
    isMember: true,
  });

  assert.deepEqual(coalesce([first, second]), [second]);
});

test("coalesceAgentAutocompleteCandidates: merges agents with the same owner and name", () => {
  const first = makeAgent({ pubkey: PUB_A, ownerPubkey: OWNER_PUBKEY });
  const second = makeAgent({
    pubkey: PUB_B,
    ownerPubkey: OWNER_PUBKEY,
    isMember: true,
  });

  assert.deepEqual(coalesce([first, second]), [second]);
});

test("coalesceAgentAutocompleteCandidates: keeps same-name agents with different owners distinct", () => {
  const first = makeAgent({ pubkey: PUB_A, ownerPubkey: OWNER_PUBKEY });
  const second = makeAgent({
    pubkey: PUB_B,
    ownerPubkey: OTHER_OWNER_PUBKEY,
  });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});

test("coalesceAgentAutocompleteCandidates: keeps owner-less same-name agents distinct", () => {
  const first = makeAgent({ pubkey: PUB_A });
  const second = makeAgent({ pubkey: PUB_B });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});

test("coalesceAgentAutocompleteCandidates: keeps owner-less managed same-name agents distinct", () => {
  const first = makeAgent({ pubkey: PUB_A, isManagedAgent: true });
  const second = makeAgent({ pubkey: PUB_B, isManagedAgent: true });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});

test("coalesceAgentAutocompleteCandidates: merges current-owner same-name agents", () => {
  const first = makeAgent({ pubkey: PUB_A, ownerPubkey: CURRENT_PUBKEY });
  const second = makeAgent({
    pubkey: PUB_B,
    ownerPubkey: CURRENT_PUBKEY,
    isManagedAgent: true,
  });

  assert.deepEqual(coalesce([first, second]), [second]);
});

test("coalesceAgentAutocompleteCandidates: leaves non-agents alone", () => {
  const first = makeAgent({ pubkey: PUB_A, isAgent: false });
  const second = makeAgent({ pubkey: PUB_B, isAgent: false });

  assert.deepEqual(coalesce([first, second]), [first, second]);
});

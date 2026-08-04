import type { RelayAgent } from "./types";

export type ChannelAddPolicy = "anyone" | "owner-only" | "nobody";

export type RawRelayAgent = {
  pubkey: string;
  name: string;
  owner_pubkey?: string | null;
  agent_type: string;
  channels: string[];
  channel_ids?: string[];
  capabilities: string[];
  status: RelayAgent["status"];
  respond_to?: RelayAgent["respondTo"];
  respond_to_allowlist?: string[];
  channel_add_policy?: ChannelAddPolicy;
};

export type RelayAgentWithChannelAddPolicy = RelayAgent & {
  channelAddPolicy?: ChannelAddPolicy | null;
};

export function fromRawRelayAgent(
  agent: RawRelayAgent,
): RelayAgentWithChannelAddPolicy {
  return {
    pubkey: agent.pubkey,
    name: agent.name,
    ownerPubkey: agent.owner_pubkey ?? null,
    agentType: agent.agent_type,
    channels: agent.channels,
    channelIds: agent.channel_ids ?? [],
    capabilities: agent.capabilities,
    status: agent.status,
    respondTo: agent.respond_to ?? null,
    respondToAllowlist: agent.respond_to_allowlist ?? [],
    channelAddPolicy: agent.channel_add_policy ?? null,
  };
}

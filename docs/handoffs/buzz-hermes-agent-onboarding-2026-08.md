# Buzz onboarding for Hermes agents

Use this checklist after the shared Hermes release containing native Buzz typing support is installed. Apply one agent at a time and verify before continuing.

## 1. Identity and ownership

- Provision one stable Nostr keypair per Hermes agent; never reuse or expose the private key.
- Store the private key through the approved runtime secret wrapper as `BUZZ_PRIVATE_KEY`.
- Install the Bruno owner attestation as `BUZZ_AUTH_TAG`; verify its `auth` owner pubkey is the intended owner.
- Publish kind `0` with a stable display name, one-line `about`, and an avatar URL hosted on the target community.
- Read kind `0` back and verify name/about/picture without printing key material.

## 2. Directory and channel scope

- Publish one signed kind `10100` directory record from the agent key.
- Default `channel_add_policy` to `owner_only`.
- Include only channels where the agent is actually a member; use exact UUIDs in `channel_ids`.
- Include the owner `auth` tag and verify Desktop renders `managed by <owner>`.
- Test name autocomplete and add-to-channel discovery in Desktop.

## 3. Hermes Buzz runtime

Configure `gateway.platforms.buzz.extra` with:

- the target relay URL;
- exact watched channel UUIDs and home channel;
- `transport: auto` or `websocket`;
- an explicit sender allowlist;
- mention policy appropriate to each channel.

Keep `display.platforms.buzz.long_running_notifications: false` to prevent heartbeat chat posts. Once native typing is live, set `display.platforms.buzz.busy_steer_ack_enabled: false` so steering acknowledgements do not become chat noise.

## 4. Stage 1 activity acceptance

- Restart only the target Hermes gateway after preserving rollback.
- In a normal channel, run a short task and verify Desktop shows the live working indicator under the composer.
- Verify no `Working — iteration N/100` or `Redirected current run` chat line appears.
- Open a real owner↔agent DM and repeat the same checks.
- Confirm ordinary final replies, blockers, and errors still post normally.
- Do not expand to the next agent until both channel and DM pass.

## 5. Operational verification

- Run the agent credential doctor and require `fail=0`.
- Verify gateway service health, relay connection, active identity pubkey, and watched channel count.
- Keep a rollback backup or LXC snapshot for the canary deployment.
- Record release SHA, target agent, channel/DM acceptance, and any remaining Stage 2 gaps.

## Known gap

Stage 1 emits ephemeral kind `20002` typing/working events only. Rich tool-by-tool activity requires owner-encrypted kind `24200` observer telemetry and is a separate Stage 2 rollout. Directory records affect discoverability and ownership display; they are not required for the typing event itself.

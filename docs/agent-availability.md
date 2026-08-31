# Agent availability and lifecycle

Availability means conversational presence on the relay, not process health.
A retained deployment receipt, PID, or `running` record cannot turn an agent's
availability dot green. A successful presence snapshot with no entry means
Offline. A pending/failed read or disconnected relay means unknown, including
when the cache previously contained Online. Online and Away require presence.

Agents cards and profiles share the existing presence query and live subscription;
there is no second availability cache or substrate poller. Lifecycle controls
remain separate: a deployed provider agent still offers Shutdown while offline.
Shutdown sends a request, not a confirmed termination, and absence of presence
is not permission to deploy a duplicate body. Local Stop/Start routing is unchanged.

## Regression evidence

- `desktop/src/features/agents/lib/useAgentAvailability.test.mjs`: successful,
  missing, unavailable, and disconnected presence; lifecycle routing and badges.
- `desktop/tests/e2e/agent-availability.spec.ts`: deployed/offline profile and
  card, live presence and disconnected-state behavior.
- `desktop/tests/e2e/agents.spec.ts`: Start badge animation and avatar continuity,
  runtime-only negative control, then authored Online presence.
- `desktop/tests/e2e/profile.spec.ts`: created-agent initial Online snapshot and
  live Offline/Online transitions alongside independent Stop/Start controls.
- `desktop/tests/e2e/onboarding.spec.ts`: Welcome kickoff stays pending after
  runtime start alone, then greets the owner and addresses Honey/Pollen only
  after the scenario publishes their explicit presence. Healthy-team scenarios
  use `tests/helpers/welcomeTeam.ts`, without changing the product readiness wait.

Mock create/start/stop only model runtime bookkeeping. Scenarios that require
availability use `__BUZZ_E2E_EMIT_MOCK_PRESENCE__` to seed the snapshot and emit
kind 20001 with the agent as author. Updating a directory row or posting a chat
message is not presence. Mock browser tests do not certify native relay TTL,
provider termination, or substrate health.

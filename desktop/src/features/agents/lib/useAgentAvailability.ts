import * as React from "react";
import { usePresenceQuery } from "@/features/presence/hooks";
import type { PresenceStatus } from "@/shared/api/types";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** Availability is relay presence, never a retained deployment receipt or PID. */
export function resolveAgentAvailability(
  status: PresenceStatus | undefined,
  presenceLoaded: boolean,
  connected: boolean,
): PresenceStatus | undefined {
  // Missing entries in a successful presence snapshot mean offline. Failed or
  // disconnected reads cannot establish availability (including cached online).
  return presenceLoaded && connected ? (status ?? "offline") : undefined;
}

/** Positive presence blocks another start, but never grants lifecycle control.
 * Missing/offline presence is not proof that starting another body is safe.
 */
export function agentPresenceStartBlockReason(
  isLifecycleActive: boolean,
  availability: PresenceStatus | undefined,
): string | undefined {
  return !isLifecycleActive &&
    (availability === "online" || availability === "away")
    ? "This agent is present on the relay. Starting another instance is unavailable."
    : undefined;
}

/** Read availability from the surface-owned snapshot, not per-row polling. */
export type AgentAvailabilityReader = (
  pubkey: string | null | undefined,
) => PresenceStatus | undefined;

/** One query/connection observer for a surface's cards and lifecycle actions. */
export function useAgentAvailabilityLookup(
  pubkeys: string[],
  options?: { enabled?: boolean },
) {
  const query = usePresenceQuery(pubkeys, options);
  const connection = useRelayConnection();
  const getAvailability: AgentAvailabilityReader = React.useCallback(
    (pubkey) =>
      pubkey
        ? resolveAgentAvailability(
            query.data?.[normalizePubkey(pubkey)],
            query.isSuccess,
            connection === "connected",
          )
        : undefined,
    [query.data, query.isSuccess, connection],
  );
  return { query, getAvailability };
}

/** Single-identity surfaces use the same authority as aggregate surfaces. */
export function useAgentAvailability(pubkey: string | null | undefined) {
  const { query, getAvailability } = useAgentAvailabilityLookup(
    pubkey ? [pubkey] : [],
  );
  return { query, status: getAvailability(pubkey) };
}

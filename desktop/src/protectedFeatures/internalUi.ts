import { useQuery } from "@tanstack/react-query";
import { createElement, useMemo } from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import type { TimelineMessage } from "@/features/messages/types";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel, ManagedAgent } from "@/shared/api/types";
import { useFeatureEnabled } from "@/shared/features";
import { BestieGlobalOverlay } from "./bestie/BestieGlobalOverlay";
import { BestieCardBadge } from "./bestie/BestieCardBadge";
import { BestieMessageAction } from "./bestie/BestieMessageAction";
import { BestieProfileAction } from "./bestie/BestieProfileSection";
import { BestieSidebarEntry } from "./bestie/BestieSidebarEntry";
import { getBestieAssignment, type BestieScope } from "./bestie/api";
import { filterBestieDmChannels } from "./bestie/filterBestieDmChannels";
import { bestieAssignmentQueryKey } from "./bestie/useBestie";

export function ProtectedGlobalOverlay() {
  const enabled = useFeatureEnabled("bestie");
  return enabled ? createElement(BestieGlobalOverlay) : null;
}

export function ProtectedMessageAction(props: {
  channelId?: string | null;
  message: TimelineMessage;
}) {
  const enabled = useFeatureEnabled("bestie");
  return enabled ? createElement(BestieMessageAction, props) : null;
}

export function ProtectedAgentBestieAction(props: { agent: ManagedAgent }) {
  const enabled = useFeatureEnabled("bestie");
  return enabled ? createElement(BestieProfileAction, props) : null;
}

export function ProtectedBestieCardBadge(props: { agent: ManagedAgent }) {
  const enabled = useFeatureEnabled("bestie");
  return enabled ? createElement(BestieCardBadge, props) : null;
}

export function ProtectedBestieSidebarEntry() {
  const enabled = useFeatureEnabled("bestie");
  return enabled ? createElement(BestieSidebarEntry) : null;
}

export function useProtectedVisibleDirectMessages(
  channels: Channel[],
  currentPubkey: string | undefined,
) {
  const enabled = useFeatureEnabled("bestie");
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const relayUrl = activeCommunity?.relayUrl ?? "";
  const ownerPubkey = identityQuery.data?.pubkey ?? "";
  const scope: BestieScope | null =
    relayUrl && ownerPubkey
      ? {
          expectedRelayUrl: relayUrl,
          expectedSignerPubkey: ownerPubkey,
        }
      : null;
  const assignmentQuery = useQuery({
    enabled: enabled && scope !== null,
    queryFn: () => getBestieAssignment(scope as BestieScope),
    queryKey: bestieAssignmentQueryKey(relayUrl, ownerPubkey),
  });
  const bestiePubkey = enabled
    ? (assignmentQuery.data?.agentPubkey ?? null)
    : null;

  return useMemo(
    () => filterBestieDmChannels(channels, currentPubkey, bestiePubkey),
    [bestiePubkey, channels, currentPubkey],
  );
}

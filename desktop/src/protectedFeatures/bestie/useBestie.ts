import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  managedAgentRuntimesQueryKey,
  useManagedAgentRuntimeAction,
  useManagedAgentRuntimesQuery,
} from "@/features/agents/managedAgentRuntimeHooks";
import {
  canonicalRelayUrl,
  findManagedAgentRuntime,
  managedAgentPairAction,
} from "@/features/agents/managedAgentRuntimeStatus";
import {
  channelsQueryKey,
  upsertCachedChannel,
} from "@/features/channels/hooks";
import { dmVisibilityQueryKeyFor } from "@/features/channels/useHiddenDmIds";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel, ManagedAgent } from "@/shared/api/types";
import {
  assignBestie,
  clearBestieAssignment,
  getBestieAssignment,
  resolveBestieConversation,
  type BestieScope,
} from "./api";

export function bestieAssignmentQueryKey(
  relayUrl: string,
  ownerPubkey: string,
) {
  return [
    "bestie-assignment",
    canonicalRelayUrl(relayUrl) ?? relayUrl,
    ownerPubkey.toLowerCase(),
  ] as const;
}

export function useBestie() {
  const queryClient = useQueryClient();
  const { goChannel } = useAppNavigation();
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
  const queryKey = bestieAssignmentQueryKey(relayUrl, ownerPubkey);
  const assignmentQuery = useQuery({
    enabled: scope !== null,
    queryKey,
    queryFn: () => getBestieAssignment(scope as BestieScope),
  });
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: scope !== null });
  const runtimesQuery = useManagedAgentRuntimesQuery({
    enabled: scope !== null,
  });
  const runtimeAction = useManagedAgentRuntimeAction();
  const eligibleAgents = (managedAgentsQuery.data ?? []).filter(
    (agent) => agent.backend.type === "local",
  );
  const assignedAgent = assignmentQuery.data
    ? (eligibleAgents.find(
        (agent) => agent.pubkey === assignmentQuery.data?.agentPubkey,
      ) ?? null)
    : null;
  const runtime = assignedAgent
    ? findManagedAgentRuntime(
        runtimesQuery.data ?? [],
        assignedAgent.pubkey,
        relayUrl,
      )
    : undefined;

  const assignMutation = useMutation({
    mutationFn: (agent: ManagedAgent) => {
      if (!scope) throw new Error("Bestie is unavailable outside a community");
      return assignBestie(agent.pubkey, scope);
    },
    onSuccess: (assignment) => {
      queryClient.setQueryData(queryKey, assignment);
    },
  });
  const clearMutation = useMutation({
    mutationFn: () => {
      if (!scope) throw new Error("Bestie is unavailable outside a community");
      return clearBestieAssignment(scope);
    },
    onSuccess: () => {
      queryClient.setQueryData(queryKey, null);
    },
  });
  const resolveMutation = useMutation({
    mutationFn: () => {
      if (!scope) throw new Error("Bestie is unavailable outside a community");
      return resolveBestieConversation(scope);
    },
    onSuccess: (channel) => {
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
        upsertCachedChannel(current, channel),
      );
      queryClient.setQueryData<Set<string>>(
        dmVisibilityQueryKeyFor(relayUrl, ownerPubkey),
        (current) => {
          const next = new Set(current);
          next.delete(channel.id);
          return next;
        },
      );
    },
  });

  const openConversation = async (draft?: string) => {
    if (assignedAgent) {
      const action = managedAgentPairAction(runtime);
      if (action !== "stop") {
        await runtimeAction.mutateAsync({
          action,
          pubkey: assignedAgent.pubkey,
          relayUrl,
        });
        await queryClient.invalidateQueries({
          queryKey: managedAgentRuntimesQueryKey,
        });
      }
    }
    const channel = await resolveMutation.mutateAsync();
    await goChannel(channel.id, draft ? { autoSend: draft } : undefined);
  };

  return {
    assignedAgent,
    assignment: assignmentQuery.data ?? null,
    assignmentError: assignmentQuery.error,
    assignAgent: assignMutation.mutateAsync,
    clearAssignment: clearMutation.mutateAsync,
    isAssigning: assignMutation.isPending,
    isLoading:
      assignmentQuery.isLoading ||
      managedAgentsQuery.isLoading ||
      runtimesQuery.isLoading,
    isOpening: resolveMutation.isPending || runtimeAction.isPending,
    openConversation,
    runtime,
  };
}

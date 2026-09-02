import type { TimelineMessage } from "@/features/messages/types";
import type { ManagedAgent } from "@/shared/api/types";

export function ProtectedGlobalOverlay() {
  return null;
}

export function ProtectedMessageAction(_props: {
  channelId?: string | null;
  message: TimelineMessage;
}) {
  return null;
}

export function ProtectedAgentBestieAction(_props: { agent: ManagedAgent }) {
  return null;
}

export function ProtectedBestieCardBadge(_props: { agent: ManagedAgent }) {
  return null;
}

export function ProtectedBestieSidebarEntry() {
  return null;
}

import { createElement } from "react";

import type { TimelineMessage } from "@/features/messages/types";
import type { ManagedAgent } from "@/shared/api/types";
import { useFeatureEnabled } from "@/shared/features";
import { BestieGlobalOverlay } from "./bestie/BestieGlobalOverlay";
import { BestieCardBadge } from "./bestie/BestieCardBadge";
import { BestieMessageAction } from "./bestie/BestieMessageAction";
import { BestieProfileAction } from "./bestie/BestieProfileSection";
import { BestieSidebarEntry } from "./bestie/BestieSidebarEntry";

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

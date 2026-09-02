import { ArrowUp, Plus, X } from "lucide-react";
import { motion } from "motion/react";
import * as React from "react";
import { toast } from "sonner";

import type { TimelineMessage } from "@/features/messages/types";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import type {
  ManagedAgent,
  ManagedAgentRuntimeStatus,
  PresenceStatus,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Textarea } from "@/shared/ui/textarea";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { useBestie } from "./useBestie";

export function bestiePresenceStatus(
  lifecycle: ManagedAgentRuntimeStatus["lifecycle"] | undefined,
): PresenceStatus {
  if (lifecycle === "ready" || lifecycle === "listening") return "online";
  if (lifecycle === "starting" || lifecycle === "waking") return "away";
  return "offline";
}

export function BestieTriggerVisual({
  agent,
  className,
  compact = false,
  imageDraggable,
}: {
  agent: ManagedAgent | null;
  className?: string;
  compact?: boolean;
  imageDraggable?: boolean;
}) {
  if (agent) {
    return (
      <UserAvatar
        avatarUrl={agent.avatarUrl}
        className={cn(compact ? "h-6 w-6" : "h-10 w-10", className)}
        displayName={agent.name}
        fallbackDelayMs={0}
        imageDraggable={imageDraggable}
        size={compact ? "sm" : "md"}
        testId="bestie-trigger-avatar"
      />
    );
  }

  return (
    <Plus
      aria-hidden="true"
      className={cn(compact ? "h-4 w-4" : "h-5 w-5", className)}
      data-testid="bestie-empty-mark"
    />
  );
}

export function BestieAgentLockup({
  agent,
  avatarLayoutId,
  presenceStatus,
  compact = false,
}: {
  agent: ManagedAgent;
  avatarLayoutId?: string;
  presenceStatus: PresenceStatus;
  compact?: boolean;
}) {
  return (
    <div className="flex min-w-0 items-center gap-2.5">
      <motion.div
        aria-hidden="true"
        className="relative shrink-0"
        data-testid="bestie-agent-avatar"
        layoutId={avatarLayoutId}
      >
        <UserAvatar
          avatarUrl={agent.avatarUrl}
          className={compact ? "h-5 w-5" : "h-8 w-8"}
          displayName={agent.name}
          fallbackDelayMs={0}
          size={compact ? "xs" : "sm"}
        />
        <span
          className={cn(
            "absolute flex items-center justify-center rounded-full",
            compact
              ? "-bottom-0.5 -right-0.5 h-2.5 w-2.5 bg-sidebar"
              : "-bottom-0.5 -right-0.5 h-3.5 w-3.5 bg-popover",
          )}
        >
          <PresenceDot
            className={compact ? "h-1.5 w-1.5" : "h-2 w-2"}
            data-testid="bestie-activity-dot"
            status={presenceStatus}
          />
        </span>
      </motion.div>
      <span className="min-w-0 truncate text-sm font-medium">{agent.name}</span>
      <span className="sr-only">{presenceStatus}</span>
    </div>
  );
}

function EmptyBestie() {
  return (
    <div className="flex min-h-32 flex-col items-center justify-center gap-3 px-4 text-center">
      <div className="flex h-10 w-10 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <Plus aria-hidden="true" className="h-5 w-5" />
      </div>
      <div>
        <h2 className="text-sm font-semibold">Choose a Bestie</h2>
        <p className="mt-1 text-xs text-muted-foreground">
          Open one of your local agents and turn on Bestie.
        </p>
      </div>
    </div>
  );
}

export function BestiePopover({
  avatarLayoutId,
  contextMessage,
  onRequestClose,
}: {
  avatarLayoutId?: string;
  contextMessage?: TimelineMessage;
  onRequestClose?: () => void;
}) {
  const bestie = useBestie();
  const [draft, setDraft] = React.useState("");

  if (bestie.isLoading) {
    return <p className="text-sm text-muted-foreground">Loading Bestie…</p>;
  }
  if (!bestie.assignedAgent) return <EmptyBestie />;

  const agent = bestie.assignedAgent;
  const presenceStatus = bestiePresenceStatus(bestie.runtime?.lifecycle);
  const openConversation = (message?: string) => {
    onRequestClose?.();
    void bestie.openConversation(message).catch((error) => {
      toast.error(
        error instanceof Error ? error.message : "Couldn’t open Bestie",
      );
    });
  };
  const contextualDraft = contextMessage
    ? `Help me with this message from ${contextMessage.author}:\n\n> ${contextMessage.body.replaceAll("\n", "\n> ")}\n\n${draft.trim()}`
    : draft.trim();

  return (
    <div className="space-y-4">
      <div
        className="flex touch-none select-none items-center gap-3 cursor-grab active:cursor-grabbing"
        data-bestie-drag-handle
      >
        <BestieAgentLockup
          agent={agent}
          avatarLayoutId={avatarLayoutId}
          presenceStatus={presenceStatus}
        />
        <div className="flex-1" />
        <Button
          aria-label="Close Bestie"
          onClick={onRequestClose}
          size="icon-xs"
          variant="ghost"
        >
          <X />
        </Button>
      </div>

      {contextMessage ? (
        <div className="space-y-2" data-testid="bestie-message-context">
          <div
            className="max-h-24 max-w-[75%] overflow-hidden rounded-xl border border-border/70 bg-muted/45 p-2.5 shadow-xs"
            data-testid="bestie-message-snapshot"
          >
            <div className="flex min-w-0 items-center gap-2">
              <UserAvatar
                avatarUrl={contextMessage.avatarUrl ?? null}
                className="h-5 w-5"
                displayName={contextMessage.author}
                fallbackDelayMs={0}
                size="xs"
              />
              <span className="truncate text-xs font-semibold">
                {contextMessage.author}
              </span>
            </div>
            <p className="mt-1.5 whitespace-pre-wrap break-words text-xs leading-4 text-foreground/80">
              {contextMessage.body}
            </p>
          </div>
          <div className="w-fit rounded-2xl bg-muted px-3 py-2 text-sm">
            How can I help?
          </div>
        </div>
      ) : null}

      <div className="relative">
        <Textarea
          aria-label={`Message ${agent.name}`}
          className="min-h-24 resize-none rounded-2xl pb-11"
          onChange={(event) => setDraft(event.target.value)}
          placeholder={`Message ${agent.name}`}
          value={draft}
        />
        <Button
          aria-label="Send in Bestie conversation"
          className="absolute bottom-2 right-2 rounded-full"
          disabled={!draft.trim() || bestie.isOpening}
          onClick={() => openConversation(contextualDraft)}
          size="icon"
        >
          <ArrowUp />
        </Button>
      </div>
    </div>
  );
}

import { Star } from "lucide-react";

import type { ManagedAgent } from "@/shared/api/types";
import { useBestie } from "./useBestie";

export function BestieCardBadge({ agent }: { agent: ManagedAgent }) {
  const bestie = useBestie();
  const isBestie =
    bestie.assignment?.agentPubkey.toLowerCase() === agent.pubkey.toLowerCase();

  if (!isBestie) return null;

  return (
    <span
      aria-label={`${agent.name} is your Bestie`}
      className="inline-flex shrink-0 items-center justify-center text-foreground opacity-50"
      data-testid={`bestie-card-badge-${agent.pubkey}`}
      role="img"
      title="Bestie"
    >
      <Star className="h-4 w-4 fill-current" />
    </span>
  );
}

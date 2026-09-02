import * as React from "react";
import { detectPrefixQuery } from "@/shared/lib/detectPrefixQuery";
import type { MentionSuggestion } from "../ui/MentionAutocomplete";

type Snapshot = { text: string; cursor: number };
type Context = Snapshot & {
  query: string;
  startIndex: number;
  origin: "inline" | "explicit" | "default";
  visit: object;
};
const normalize = (query: string) => query.trim().toLowerCase();

/** Owns the editor context that produced a picker choice, independently of admission. */
export function useMentionQuery(
  getSnapshot: (() => Snapshot) | undefined,
  visit: object,
) {
  const visitRef = React.useRef(visit);
  visitRef.current = visit;
  const [context, setContext] = React.useState<Context | null>(null);
  const contextRef = React.useRef(context);
  const originRef = React.useRef<Context["origin"] | null>(null);
  const inputRef = React.useRef<Snapshot>({ text: "", cursor: 0 });
  const latestValueRef = React.useRef("");
  const latestCursorRef = React.useRef(0);
  const debounceTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const searchableNamesLowerRef = React.useRef<string[]>([]);
  const snapshotRef = React.useRef(getSnapshot);
  snapshotRef.current = getSnapshot;
  const clearSelectionRef = React.useRef(() => {});
  const clearSelection = React.useCallback(
    () => clearSelectionRef.current(),
    [],
  );
  const choices = React.useRef(new WeakMap<MentionSuggestion, Context>());
  const read = React.useCallback(() => {
    const snapshot = snapshotRef.current?.();
    if (snapshot) {
      latestValueRef.current = snapshot.text;
      latestCursorRef.current = snapshot.cursor;
    }
    return { text: latestValueRef.current, cursor: latestCursorRef.current };
  }, []);
  const prefixFor = React.useCallback(
    (snapshot: Snapshot) =>
      detectPrefixQuery(
        "@",
        snapshot.text,
        snapshot.cursor,
        searchableNamesLowerRef.current,
      ),
    [],
  );
  const currentPrefix = React.useCallback(
    () => prefixFor(read()),
    [prefixFor, read],
  );
  const stopTimer = React.useCallback(() => {
    if (debounceTimerRef.current !== null)
      clearTimeout(debounceTimerRef.current);
    debounceTimerRef.current = null;
  }, []);
  React.useEffect(() => stopTimer, [stopTimer]);
  const publish = React.useCallback((next: Context | null) => {
    const old = contextRef.current;
    // A settled debounce for the same document/query must preserve deliberate key intent.
    if (
      old &&
      next &&
      old.visit === next.visit &&
      old.text === next.text &&
      old.cursor === next.cursor &&
      old.startIndex === next.startIndex &&
      old.origin === next.origin &&
      normalize(old.query) === normalize(next.query)
    )
      return old;
    contextRef.current = next;
    setContext(next);
    return next;
  }, []);
  const refresh = React.useCallback(() => {
    stopTimer();
    const snapshot = read();
    const prefix = prefixFor(snapshot);
    originRef.current = prefix ? "inline" : null;
    return publish(
      prefix
        ? { ...snapshot, ...prefix, origin: "inline", visit: visitRef.current }
        : null,
    );
  }, [prefixFor, publish, read, stopTimer]);
  const cancel = React.useCallback(() => {
    stopTimer();
    originRef.current = null;
    publish(null);
    clearSelection();
  }, [clearSelection, publish, stopTimer]);
  const update = React.useCallback(
    (text: string, cursor: number) => {
      clearSelection();
      inputRef.current = { text, cursor };
      latestValueRef.current = text;
      latestCursorRef.current = cursor;
      const prefix = prefixFor({ text, cursor });
      stopTimer();
      if (!prefix && originRef.current === "explicit") {
        // A settings menu has no typed trigger to abandon. Keep it open with
        // fresh rows for the edited document, never rebind retained old rows.
        publish({
          text,
          cursor,
          query: contextRef.current?.query ?? "",
          startIndex: cursor,
          origin: "explicit",
          visit: visitRef.current,
        });
        return;
      }
      originRef.current = prefix ? "inline" : null;
      debounceTimerRef.current = setTimeout(refresh, 120);
    },
    [clearSelection, prefixFor, publish, refresh, stopTimer],
  );
  const open = React.useCallback(
    (cursor: number, preserve = false) => {
      stopTimer();
      const snapshot = read();
      originRef.current = "explicit";
      publish({
        ...snapshot,
        cursor,
        query: preserve ? (contextRef.current?.query ?? "") : "",
        startIndex: cursor,
        origin: "explicit",
        visit: visitRef.current,
      });
    },
    [publish, read, stopTimer],
  );
  const matches = React.useCallback(
    (owner: Context | null) => {
      if (!owner || owner.visit !== visitRef.current) return false;
      const live = read();
      if (live.text !== owner.text) return false;
      if (owner.origin === "default")
        return contextRef.current === null && live.cursor === owner.cursor;
      if (owner !== contextRef.current || owner.origin !== originRef.current)
        return false;
      const prefix = prefixFor(live);
      if (owner.origin === "explicit") {
        // An explicit no-trigger menu may insert at a newly chosen endpoint, but
        // cannot acquire a typed trigger that wasn't present when it was opened.
        const original = prefixFor(owner);
        return !prefix
          ? !original
          : !!original &&
              prefix.startIndex === original.startIndex &&
              normalize(prefix.query) === normalize(original.query);
      }
      return (
        !!prefix &&
        live.cursor === owner.cursor &&
        prefix.startIndex === owner.startIndex &&
        normalize(prefix.query) === normalize(owner.query)
      );
    },
    [prefixFor, read],
  );
  const bind = React.useCallback(
    (suggestion: MentionSuggestion, owner = context) => {
      if (owner && owner.visit === visit)
        choices.current.set(suggestion, owner);
      return suggestion;
    },
    [context, visit],
  );
  const canCommit = React.useCallback(
    (suggestion: MentionSuggestion) => {
      if (matches(choices.current.get(suggestion) ?? null)) return true;
      clearSelection();
      refresh();
      return false;
    },
    [clearSelection, matches, refresh],
  );
  const bindFresh = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const owner = refresh();
      if (owner) choices.current.set(suggestion, owner);
      return suggestion;
    },
    [refresh],
  );
  const bindDefault = React.useCallback(
    (suggestion: MentionSuggestion | null) => {
      if (!suggestion) return null;
      const snapshot = read();
      choices.current.set(suggestion, {
        ...snapshot,
        query: "",
        startIndex: snapshot.cursor,
        origin: "default",
        visit,
      });
      return suggestion;
    },
    [read, visit],
  );
  return {
    context,
    clearSelectionRef,
    originRef,
    latestValueRef,
    latestCursorRef,
    debounceTimerRef,
    searchableNamesLowerRef,
    currentPrefix,
    read,
    refresh,
    cancel,
    update,
    open,
    bind,
    bindFresh,
    bindDefault,
    canCommit,
    isCurrent: () => matches(context),
    canResolve: () => {
      const live = read();
      return (
        live.text === inputRef.current.text &&
        live.cursor === inputRef.current.cursor
      );
    },
  };
}

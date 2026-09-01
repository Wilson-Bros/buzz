/**
 * Mounted regression: real AgentDefaultsEditor and DefaultConfigStep parents
 * exercise the same effort journeys as effortAutoClear.test.mjs, through the
 * production component trees that users actually encounter.
 *
 * Finding 2 (PR #4625): effortAutoClear.test.mjs tests AgentConfigFields
 * directly via a hand-rolled SettingsParent. These tests mount the real parents
 * to confirm the same invariants hold through the production entry points.
 *
 * AgentDefaultsEditor (Settings surface):
 *   - Loads config via `get_global_agent_config` IPC on mount.
 *   - Selects harness from the ACP runtime cache (QueryClientProvider).
 *   - Renders AgentConfigFields with useCustomSelect=true.
 *   - Effort "off" must appear as "Off" in the custom trigger.
 *
 * DefaultConfigStep (onboarding surface):
 *   - Mounts with Goose runtime selected and effort "off" in env_vars.
 *   - The custom trigger must show "Off" — the isHarnessNativeEffort branch
 *     routes to effortCanonicalValues which includes "off".
 *
 * Provider-empty convergence (Carl P2 regression):
 *   - Covered directly by effortAutoClear.test.mjs through AgentConfigFields.
 *   - These tests confirm the invariant holds through the real parent trees.
 *
 * Mutation proofs:
 *   - Removing isHarnessNativeEffort branch in AgentConfigFields → effort custom
 *     trigger shows inherit placeholder instead of "Off" → both tests RED.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

// ── Global env setup ─────────────────────────────────────────────────────────
Object.assign(globalThis, {
  document: dom.window.document,
  window: dom.window,
  IS_REACT_ACT_ENVIRONMENT: true,
  localStorage: dom.window.localStorage,
  self: dom.window,
  ResizeObserver: class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
  writable: true,
});
dom.window.requestAnimationFrame = (cb) => setTimeout(cb, 0);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
dom.window.matchMedia ??= (query) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: () => {},
  removeListener: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => false,
});
globalThis.matchMedia = dom.window.matchMedia;
for (const key of Object.getOwnPropertyNames(dom.window)) {
  if (key === "window" || key === "document" || key === "globalThis") continue;
  const value = dom.window[key];
  if (
    typeof value === "function" &&
    /^(HTML|SVG)|Element$|Event$|EventTarget$|^Node|^Document|Observer$/.test(
      key,
    )
  ) {
    globalThis[key] = value;
  }
}
globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
const _origDispatch = dom.window.EventTarget.prototype.dispatchEvent;
dom.window.EventTarget.prototype.dispatchEvent = function (event) {
  if (!(event instanceof dom.window.Event)) return false;
  return _origDispatch.call(this, event);
};
globalThis.EventTarget = dom.window.EventTarget;

// ── QueryClient tracking ──────────────────────────────────────────────────────
// react-query's default gcTime schedules timers that outlive each test and
// stall the process. Track every client; cancel + clear in afterEach.
const clients = [];

// ── Tauri IPC stub ────────────────────────────────────────────────────────────
const DEFAULT_CONFIG = {
  env_vars: {},
  provider: null,
  model: null,
  preferred_runtime: "goose",
};

function makeIpcHandler(overrides = {}) {
  return (cmd, payload) => {
    if (cmd in overrides) return overrides[cmd](payload);
    if (cmd === "get_global_agent_config")
      return Promise.resolve(DEFAULT_CONFIG);
    if (cmd === "set_global_agent_config")
      return Promise.resolve({
        config: payload?.config ?? DEFAULT_CONFIG,
        restarted_count: 0,
        failed_restart_count: 0,
      });
    if (cmd === "get_baked_build_env" || cmd === "get_baked_build_env_keys")
      return Promise.resolve([]);
    if (cmd === "discover_acp_providers")
      return Promise.resolve([rawGooseCatalogEntry()]);
    if (cmd === "discover_agent_models")
      return Promise.resolve({ options: [], is_optional: true });
    if (cmd === "get_runtime_file_config") return Promise.resolve(null);
    return Promise.reject(new Error(`unmocked: ${cmd}`));
  };
}

globalThis.__TAURI_INTERNALS__ = {
  invoke: makeIpcHandler(),
  transformCallback: () => 1,
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

// ── Deferred imports ──────────────────────────────────────────────────────────
let act, render, screen, cleanup, createElement;
let AgentDefaultsEditor;
let DefaultConfigStep;
let QueryClient, QueryClientProvider;
let acpRuntimesQueryKey, fromRawAcpRuntimeCatalogEntry;

before(async () => {
  ({ act, render, screen, cleanup } = await import("@testing-library/react"));
  ({ createElement } = await import("react"));
  ({ AgentDefaultsEditor } = await import("./AgentDefaultsEditor.tsx"));
  ({ DefaultConfigStep } = await import(
    "../../onboarding/ui/DefaultConfigStep.tsx"
  ));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ acpRuntimesQueryKey } = await import(
    "@/features/agents/acpRuntimesQuery.ts"
  ));
  ({ fromRawAcpRuntimeCatalogEntry } = await import("@/shared/api/tauri.ts"));
});

afterEach(() => {
  cleanup?.();
  // Cancel + clear all QueryClients to prevent gcTime timers from stalling
  // the process after tests complete.
  for (const client of clients.splice(0)) {
    client.cancelQueries();
    client.clear();
  }
  // Restore default IPC stub.
  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler();
  dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

// ── Fixtures ──────────────────────────────────────────────────────────────────

/** Minimal raw Goose catalog entry with effort_canonical_values. */
function rawGooseCatalogEntry() {
  return {
    id: "goose",
    label: "Goose",
    avatar_url: "",
    availability: "available",
    command: "goose",
    binary_path: "/usr/local/bin/goose",
    default_args: [],
    mcp_command: null,
    model_env_var: "GOOSE_MODEL",
    provider_env_var: "GOOSE_PROVIDER",
    thinking_env_var: "GOOSE_THINKING_EFFORT",
    max_tokens_env_var: null,
    context_limit_env_var: null,
    max_rounds_env_var: null,
    install_hint: "",
    install_instructions_url: "",
    can_auto_install: false,
    requires_external_cli: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "not_applicable" },
    login_hint: null,
    source: "builtin",
    effort_canonical_values: ["off", "low", "medium", "high", "max"],
  };
}

function makeQueryClient() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  clients.push(client);
  return client;
}

function seedGooseRuntime(queryClient) {
  const entry = fromRawAcpRuntimeCatalogEntry(rawGooseCatalogEntry());
  queryClient.setQueryData(acpRuntimesQueryKey, [entry]);
  return entry;
}

function withQueryClient(client, children) {
  return createElement(QueryClientProvider, { client }, children);
}

/** Drain React update queue. */
async function settle() {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 50));
  });
  await act(async () => {});
}

// ── Tests ──────────────────────────────────────────────────────────────────────

test("AgentDefaultsEditor: Goose effort 'off' renders 'Off' in custom trigger after mount", async () => {
  // Production Settings journey through the real AgentDefaultsEditor parent.
  // AgentDefaultsEditor loads config from get_global_agent_config IPC, selects
  // the Goose harness from the seeded cache, and renders AgentConfigFields with
  // useCustomSelect=true.
  //
  // Mutation proof: removing the isHarnessNativeEffort branch in AgentConfigFields
  // → effortValidForRenderer uses buzz-agent vocab (no "off") → the custom
  // trigger shows the inherit placeholder instead of "Off" → this test fails:
  // `trigger text must contain human label "Off"; got: "Select"`.

  const savedConfig = {
    env_vars: { GOOSE_THINKING_EFFORT: "off" },
    provider: "anthropic",
    model: "claude-3-5-sonnet",
    preferred_runtime: "goose",
  };

  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler({
    get_global_agent_config: () => Promise.resolve(savedConfig),
    set_global_agent_config: (payload) =>
      Promise.resolve({
        config: payload?.config ?? savedConfig,
        restarted_count: 0,
        failed_restart_count: 0,
      }),
  });
  dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

  const queryClient = makeQueryClient();
  seedGooseRuntime(queryClient);

  render(
    withQueryClient(
      queryClient,
      createElement(AgentDefaultsEditor, { layout: "grouped" }),
    ),
  );

  await settle();

  // The effort trigger must show "Off" with data-value="off".
  const trigger = screen.queryByTestId("global-agent-thinking-effort-select");
  assert.ok(
    trigger,
    "effort custom select trigger must be present after AgentDefaultsEditor loads",
  );
  assert.equal(
    trigger.getAttribute("data-value"),
    "off",
    'trigger data-value must be "off" (Goose canonical)',
  );
  assert.ok(
    trigger.textContent?.includes("Off"),
    `trigger text must contain human label "Off"; got: "${trigger.textContent}"`,
  );
});

test("DefaultConfigStep: Goose effort 'off' renders 'Off' in custom trigger after mount", async () => {
  // Production onboarding journey through the real DefaultConfigStep parent.
  // Mounts with a provider present (anthropic) so the effort field is enabled,
  // and Goose effort "off" in env_vars — the isHarnessNativeEffort branch must
  // route to effortCanonicalValues which includes "off".
  //
  // Mutation proof: removing the isHarnessNativeEffort branch in AgentConfigFields
  // → effortValidForRenderer uses buzz-agent vocab (no "off") → the custom
  // trigger shows the inherit placeholder instead of "Off" → this test fails.
  //
  // Provider-empty convergence (Carl P2) is covered by effortAutoClear.test.mjs.

  const gooseConfig = {
    env_vars: { GOOSE_THINKING_EFFORT: "off" },
    provider: "anthropic",
    model: "claude-3-5-sonnet",
    preferred_runtime: "goose",
  };

  globalThis.__TAURI_INTERNALS__.invoke = makeIpcHandler({
    get_global_agent_config: () => Promise.resolve(gooseConfig),
  });
  dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

  const queryClient = makeQueryClient();
  seedGooseRuntime(queryClient);

  const actions = {
    back: () => {},
    complete: () => {},
    discardDraft: () => {},
    updateDraft: () => {},
  };

  render(
    withQueryClient(
      queryClient,
      createElement(DefaultConfigStep, {
        actions,
        direction: "forward",
        draft: null,
        readyRuntimeIds: ["goose"],
      }),
    ),
  );

  await settle();

  // The effort trigger must show "Off" with data-value="off".
  const trigger = screen.queryByTestId("global-agent-thinking-effort-select");
  assert.ok(
    trigger,
    "effort custom select trigger must be present in DefaultConfigStep after Goose loads",
  );
  assert.equal(
    trigger.getAttribute("data-value"),
    "off",
    'DefaultConfigStep effort trigger data-value must be "off" (Goose canonical)',
  );
  assert.ok(
    trigger.textContent?.includes("Off"),
    `DefaultConfigStep effort trigger must show "Off"; got: "${trigger.textContent}"`,
  );
});

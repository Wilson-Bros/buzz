import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const LOCAL = "d".repeat(64);

test("saved deployment with offline presence is not shown as online", async ({
  page,
}, testInfo) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: LOCAL,
        name: "Offline deployment",
        status: "deployed",
        backend: { type: "provider", id: "fixture", config: {} },
        channelNames: ["agents"],
      },
    ],
  });
  await page.goto("/#/agents");
  const dot = page.getByTestId(`agent-runtime-active-${LOCAL}`);
  await expect(dot).toHaveAttribute(
    "aria-label",
    "Offline deployment: Offline",
  );
  await expect(dot.locator("xpath=../..")).not.toHaveClass(/bg-emerald-500/);
  await page
    .getByRole("button", { name: "Offline deployment agent profile" })
    .click();
  await expect(page.getByTestId("user-profile-presence-badge")).toHaveAttribute(
    "aria-label",
    "Offline",
  );
  // Preserve the existing request-only lifecycle control; no inferred redeploy.
  await expect(
    page.getByTestId("user-profile-agent-primary-action"),
  ).toHaveAttribute("aria-label", "Shutdown");
  await waitForAnimations(page);
  await page
    .getByTestId("user-profile-panel")
    .screenshot({ path: testInfo.outputPath("offline-deployment.png") });

  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
          channelName: "agents",
          kind: 20001,
        }),
      ),
    )
    .toBe(true);
  for (const status of ["online", "away", "offline"] as const) {
    await page.evaluate(
      ({ pubkey, status }) => {
        const emit = window.__BUZZ_E2E_EMIT_MOCK_PRESENCE__;
        if (!emit) throw new Error("Mock presence emitter is unavailable.");
        emit({ pubkey, status });
      },
      { pubkey: LOCAL, status },
    );
    await expect(
      page.getByTestId("user-profile-presence-badge"),
    ).toHaveAttribute("aria-label", status[0].toUpperCase() + status.slice(1));
    await expect(dot).toHaveAttribute(
      "aria-label",
      `Offline deployment: ${status[0].toUpperCase() + status.slice(1)}`,
    );
    await expect(
      page.getByTestId("user-profile-agent-primary-action"),
    ).toHaveAttribute("aria-label", "Shutdown");
    if (status === "online") {
      await waitForAnimations(page);
      await page
        .getByTestId("user-profile-panel")
        .screenshot({ path: testInfo.outputPath("online-deployment.png") });
      await page.getByTestId("user-profile-agent-primary-action").click();
      await expect(
        page.locator("[data-sonner-toast]").filter({
          hasText:
            "Shutdown requested. This does not confirm the agent has stopped.",
        }),
      ).toBeVisible();
      await expect(
        page.getByTestId("user-profile-presence-badge"),
      ).toHaveAttribute("aria-label", "Online");
      await expect(
        page.getByTestId("user-profile-agent-primary-action"),
      ).toHaveAttribute("aria-label", "Shutdown");
      expect(
        await page.evaluate(() =>
          (window.__BUZZ_E2E_COMMANDS__ ?? []).filter((command) =>
            ["start_managed_agent", "stop_managed_agent"].includes(command),
          ),
        ),
      ).toEqual([]);
    }
  }
  await page.evaluate(() =>
    window.__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?.("disconnected"),
  );
  await expect(page.getByTestId("user-profile-presence-badge")).toHaveCount(0);
  await expect(dot).toHaveAttribute(
    "aria-label",
    "Offline deployment: Availability unknown",
  );
});

test("missing snapshot is offline but failed reads cannot reuse cached online", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: LOCAL,
        name: "Snapshot agent",
        status: "running",
        channelNames: ["agents"],
      },
    ],
  });
  await page.goto("/#/agents");
  const dot = page.getByTestId(`agent-runtime-active-${LOCAL}`);
  await expect(dot).toHaveAttribute("aria-label", "Snapshot agent: Offline");
  await page
    .getByRole("button", { name: "Snapshot agent agent profile" })
    .click();
  const badge = page.getByTestId("user-profile-presence-badge");
  await expect(badge).toHaveAttribute("aria-label", "Offline");

  // Override the IPC response, not rendered state or query data. Both card and
  // profile must consume the same real query success/error boundary.
  await page.evaluate(() => {
    const w = window as typeof window & {
      __AVAILABILITY_RESPONSE__?: "missing" | "online" | "error";
      __TAURI_INTERNALS__: {
        invoke: (
          command: string,
          payload: unknown,
          options: unknown,
        ) => Promise<unknown>;
      };
    };
    const original = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
    w.__TAURI_INTERNALS__.invoke = async (command, payload, options) => {
      if (command === "get_presence") {
        if (w.__AVAILABILITY_RESPONSE__ === "error")
          throw "relay unreachable: request timed out"; // Native Result::Err IPC shape
        if (w.__AVAILABILITY_RESPONSE__ === "missing") return {};
        if (w.__AVAILABILITY_RESPONSE__ === "online") {
          return Object.fromEntries(
            (payload as { pubkeys: string[] }).pubkeys.map((key) => [
              key,
              "online",
            ]),
          );
        }
      }
      return original(command, payload, options);
    };
  });
  for (const response of ["online", "missing", "online", "error"] as const) {
    await page.evaluate(async (response) => {
      const w = window as typeof window & {
        __AVAILABILITY_RESPONSE__?: typeof response;
        __BUZZ_E2E_QUERY_CLIENT__?: {
          invalidateQueries: (filter: { queryKey: string[] }) => Promise<void>;
        };
      };
      w.__AVAILABILITY_RESPONSE__ = response;
      if (!w.__BUZZ_E2E_QUERY_CLIENT__)
        throw new Error("Query client unavailable");
      await w.__BUZZ_E2E_QUERY_CLIENT__.invalidateQueries({
        queryKey: ["presence"],
      });
    }, response);
    const label =
      response === "error"
        ? "Availability unknown"
        : response === "missing"
          ? "Offline"
          : "Online";
    await expect(dot).toHaveAttribute("aria-label", `Snapshot agent: ${label}`);
    if (response === "error") await expect(badge).toHaveCount(0);
    else await expect(badge).toHaveAttribute("aria-label", label);
    await expect(
      page.getByTestId("user-profile-agent-primary-action"),
    ).toHaveAttribute("aria-label", "Stop");
  }
});

test("stopped local with authored presence has a dot, not an invokable Start", async ({
  page,
}, testInfo) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: LOCAL,
        name: "Present local",
        status: "stopped",
        channelNames: ["agents"],
      },
    ],
  });
  await page.goto("/#/agents");
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
          channelName: "agents",
          kind: 20001,
        }),
      ),
    )
    .toBe(true);
  await page.evaluate(
    (pubkey) =>
      window.__BUZZ_E2E_EMIT_MOCK_PRESENCE__?.({ pubkey, status: "online" }),
    LOCAL,
  );
  await expect(
    page.getByTestId(`agent-runtime-active-${LOCAL}`),
  ).toHaveAttribute("aria-label", "Present local: Online");
  await expect(page.getByTestId(`agent-runtime-start-${LOCAL}`)).toHaveCount(0);
  await page
    .getByRole("button", { name: "Present local agent profile" })
    .press("Enter");
  const action = page.getByTestId("user-profile-agent-primary-action");
  await expect(action).toHaveAttribute("aria-label", "Start agent");
  await expect(action).toBeDisabled();
  await action.evaluate((button: HTMLButtonElement) => button.click());
  await action.press("Enter");
  await action.press("Space");
  expect(
    await page.evaluate(() =>
      (window.__BUZZ_E2E_COMMANDS__ ?? []).filter((command) =>
        ["start_managed_agent", "stop_managed_agent"].includes(command),
      ),
    ),
  ).toEqual([]);
  await expect(page.getByTestId("user-profile-agent-restart")).toHaveCount(0);
  const badge = page.getByTestId("user-profile-presence-badge");
  await expect(badge).toHaveAttribute("aria-label", "Online");
  await waitForAnimations(page);
  await page
    .getByTestId(`managed-agent-${LOCAL}`)
    .screenshot({ path: testInfo.outputPath("stopped-online-card.png") });
  await page
    .getByTestId("user-profile-panel")
    .screenshot({ path: testInfo.outputPath("stopped-online-profile.png") });
  for (const status of ["away", "offline"] as const) {
    await page.evaluate(
      ({ pubkey, status }) =>
        window.__BUZZ_E2E_EMIT_MOCK_PRESENCE__?.({ pubkey, status }),
      { pubkey: LOCAL, status },
    );
    await expect(badge).toHaveAttribute(
      "aria-label",
      status === "away" ? "Away" : "Offline",
    );
    if (status === "away") {
      await expect(
        page.getByTestId(`agent-runtime-active-${LOCAL}`),
      ).toHaveAttribute("aria-label", "Present local: Away");
      await expect(action).toBeDisabled();
    } else {
      await expect(action).toBeEnabled();
      await expect(
        page.getByTestId(`agent-runtime-start-${LOCAL}`),
      ).toBeVisible();
    }
  }
  // Keyboard activation is restored for the ordinary stopped/offline control.
  // Runtime startup alone must still not manufacture authored availability.
  await action.press("Enter");
  await expect(action).toHaveAttribute("aria-label", "Stop");
  await expect(badge).toHaveAttribute("aria-label", "Offline");
  await expect(
    page.getByTestId(`agent-runtime-active-${LOCAL}`),
  ).toHaveAttribute("aria-label", "Present local: Offline");
});

test("member menu cannot start a present stopped local runtime", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: LOCAL,
        name: "Present member",
        status: "stopped",
        channelNames: ["agents"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-agents").click();
  await page.getByTestId("channel-members-trigger").click();
  const row = page.getByTestId(`sidebar-member-${LOCAL}`);
  await expect(row).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
          channelName: "agents",
          kind: 20001,
        }),
      ),
    )
    .toBe(true);
  await page.evaluate(
    (pubkey) =>
      window.__BUZZ_E2E_EMIT_MOCK_PRESENCE__?.({ pubkey, status: "away" }),
    LOCAL,
  );
  await row.hover();
  const menu = page.getByTestId(`sidebar-member-menu-${LOCAL}`);
  await menu.focus();
  await menu.press("Enter");
  const action = page.getByTestId(`sidebar-agent-action-${LOCAL}`);
  await expect(action).toContainText("Start");
  await expect(action).toHaveAttribute("aria-disabled", "true");
  await action.press("Enter");
  await action.evaluate((node: HTMLElement) => node.click());
  expect(
    await page.evaluate(() =>
      (window.__BUZZ_E2E_COMMANDS__ ?? []).filter((command) =>
        /^(start|stop)_managed_agent/.test(command),
      ),
    ),
  ).toEqual([]);
  await page.evaluate(
    (pubkey) =>
      window.__BUZZ_E2E_EMIT_MOCK_PRESENCE__?.({ pubkey, status: "offline" }),
    LOCAL,
  );
  await expect(action).not.toHaveAttribute("aria-disabled", "true");
  await action.click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMANDS__ ?? []).filter(
            (command) => command === "start_managed_agent_runtime",
          ).length,
      ),
    )
    .toBe(1);
});

for (const surface of ["agents", "members"] as const) {
  test(`${surface}: three rows and their actions share one presence request`, async ({
    page,
  }) => {
    const keys = [LOCAL, "e".repeat(64), "f".repeat(64)];
    await installMockBridge(page, {
      managedAgents: keys.map((pubkey, index) => ({
        pubkey,
        name: `Shared snapshot ${index}`,
        status: "running" as const,
        channelNames: ["agents"],
      })),
    });
    await page.goto(surface === "agents" ? "/#/agents" : "/");
    if (surface === "members") {
      await page.getByTestId("channel-agents").click();
      await page.getByTestId("channel-members-trigger").click();
    }
    for (const pubkey of keys) {
      await expect(
        page.getByTestId(
          surface === "agents"
            ? `managed-agent-${pubkey}`
            : `sidebar-member-${pubkey}`,
        ),
      ).toBeVisible();
    }
    // Count requests at the actual IPC boundary. Refresh every active presence
    // observer so a hidden per-row/action query cannot escape the assertion.
    const calls = await page.evaluate(async (keys) => {
      const w = window as typeof window & {
        __TAURI_INTERNALS__: {
          invoke: (
            command: string,
            payload: unknown,
            options: unknown,
          ) => Promise<unknown>;
        };
        __BUZZ_E2E_QUERY_CLIENT__: {
          invalidateQueries: (filter: { queryKey: string[] }) => Promise<void>;
        };
      };
      const calls: string[][] = [];
      const original = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = (command, payload, options) => {
        if (command === "get_presence") {
          const pubkeys = (payload as { pubkeys: string[] }).pubkeys;
          if (pubkeys.some((key) => keys.includes(key))) calls.push(pubkeys);
        }
        return original(command, payload, options);
      };
      try {
        await w.__BUZZ_E2E_QUERY_CLIENT__.invalidateQueries({
          queryKey: ["presence"],
        });
      } finally {
        w.__TAURI_INTERNALS__.invoke = original;
      }
      return calls;
    }, keys);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual(expect.arrayContaining(keys));
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "agents",
            kind: 20001,
          }),
        ),
      )
      .toBe(true);
    await page.evaluate(
      (pubkey) =>
        window.__BUZZ_E2E_EMIT_MOCK_PRESENCE__?.({ pubkey, status: "away" }),
      LOCAL,
    );
    if (surface === "agents") {
      await expect(
        page.getByTestId(`agent-runtime-active-${LOCAL}`),
      ).toHaveAttribute("aria-label", "Shared snapshot 0: Away");
      await expect(
        page.getByTestId(`agent-runtime-active-${keys[1]}`),
      ).toHaveAttribute("aria-label", "Shared snapshot 1: Offline");
    } else {
      await expect(
        page.getByTestId(`sidebar-member-presence-${LOCAL}`).locator("span"),
      ).toHaveClass(/bg-amber-500/);
      await expect(
        page.getByTestId(`sidebar-member-presence-${keys[1]}`).locator("span"),
      ).toHaveClass(/bg-muted-foreground/);
    }
  });
}

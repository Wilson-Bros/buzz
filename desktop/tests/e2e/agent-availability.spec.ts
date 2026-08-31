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
          throw new Error("Presence fixture: relay read failed");
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

import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const BESTIE_PUBKEY = "be".repeat(32);
const RELAY_URL = "ws://localhost:3000";

async function enableBestie(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ bestie: true }),
    );
  });
}

test("assigns from an agent profile, reopens, drags, and offers the message action", async ({
  page,
}) => {
  await enableBestie(page);
  await installMockBridge(
    page,
    {
      managedAgents: [
        {
          pubkey: BESTIE_PUBKEY,
          name: "Mochi",
          avatarUrl:
            "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==",
          status: "stopped",
        },
      ],
      managedAgentRuntimes: [
        {
          pubkey: BESTIE_PUBKEY,
          relayUrl: RELAY_URL,
          lifecycle: "stopped",
        },
      ],
    },
    { seedPreviewFeatures: false },
  );
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const floatingAvatar = page.getByTestId("bestie-floating-avatar");
  await expect(floatingAvatar).toBeVisible();
  expect(
    await floatingAvatar.evaluate(
      (element) => element.parentElement === document.body,
    ),
  ).toBe(true);
  await expect(floatingAvatar.getByTestId("bestie-empty-mark")).toBeVisible();
  await floatingAvatar.click();
  await expect(
    page.getByText("Choose a Bestie", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText(/Open one of your local agents/)).toBeVisible();
  await page.keyboard.press("Escape");

  await page.getByTestId("open-agents-view").click();
  await page.getByRole("button", { name: "Mochi agent profile" }).click();
  const bestieAction = page.getByTestId("user-profile-bestie-action");
  await expect(bestieAction).toContainText("Make Bestie");
  await bestieAction.click();
  const confirmation = page.getByTestId("bestie-confirm-dialog");
  await expect(confirmation).toContainText("Make Mochi your Bestie?");
  await confirmation.getByRole("button", { name: "Make Bestie" }).click();
  await expect(bestieAction).toContainText("Remove Bestie");
  await expect(
    floatingAvatar.getByTestId("bestie-trigger-avatar"),
  ).toBeVisible();
  await expect(
    floatingAvatar.getByTestId("bestie-trigger-avatar-image"),
  ).toHaveAttribute("draggable", "false");
  const sidebarBestie = page.getByTestId("bestie-sidebar-entry");
  await expect(sidebarBestie).toContainText("Mochi");
  expect(
    await sidebarBestie.evaluate(
      (element) =>
        element.previousElementSibling?.querySelector(
          '[data-testid="open-agents-view"]',
        ) !== null,
    ),
  ).toBe(true);
  await waitForAnimations(page);
  await page.getByTestId("sidebar-primary-menu").screenshot({
    path: "test-results/bestie/00-sidebar-entry.png",
  });
  await page.getByTestId("user-profile-agent-management-section").screenshot({
    path: "test-results/bestie/00-profile-actions.png",
  });

  await page.keyboard.press("Escape");
  const bestieCard = page.getByTestId(`managed-agent-${BESTIE_PUBKEY}`);
  await expect(
    bestieCard.getByTestId(`bestie-card-badge-${BESTIE_PUBKEY}`),
  ).toBeVisible();
  await waitForAnimations(page);
  await bestieCard.screenshot({
    path: "test-results/bestie/00-bestie-card.png",
  });

  await floatingAvatar.click();
  await expect(page.getByTestId("bestie-activity-dot").last()).toBeVisible();
  await expect(page.getByText("Waking", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Choose a different agent")).toHaveCount(0);
  await waitForAnimations(page);
  await page.getByTestId("bestie-bloom-content").screenshot({
    path: "test-results/bestie/01-floating-popover.png",
  });

  const bloomContent = page.getByTestId("bestie-bloom-content");
  const bloomAvatar = bloomContent.getByTestId("bestie-agent-avatar");
  const openPanelBeforeDrag = await bloomContent.boundingBox();
  const openAvatarBeforeDrag = await bloomAvatar.boundingBox();
  for (const edge of ["top", "right", "bottom", "left"]) {
    await expect(
      floatingAvatar.locator(`[data-bestie-drag-edge="${edge}"]`),
    ).toBeVisible();
  }
  const dragHandle = floatingAvatar.locator('[data-bestie-drag-edge="bottom"]');
  const dragHandleBox = await dragHandle.boundingBox();
  expect(dragHandleBox).not.toBeNull();
  await page.mouse.move(
    (dragHandleBox?.x ?? 0) + (dragHandleBox?.width ?? 0) / 2,
    (dragHandleBox?.y ?? 0) + (dragHandleBox?.height ?? 0) / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    (dragHandleBox?.x ?? 0) + (dragHandleBox?.width ?? 0) / 2 - 80,
    (dragHandleBox?.y ?? 0) + (dragHandleBox?.height ?? 0) / 2 + 80,
    { steps: 8 },
  );
  await page.mouse.up();
  const openPanelAfterDrag = await bloomContent.boundingBox();
  const openAvatarAfterDrag = await bloomAvatar.boundingBox();
  const panelDelta = {
    x: (openPanelAfterDrag?.x ?? 0) - (openPanelBeforeDrag?.x ?? 0),
    y: (openPanelAfterDrag?.y ?? 0) - (openPanelBeforeDrag?.y ?? 0),
  };
  const avatarDelta = {
    x: (openAvatarAfterDrag?.x ?? 0) - (openAvatarBeforeDrag?.x ?? 0),
    y: (openAvatarAfterDrag?.y ?? 0) - (openAvatarBeforeDrag?.y ?? 0),
  };
  expect(Math.abs(panelDelta.x - avatarDelta.x)).toBeLessThan(1);
  expect(Math.abs(panelDelta.y - avatarDelta.y)).toBeLessThan(1);

  await bloomContent.getByRole("button", { name: "Close Bestie" }).click();
  await expect(bloomContent).toHaveCount(0);
  await sidebarBestie.click();
  await expect(page).toHaveURL(/\/channels\//);
  await expect(page.getByRole("heading", { name: "Mochi" })).toBeVisible();
  const firstConversationUrl = page.url();
  await expect(page).toHaveURL(firstConversationUrl);
  await expect(page.getByTestId("bestie-bloom-content")).toHaveCount(0);

  await floatingAvatar.click();
  await expect(page.getByTestId("bestie-activity-dot").last()).toBeVisible();
  await page.getByRole("button", { name: "Close Bestie" }).click();
  const beforeDrag = await floatingAvatar.boundingBox();
  const collapsedAvatar = floatingAvatar.getByTestId("bestie-trigger-avatar");
  const collapsedAvatarBeforeDrag = await collapsedAvatar.boundingBox();
  expect(beforeDrag).not.toBeNull();
  await page.mouse.move(
    (beforeDrag?.x ?? 0) + (beforeDrag?.width ?? 0) / 2,
    (beforeDrag?.y ?? 0) + (beforeDrag?.height ?? 0) / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    (beforeDrag?.x ?? 0) - 120,
    (beforeDrag?.y ?? 0) + 120,
    { steps: 8 },
  );
  await page.mouse.up();
  const afterDrag = await floatingAvatar.boundingBox();
  const collapsedAvatarAfterDrag = await collapsedAvatar.boundingBox();
  expect(afterDrag?.x).toBeLessThan(beforeDrag?.x ?? 0);
  expect(afterDrag?.y).toBeGreaterThan(beforeDrag?.y ?? 0);
  expect(
    Math.abs(
      (afterDrag?.x ?? 0) -
        (beforeDrag?.x ?? 0) -
        ((collapsedAvatarAfterDrag?.x ?? 0) -
          (collapsedAvatarBeforeDrag?.x ?? 0)),
    ),
  ).toBeLessThan(1);
  expect(
    Math.abs(
      (afterDrag?.y ?? 0) -
        (beforeDrag?.y ?? 0) -
        ((collapsedAvatarAfterDrag?.y ?? 0) -
          (collapsedAvatarBeforeDrag?.y ?? 0)),
    ),
  ).toBeLessThan(1);

  await page.getByTestId("channel-general").click();
  const messageRow = page.getByTestId("message-row").first();
  await messageRow.hover();
  const messageAction = messageRow.locator('[data-testid^="bestie-message-"]');
  await expect(messageAction).toBeVisible();
  await messageAction.click();
  const messagePopover = page
    .locator("[data-radix-popper-content-wrapper]")
    .last();
  await expect(messagePopover.getByLabel("Message Mochi")).toHaveValue("");
  await expect(
    messagePopover.getByText("How can I help?", { exact: true }),
  ).toBeVisible();
  const snapshot = messagePopover.getByTestId("bestie-message-snapshot");
  await expect(snapshot).toBeVisible();
  const snapshotBox = await snapshot.boundingBox();
  const popoverBox = await messagePopover.boundingBox();
  expect(snapshotBox?.height).toBeLessThanOrEqual(96);
  expect(snapshotBox?.width).toBeLessThanOrEqual(
    (popoverBox?.width ?? 0) * 0.75 + 1,
  );
  await waitForAnimations(page);
  await messagePopover.screenshot({
    path: "test-results/bestie/02-message-popover.png",
  });
  await messagePopover.getByRole("button", { name: "Close Bestie" }).click();
  await expect(page.getByLabel("Message Mochi")).toHaveCount(0);
});

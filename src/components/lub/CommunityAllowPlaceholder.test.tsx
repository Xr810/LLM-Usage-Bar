import { readFileSync } from "node:fs";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { beforeAll, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import { toast } from "sonner";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { formatUsd } from "@/components/tray-usage/trayUsagePresentation";
import { AboutSection } from "@/components/settings/AboutSection";
import { server } from "../../../tests/msw/server";
import { tauriInvoke } from "../../../tests/msw/tauriMocks";
import { CommunityAllowPlaceholder } from "./CommunityAllowPlaceholder";
import {
  communityChromeVisible,
  detectLubBuild,
  readUpsellEntry,
  resolveLubBuild,
} from "./detectLubBuild";
import { presentLubCommandError } from "./lubErrors";
import { displayUsd } from "./money";
import { designPanelFixture } from "./panelFixture";
import type { LubBuild, LubTier } from "./panelModel";
import type { TrayUsageSnapshot } from "@/types/trayUsage";
import { TrayPanelV5 } from "./TrayPanelV5";

function allowanceLabel(locale: string) {
  return displayUsd(formatUsd(String(5), locale));
}

vi.mock("sonner", () => ({
  toast: { error: vi.fn() },
}));

beforeAll(async () => {
  i18n.addResourceBundle("en", "translation", { lub: en.lub }, true, true);
  i18n.addResourceBundle("zh", "translation", { lub: zh.lub }, true, true);
  await i18n.changeLanguage("zh");
});

function overFixture() {
  return designPanelFixture({
    state: "over",
    todayUsd: "12.00",
    dailyCapUsd: "10",
  });
}

function renderPanel(options: {
  state?: "over" | "ok";
  build?: LubBuild;
  tier?: LubTier;
  upsellEntry?: boolean;
  locale?: string;
}) {
  const model = options.state === "ok" ? designPanelFixture() : overFixture();
  return render(
    <TrayPanelV5
      model={model}
      locale={options.locale ?? "zh-CN"}
      build={options.build ?? "official"}
      tier={options.tier}
      upsellEntry={options.upsellEntry}
      onOpenTrends={() => undefined}
      onOpenSettings={() => undefined}
    />,
  );
}

function allowButtons() {
  return screen.queryAllByRole("button", {
    name: /放行这 1 笔|今日再放行/,
  });
}

describe("community allow placeholder", () => {
  it("shows two focusable disabled buttons and sends no command", async () => {
    const user = userEvent.setup();
    const callsBefore = tauriInvoke.mock.calls.length;
    renderPanel({ build: "community" });

    const amount = allowanceLabel("zh-CN");
    const unavailable = zh.lub.errors.feature_unavailable;
    const one = screen.getByRole("button", {
      name: i18n.t("lub.community.aria_allow_disabled", {
        action: zh.lub.community.allow_this_one,
      }),
    });
    const more = screen.getByRole("button", {
      name: i18n.t("lub.community.aria_allow_disabled", {
        action: i18n.t("lub.community.allow_more_today", { amount }),
      }),
    });

    for (const button of [one, more]) {
      expect(button).toHaveAttribute("aria-disabled", "true");
      expect(button).not.toHaveAttribute("disabled");
      expect(button).toHaveAttribute("title", unavailable);
      expect(button.className).toContain("opacity-45");
      expect(button.className).toContain("cursor-not-allowed");
      button.focus();
      expect(button).toHaveFocus();
    }
    expect(more).toHaveTextContent(amount);
    expect(
      screen.getByText(zh.lub.community.placeholder_note),
    ).toBeInTheDocument();
    expect(screen.getByText(zh.lub.community.badge)).toBeInTheDocument();
    expect(screen.getByLabelText(zh.lub.community.aria_badge)).toBe(
      screen.getByText(zh.lub.community.badge),
    );

    const slot = document.querySelector("[data-slot='community-allow']");
    expect(slot?.querySelector("a, [href]")).toBeNull();
    expect(screen.queryByText(zh.lub.overcap.compare_link)).toBeNull();

    await user.click(one);
    await user.click(more);
    one.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
    expect(tauriInvoke.mock.calls.length).toBe(callsBefore);
  });

  it("keeps the badge without allow buttons when spend is under the cap", () => {
    renderPanel({ build: "community", state: "ok" });
    expect(allowButtons()).toHaveLength(0);
    expect(document.querySelector("[data-slot='community-allow']")).toBeNull();
    expect(screen.getByText(zh.lub.community.badge)).toBeInTheDocument();
    expect(screen.queryByText(zh.lub.overcap.compare_link)).toBeNull();
  });

  it("keeps official Free and Pro free of community chrome when upsell is on", () => {
    const { unmount } = renderPanel({
      build: "official",
      tier: "free",
      upsellEntry: true,
    });
    expect(allowButtons()).toHaveLength(0);
    expect(screen.queryByText(zh.lub.community.badge)).toBeNull();
    expect(document.querySelector("[data-slot='community-allow']")).toBeNull();
    expect(
      screen.getByRole("button", { name: zh.lub.overcap.compare_link }),
    ).toBeInTheDocument();
    expect(document.querySelector("a, [href]")).toBeNull();
    unmount();

    renderPanel({ build: "official", tier: "pro", upsellEntry: true });
    expect(allowButtons()).toHaveLength(0);
    expect(screen.queryByText(zh.lub.community.badge)).toBeNull();
    expect(
      screen.getByRole("button", { name: zh.lub.overcap.compare_link }),
    ).toBeInTheDocument();
    expect(document.querySelector("a, [href]")).toBeNull();
  });

  it("ignores upsell_entry for a community build", () => {
    const off = renderPanel({ build: "community", upsellEntry: false });
    const offSlot = document.querySelector(
      "[data-slot='community-allow']",
    )?.innerHTML;
    const offBadge = screen.getByText(zh.lub.community.badge).textContent;
    off.unmount();

    renderPanel({ build: "community", upsellEntry: true, tier: "pro" });
    expect(
      document.querySelector("[data-slot='community-allow']")?.innerHTML,
    ).toBe(offSlot);
    expect(screen.getByText(zh.lub.community.badge).textContent).toBe(offBadge);
    expect(screen.queryByText(zh.lub.overcap.compare_link)).toBeNull();
    expect(
      communityChromeVisible({ build: "community", upsellEntry: true }),
    ).toBe(communityChromeVisible({ build: "community", upsellEntry: false }));
    expect(
      communityChromeVisible({
        build: "official",
        upsellEntry: true,
        tier: "free",
      }),
    ).toBe(false);
    expect(communityChromeVisible({ build: "official", tier: "pro" })).toBe(
      false,
    );
  });

  it("uses the English feature-unavailable tooltip", async () => {
    await i18n.changeLanguage("en");
    renderPanel({ build: "community", locale: "en" });
    const amount = allowanceLabel("en");
    const button = screen.getByRole("button", {
      name: i18n.t("lub.community.aria_allow_disabled", {
        action: i18n.t("lub.community.allow_more_today", { amount }),
      }),
    });
    expect(button).toHaveAttribute("title", en.lub.errors.feature_unavailable);
    expect(screen.getByText(en.lub.community.badge)).toBeInTheDocument();
    expect(
      screen.getByText(en.lub.community.placeholder_note),
    ).toBeInTheDocument();
    await i18n.changeLanguage("zh");
  });
});

describe("community build detection", () => {
  it("stays community while license status is loading, missing, or failed", async () => {
    expect(resolveLubBuild({ pending: true })).toBe("community");
    expect(resolveLubBuild({ license: null })).toBe("community");
    expect(
      resolveLubBuild({
        license: {
          tier: "free",
          build: "community",
          features: [],
          uiFlags: { upsellEntry: false },
        },
      }),
    ).toBe("community");
    expect(readUpsellEntry(undefined)).toBe(false);
    expect(readUpsellEntry({ upsellEntry: false })).toBe(false);
    expect(readUpsellEntry({ upsellEntry: true })).toBe(true);

    server.use(
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.json({ message: "missing" }, { status: 500 }),
      ),
    );
    await expect(detectLubBuild()).resolves.toEqual({
      build: "community",
      upsellEntry: false,
    });
    await expect(
      detectLubBuild({ snapshotBuildType: "official" }),
    ).resolves.toEqual({
      build: "official",
      upsellEntry: false,
    });

    type SnapshotHasBuildType = "buildType" extends keyof TrayUsageSnapshot
      ? true
      : false;
    const snapshotHasBuildType: SnapshotHasBuildType = true;
    expect(snapshotHasBuildType).toBe(true);

    const placeholder = readFileSync(
      "src/components/lub/CommunityAllowPlaceholder.tsx",
      "utf8",
    );
    expect(placeholder).not.toMatch(/\binvoke\s*\(/);
    expect(placeholder).not.toMatch(/<a[\s>/]/);
    const badge = readFileSync(
      "src/components/lub/CommunityBuildBadge.tsx",
      "utf8",
    );
    expect(badge).not.toMatch(/<a[\s>/]|href/);
  });

  it("reads get_license_status and prefers snapshot buildType", async () => {
    server.use(
      http.post("http://tauri.local/get_license_status", () =>
        HttpResponse.json({
          tier: "free",
          build: "official",
          features: [],
          uiFlags: { upsellEntry: true },
        }),
      ),
    );
    await expect(detectLubBuild()).resolves.toEqual({
      build: "official",
      upsellEntry: true,
    });
    await expect(
      detectLubBuild({ snapshotBuildType: "community" }),
    ).resolves.toEqual({
      build: "community",
      upsellEntry: true,
    });
    expect(
      resolveLubBuild({
        snapshotBuildType: "community",
        pending: true,
        license: {
          tier: "pro",
          build: "official",
          features: [],
          uiFlags: { upsellEntry: false },
        },
      }),
    ).toBe("community");
  });

  it("keeps the community badge until license status says official", async () => {
    let release: () => void = () => undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    server.use(
      http.post("http://tauri.local/get_license_status", async () => {
        await gate;
        return HttpResponse.json({
          tier: "free",
          build: "official",
          features: [],
        });
      }),
    );
    render(<AboutSection />);
    expect(screen.getByText(zh.lub.community.badge)).toBeInTheDocument();
    expect(screen.queryByRole("link")).toBeNull();
    release();
    await waitFor(() => {
      expect(screen.queryByText(zh.lub.community.badge)).toBeNull();
    });
  });

  it("hides the compare link on a community build even when the switch is on", () => {
    renderPanel({ build: "community", upsellEntry: true });
    expect(screen.queryByText(zh.lub.overcap.compare_link)).toBeNull();
    const { unmount } = render(
      <TrayPanelV5
        model={overFixture()}
        locale="zh-CN"
        build="community"
        upsellEntry
        onOpenTrends={() => undefined}
        onOpenSettings={() => undefined}
      />,
    );
    expect(screen.queryByText(zh.lub.overcap.compare_link)).toBeNull();
    unmount();
  });
});

describe("FEATURE_UNAVAILABLE", () => {
  it("toasts the community copy once and does not ask for a retry", () => {
    expect(presentLubCommandError("FEATURE_UNAVAILABLE")).toBe(true);
    expect(toast.error).toHaveBeenCalledTimes(1);
    expect(toast.error).toHaveBeenCalledWith(zh.lub.errors.feature_unavailable);
    expect(presentLubCommandError("SOMETHING_ELSE")).toBe(false);
    expect(toast.error).toHaveBeenCalledTimes(1);
    expect(tauriInvoke).not.toHaveBeenCalled();
  });
});

describe("About community badge", () => {
  it("shows the badge only for a community build", () => {
    const { unmount } = render(<AboutSection build="community" />);
    expect(screen.getByText(zh.lub.community.badge)).toBeInTheDocument();
    expect(
      screen.getByLabelText(zh.lub.community.aria_badge),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: zh.lub.settings.about }),
    ).toBeInTheDocument();
    expect(document.querySelector("a, [href]")).toBeNull();
    unmount();

    render(<AboutSection build="official" />);
    expect(screen.queryByText(zh.lub.community.badge)).toBeNull();
    expect(
      screen.queryByRole("heading", { name: zh.lub.settings.about }),
    ).toBeNull();
  });

  it("renders the placeholder note without a link of its own", () => {
    render(<CommunityAllowPlaceholder locale="zh-CN" />);
    expect(
      screen.getByText(zh.lub.community.placeholder_note),
    ).toBeInTheDocument();
    expect(document.querySelector("a, [href]")).toBeNull();
  });
});

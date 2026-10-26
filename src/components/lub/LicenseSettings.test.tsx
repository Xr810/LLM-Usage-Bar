import { readFileSync } from "node:fs";
import path from "node:path";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zh from "@/i18n/locales/zh.json";
import zhTW from "@/i18n/locales/zh-TW.json";
import type { LicenseStatus } from "@/types/license";
import { LicenseSettings } from "./LicenseSettings";
import { activateLicense, communityLicenseStatus } from "./licenseMock";
/** Matches the shell: batch 1 does not render the compare link. */
const proCompareEnabled = false;

function officialFree(overrides: Partial<LicenseStatus> = {}): LicenseStatus {
  return {
    tier: "free",
    state: "free",
    build: "official",
    features: [],
    uiFlags: { upsellEntry: true },
    ...overrides,
  };
}

beforeAll(async () => {
  i18n.addResourceBundle("en", "translation", { lub: en.lub }, true, true);
  i18n.addResourceBundle("zh", "translation", { lub: zh.lub }, true, true);
  i18n.addResourceBundle("zh-TW", "translation", { lub: zhTW.lub }, true, true);
  i18n.addResourceBundle("ja", "translation", { lub: ja.lub }, true, true);
  await i18n.changeLanguage("zh");
});

describe("community license shell", () => {
  it("returns COMMUNITY_BUILD and does not change the community status", () => {
    const before = communityLicenseStatus();
    expect(activateLicense("ULB1.sample")).toEqual({
      ok: false,
      error: { code: "COMMUNITY_BUILD", message: "Official build required" },
    });
    expect(communityLicenseStatus()).toEqual(before);
    expect(before.tier).toBe("free");
    expect(before.build).toBe("community");
    expect(before.uiFlags.upsellEntry).toBe(false);
    expect(before.expiresAt).toBeUndefined();
    expect(before.graceUntil).toBeUndefined();
    expect(before.emailMasked).toBeUndefined();
  });

  it("keeps the community-build label after activate and hides the compare link in batch 1", async () => {
    const user = userEvent.setup();
    render(<LicenseSettings />);
    expect(screen.getByTestId("license-tier")).toHaveTextContent("社区构建");
    expect(screen.getByTestId("license-expires")).toHaveTextContent("—");
    expect(screen.getByTestId("license-email")).toHaveTextContent("—");

    await user.click(screen.getByRole("button", { name: "激活" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("COMMUNITY_BUILD");
    expect(alert).toHaveTextContent("需官方构建");
    expect(screen.getByTestId("license-tier")).toHaveTextContent("社区构建");
    expect(screen.getByTestId("license-expires")).toHaveTextContent("—");

    const compare = screen.queryByRole("link", { name: "比较 Free 与 Pro" });
    if (proCompareEnabled) {
      expect(compare).toHaveAttribute("href", "#free-vs-pro");
    } else {
      expect(compare).toBeNull();
      expect(screen.queryByText("比较 Free 与 Pro")).toBeNull();
    }

    const source = readFileSync(
      path.resolve(process.cwd(), "src/components/lub/LicenseSettings.tsx"),
      "utf8",
    );
    expect(source).toContain('t("lub.license.community_build")');
    expect(source).toContain('t("lub.license.official_build_required")');
    expect(source).not.toContain('lub.license.community_build",');
    expect(source).not.toContain('lub.license.official_build_required",');
  });
});

describe("expired Pro versus normal Free", () => {
  const expiredCopy = {
    zh: "Pro 已过期",
    en: "Pro expired",
    "zh-TW": "Pro 已過期",
    ja: "Pro は期限切れです",
  } as const;

  it("labels free with a stored expiry as expired and hides the renewal control", async () => {
    const status = officialFree({ expiresAt: "2026-01-01T00:00:00Z" });
    const { unmount } = render(
      <LicenseSettings variant="embedded" status={status} />,
    );
    expect(screen.getByTestId("license-tier")).toHaveTextContent(
      expiredCopy.zh,
    );
    expect(screen.getByTestId("license-expires")).toHaveTextContent(
      "2026-01-01T00:00:00Z",
    );
    const slot = document.querySelector("[data-slot='purchase']");
    expect(slot).not.toBeNull();
    expect(slot).toHaveAttribute("hidden");
    expect(slot).toHaveAttribute("data-purchase-ui", "off");
    expect(slot).toHaveTextContent("");
    expect(screen.queryByText("续订 Pro")).toBeNull();
    expect(screen.queryByText("购买 Pro")).toBeNull();
    expect(screen.queryByText(/试用/)).toBeNull();
    expect(document.querySelector("a, [href]")).toBeNull();
    unmount();

    for (const language of ["en", "zh-TW", "ja"] as const) {
      await i18n.changeLanguage(language);
      const view = render(
        <LicenseSettings variant="embedded" status={status} />,
      );
      expect(screen.getByTestId("license-tier")).toHaveTextContent(
        expiredCopy[language],
      );
      expect(screen.queryByRole("link")).toBeNull();
      view.unmount();
    }
    await i18n.changeLanguage("zh");
  });

  it("keeps the expired label and omits the renewal slot when upsellEntry is off", () => {
    render(
      <LicenseSettings
        variant="embedded"
        status={officialFree({
          expiresAt: "2026-01-01T00:00:00Z",
          uiFlags: { upsellEntry: false },
        })}
      />,
    );
    expect(screen.getByTestId("license-tier")).toHaveTextContent("Pro 已过期");
    expect(document.querySelector("[data-slot='purchase']")).toBeNull();
    expect(screen.queryByText("续订 Pro")).toBeNull();
  });

  it("shows a normal Free tier when expiresAt is omitted or blank", () => {
    const { unmount } = render(
      <LicenseSettings variant="embedded" status={officialFree()} />,
    );
    expect(screen.getByTestId("license-tier")).toHaveTextContent(/^free$/);
    expect(screen.queryByText("Pro 已过期")).toBeNull();
    expect(document.querySelector("[data-slot='purchase']")).toBeNull();
    unmount();

    render(
      <LicenseSettings
        variant="embedded"
        status={officialFree({ expiresAt: "   " })}
      />,
    );
    expect(screen.getByTestId("license-tier")).toHaveTextContent(/^free$/);
    expect(screen.queryByText("Pro 已过期")).toBeNull();
    expect(screen.queryByText("续订 Pro")).toBeNull();
  });

  it("leaves an active Pro on its tier label when expiresAt is set", () => {
    render(
      <LicenseSettings
        variant="embedded"
        status={officialFree({
          tier: "pro",
          state: "active",
          expiresAt: "2026-12-01T00:00:00Z",
        })}
      />,
    );
    expect(screen.getByTestId("license-tier")).toHaveTextContent(/^pro$/);
    expect(screen.queryByText("Pro 已过期")).toBeNull();
  });
});

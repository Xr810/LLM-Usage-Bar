import { readFileSync } from "node:fs";
import path from "node:path";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { LicenseSettings } from "./LicenseSettings";
import { activateLicense, communityLicenseStatus } from "./licenseMock";
/** Matches the shell: batch 1 does not render the compare link. */
const proCompareEnabled = false;

beforeAll(async () => {
  i18n.addResourceBundle("en", "translation", { lub: en.lub }, true, true);
  i18n.addResourceBundle("zh", "translation", { lub: zh.lub }, true, true);
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

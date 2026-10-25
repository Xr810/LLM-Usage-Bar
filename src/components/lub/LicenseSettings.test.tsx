import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { LicenseSettings } from "./LicenseSettings";
import { activateLicense, communityLicenseStatus } from "./licenseMock";

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
    expect(before.emailMasked).toBeNull();
  });

  it("keeps Community build after activate and leaves the compare link inert", async () => {
    const user = userEvent.setup();
    render(<LicenseSettings />);
    expect(screen.getByTestId("license-tier")).toHaveTextContent(
      "Community build",
    );
    expect(screen.getByTestId("license-expires")).toHaveTextContent("—");
    expect(screen.getByTestId("license-email")).toHaveTextContent("—");

    await user.click(screen.getByRole("button", { name: "激活" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("COMMUNITY_BUILD");
    expect(alert).toHaveTextContent("需官方构建");
    expect(screen.getByTestId("license-tier")).toHaveTextContent(
      "Community build",
    );
    expect(screen.getByTestId("license-expires")).toHaveTextContent("—");

    const compare = screen.getByRole("link", { name: "比较 Free 与 Pro" });
    await user.click(compare);
    expect(compare).toHaveAttribute("href", "#free-vs-pro");
  });
});

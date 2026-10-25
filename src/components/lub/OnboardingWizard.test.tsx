import { readFileSync } from "node:fs";
import path from "node:path";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { OnboardingWizard } from "./OnboardingWizard";
import {
  detectTools,
  noteCopyKey,
  setClaudeSubscriptionProxy,
} from "./takeoverMock";

beforeAll(async () => {
  i18n.addResourceBundle("en", "translation", { lub: en.lub }, true, true);
  i18n.addResourceBundle("zh", "translation", { lub: zh.lub }, true, true);
  await i18n.changeLanguage("zh");
});

describe("takeover wizard", () => {
  it("maps note codes onto onboard copy and refuses an unacked proxy switch", () => {
    expect(noteCopyKey("env_overrides_plan")).toBe(
      "lub.onboard.code_acme_web_env_sets",
    );
    expect(
      detectTools().some((tool) =>
        tool.noteCodes.includes("env_overrides_plan"),
      ),
    ).toBe(true);
    expect(
      setClaudeSubscriptionProxy({ enabled: true, termsAckAt: null }),
    ).toEqual({
      ok: false,
      error: {
        code: "TERMS_ACK_REQUIRED",
        message: "Terms acknowledgement required",
      },
    });
    expect(
      setClaudeSubscriptionProxy({ enabled: true, termsAckAt: 1 }).ok,
    ).toBe(true);
  });

  it("keeps the proxy switch off until the terms box is checked", async () => {
    const user = userEvent.setup();
    render(<OnboardingWizard />);
    expect(screen.getByText("第 2 步 / 共 3 步")).toBeInTheDocument();
    expect(screen.getAllByText("受上限管控")).toHaveLength(2);
    expect(screen.getAllByText("仅监控")).toHaveLength(2);
    expect(screen.getByText(/acme-web\/\.env/)).toBeInTheDocument();
    expect(screen.queryByText("只统计")).toBeNull();

    const proxy = screen.getByRole("switch", { name: "Claude 订阅经代理" });
    expect(proxy).toHaveAttribute("aria-checked", "false");
    await user.click(proxy);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "TERMS_ACK_REQUIRED",
    );
    expect(proxy).toHaveAttribute("aria-checked", "false");

    await user.click(
      screen.getByRole("checkbox", { name: /可能违反服务商条款/ }),
    );
    await user.click(proxy);
    expect(proxy).toHaveAttribute("aria-checked", "true");
    expect(screen.queryByRole("alert")).toBeNull();

    await user.click(screen.getByRole("button", { name: "接入并设置日上限" }));
    await user.click(screen.getByRole("button", { name: "上一步" }));
    expect(proxy).toHaveAttribute("aria-checked", "true");

    const source = readFileSync(
      path.resolve(process.cwd(), "src/components/lub/OnboardingWizard.tsx"),
      "utf8",
    );
    expect(source).not.toContain("@tauri-apps");
    expect(source).not.toContain("invoke(");
  });
});

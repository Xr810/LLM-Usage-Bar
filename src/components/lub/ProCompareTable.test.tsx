import { readFileSync } from "node:fs";
import path from "node:path";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import { pricing } from "@/lib/pricing";
import { CommunityAllowPlaceholder } from "./CommunityAllowPlaceholder";
import { compareGroups } from "./proCompareModel";
import { ProCompareTable } from "./ProCompareTable";

beforeAll(async () => {
  i18n.addResourceBundle("en", "translation", { lub: en.lub }, true, true);
  i18n.addResourceBundle("zh", "translation", { lub: zh.lub }, true, true);
  await i18n.changeLanguage("zh");
});

describe("ProCompareTable", () => {
  it("rebuilds the v5.7 rows and keeps the trial button a no-op", async () => {
    const user = userEvent.setup();
    const ids = compareGroups.flatMap((group) =>
      group.rows.map((row) => row.id),
    );
    expect(ids).toContain("weekday");
    expect(ids).not.toContain("alerts");
    expect(pricing.phase).toBe("beta");
    expect(pricing.monthlyUsd).toBeNull();

    render(<ProCompareTable />);
    expect(screen.getByText("Beta 期间定价稍后公布")).toBeInTheDocument();
    expect(screen.getByText("可设，到了只提醒")).toBeInTheDocument();
    expect(screen.getAllByText("即将推出").length).toBeGreaterThan(0);
    expect(screen.getByText("7 天")).toBeInTheDocument();
    expect(screen.getByText("24 个月")).toBeInTheDocument();
    expect(screen.getByText("Beta")).toBeInTheDocument();
    expect(screen.queryByText("β")).toBeNull();
    expect(screen.queryByText("获取 Pro")).toBeNull();
    expect(screen.queryByText("超额提醒")).toBeNull();
    expect(screen.queryByText("一键停止（Kill）")).toBeNull();

    const trial = screen.getByRole("button", { name: "开始 7 天免费试用" });
    await user.click(trial);
    expect(trial).toBeInTheDocument();

    const source = readFileSync(
      path.resolve(process.cwd(), "src/components/lub/ProCompareTable.tsx"),
      "utf8",
    );
    expect(source).not.toContain("invoke(");
    expect(source).not.toContain("startTrial");
    expect(source).not.toContain("@tauri-apps");
  });
});

describe("CommunityAllowPlaceholder", () => {
  it("keeps both allow buttons visible and disabled", async () => {
    const user = userEvent.setup();
    render(<CommunityAllowPlaceholder />);
    const once = screen.getByRole("button", { name: "放行这 1 笔" });
    const more = screen.getByRole("button", { name: "今日再放行 $5" });
    expect(once).toHaveAttribute("aria-disabled", "true");
    expect(more).toHaveAttribute("aria-disabled", "true");
    expect(once).toBeVisible();
    expect(more).toBeVisible();
    await user.click(once);
    await user.click(more);
    expect(once).toBeVisible();

    const source = readFileSync(
      path.resolve(
        process.cwd(),
        "src/components/lub/CommunityAllowPlaceholder.tsx",
      ),
      "utf8",
    );
    expect(source).not.toContain("invoke(");
    expect(source).not.toContain("hold_allow");
    expect(source).not.toContain("grant_daily");
    expect(source).not.toContain("@tauri-apps");
  });
});

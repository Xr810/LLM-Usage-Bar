import { describe, expect, it } from "vitest";
import type { TrayCostQuality, TrayUsageStatus } from "@/types/trayUsage";
import {
  clampPercentForProgress,
  costQualityLabel,
  formatCount,
  formatPercent,
  formatResetTime,
  formatTokenCount,
  formatUpdatedTime,
  formatUsd,
  hasUsableBudget,
  hasUsablePercent,
  hasUsableUsd,
  providerIconName,
  quotaUnavailableReasonLabel,
  statusLabel,
  type TrayUsageTranslate,
} from "./trayUsagePresentation";

const t: TrayUsageTranslate = (key, options) =>
  String(options?.defaultValue ?? key);

describe("tray usage number formatting", () => {
  it.each([
    ["279.84", "$279.84"],
    ["0", "$0.00"],
    ["0.009999", "$0.009999"],
    ["0.000001", "$0.000001"],
    [null, "—"],
    ["", "—"],
    ["not-a-number", "—"],
    ["Infinity", "—"],
    ["-1", "—"],
  ])("formats USD value %s safely", (value, expected) => {
    expect(formatUsd(value, "en-US")).toBe(expected);
  });

  it.each([
    ["88", "88%"],
    ["50.25", "50.25%"],
    ["19.999", "19.999%"],
    ["20", "20%"],
    ["50", "50%"],
    ["50.001", "50.001%"],
    ["79.999", "79.999%"],
    ["80.001", "80.001%"],
    ["120", "120%"],
    [null, "—"],
    ["bad", "—"],
    ["-2", "—"],
  ])("formats percent value %s safely", (value, expected) => {
    expect(formatPercent(value)).toBe(expected);
  });

  it.each([
    ["120", 100],
    ["88.5", 88.5],
    ["-2", 0],
    [null, 0],
    ["bad", 0],
  ])("clamps progress value %s", (value, expected) => {
    expect(clampPercentForProgress(value)).toBe(expected);
  });

  it("distinguishes unavailable percent data from a real zero", () => {
    expect(hasUsablePercent("0")).toBe(true);
    expect(hasUsablePercent(null)).toBe(false);
    expect(hasUsablePercent("bad")).toBe(false);
    expect(hasUsablePercent("-1")).toBe(false);
  });

  it("distinguishes valid costs from positive configured budgets", () => {
    expect(hasUsableUsd("0")).toBe(true);
    expect(hasUsableUsd("")).toBe(false);
    expect(hasUsableUsd("-1")).toBe(false);
    expect(hasUsableBudget("10")).toBe(true);
    expect(hasUsableBudget("0")).toBe(false);
    expect(hasUsableBudget("bad")).toBe(false);
  });

  it("formats only safe non-negative counts and compact token totals", () => {
    expect(formatCount(12_345, "en-US")).toBe("12,345");
    expect(formatTokenCount(2_300_000_000, "en-US")).toBe("2.3B");
    expect(formatCount(-1, "en-US")).toBe("—");
    expect(formatCount(Number.NaN, "en-US")).toBe("—");
    expect(formatTokenCount(Number.MAX_SAFE_INTEGER + 1, "en-US")).toBe("—");
  });
});

describe("tray usage time formatting", () => {
  const now = new Date("2026-07-15T21:00:00Z");

  it("marks past resets pending without a negative countdown", () => {
    const result = formatResetTime(
      "2026-07-15T20:59:00Z",
      now,
      "en-US",
      "Pending refresh",
    );

    expect(result).toEqual({ text: "Pending refresh", pending: true });
    expect(result.text).not.toContain("-");
  });

  it("formats future reset duration with the locale relative-time formatter", () => {
    expect(
      formatResetTime("2026-07-15T22:30:00Z", now, "en-US", "Pending refresh"),
    ).toEqual({ text: "in 2 hours", pending: false });
  });

  it("returns a safe placeholder for invalid or absent reset values", () => {
    expect(formatResetTime("invalid", now, "en-US")).toEqual({
      text: "—",
      pending: false,
    });
    expect(formatResetTime(null, now, "en-US")).toEqual({
      text: "—",
      pending: false,
    });
  });

  it("formats last-success seconds without accepting unsafe timestamps", () => {
    expect(formatUpdatedTime(1_784_149_140, now, "en-US")).toBe("1 minute ago");
    expect(formatUpdatedTime(null, now, "en-US")).toBeNull();
    expect(
      formatUpdatedTime(Number.MAX_SAFE_INTEGER + 1, now, "en-US"),
    ).toBeNull();
  });
});

describe("tray usage presentation labels", () => {
  it.each<[TrayUsageStatus, string]>([
    ["green", "Healthy"],
    ["yellow", "Warning"],
    ["red", "Critical"],
    ["unknown", "Data unavailable"],
  ])("labels %s status", (status, expected) => {
    expect(statusLabel(status, t)).toBe(expected);
  });

  it.each<[TrayCostQuality, string]>([
    ["complete", "Complete"],
    ["estimated", "Estimated"],
    ["partial", "Partial data"],
    ["unavailable", "Unavailable"],
  ])("labels %s cost quality", (quality, expected) => {
    expect(costQualityLabel(quality, t)).toBe(expected);
  });

  it.each([
    ["chatgpt-subscription", "openai"],
    ["openai-api", "openai"],
    ["claude-subscription", "claude"],
    ["anthropic-api", "anthropic"],
    ["openrouter-api", "openrouter"],
    ["custom-provider", undefined],
    [null, undefined],
  ])("maps preset %s to its existing icon", (preset, expected) => {
    expect(providerIconName(preset)).toBe(expected);
  });

  it.each([
    ["invalid_quota_percent", "Usage percentage unavailable"],
    ["quota_unavailable", "Quota data unavailable"],
    ["invalid_reset_timestamp", "Reset time unavailable"],
    ["reset_pending_refresh", "Waiting for refreshed quota"],
  ])("localizes the safe quota reason code %s", (reason, expected) => {
    expect(quotaUnavailableReasonLabel(reason, t)).toBe(expected);
  });

  it.each([null, "", "raw backend error", "oauth_token=secret"])(
    "never exposes an unknown quota reason %s",
    (reason) => {
      expect(quotaUnavailableReasonLabel(reason, t)).toBeNull();
    },
  );
});

import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { UsageDateRangePicker } from "./UsageDateRangePicker";

afterEach(() => {
  vi.useRealTimers();
});

describe("UsageDateRangePicker", () => {
  it.each(["10:00", "09:59"])(
    "rejects an end time of %s at or before the start and allows correction",
    async (endTime) => {
      const user = userEvent.setup();
      const onApply = vi.fn();
      const start = new Date(2026, 9, 15, 10).getTime() / 1000;
      render(
        <UsageDateRangePicker
          selection={{
            preset: "custom",
            customStartDate: start,
            customEndDate: start + 3600,
          }}
          onApply={onApply}
          triggerLabel="Custom range"
        />,
      );
      await user.click(screen.getByRole("button", { name: "Custom range" }));
      const endInput = document.querySelectorAll('input[type="time"]')[1];
      fireEvent.change(endInput, { target: { value: endTime } });
      await user.click(screen.getByRole("button", { name: "common.confirm" }));

      expect(onApply).not.toHaveBeenCalled();
      expect(screen.getByRole("alert")).toHaveTextContent(
        "开始时间必须早于结束时间",
      );
      expect(screen.getByRole("dialog")).toBeVisible();

      fireEvent.change(endInput, { target: { value: "10:01" } });
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "common.confirm" }));
      expect(onApply).toHaveBeenCalledExactlyOnceWith({
        preset: "custom",
        customStartDate: start,
        customEndDate: start + 60,
        liveEndTime: false,
      });
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    },
  );

  it("offers a 1 year shortcut and explains that longer custom ranges are supported", async () => {
    const user = userEvent.setup();
    const onApply = vi.fn();

    render(
      <UsageDateRangePicker
        selection={{ preset: "30d" }}
        onApply={onApply}
        triggerLabel="Custom range"
      />,
    );

    await user.click(screen.getByRole("button", { name: "Custom range" }));

    expect(
      await screen.findByText("支持日期与时间，可选择超过 1 年的范围"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "1 year" }));

    expect(onApply).toHaveBeenCalledWith({ preset: "1y" });
  });

  it("limits presets to 30 days, disables earlier dates, and clamps Apply", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 9, 26, 12, 0, 0));
    const user = userEvent.setup();
    const onApply = vi.fn();

    render(
      <UsageDateRangePicker
        limited
        selection={{ preset: "30d" }}
        onApply={onApply}
        triggerLabel="Custom range"
      />,
    );

    await user.click(screen.getByRole("button", { name: "Custom range" }));
    const dialog = await screen.findByRole("dialog");
    for (const name of ["当天", "1d", "7d", "14d", "30d"]) {
      expect(within(dialog).getByRole("button", { name })).toBeInTheDocument();
    }
    expect(within(dialog).queryByRole("button", { name: "1 year" })).toBeNull();

    const blocked = dialog.querySelectorAll('button[aria-disabled="true"]');
    expect(blocked.length).toBeGreaterThan(0);
    const startInput = dialog.querySelector(
      'input[type="date"]',
    ) as HTMLInputElement;
    const startBefore = startInput.value;
    fireEvent.click(blocked[0]!);
    expect(startInput.value).toBe(startBefore);

    fireEvent.change(startInput, { target: { value: "2026-01-01" } });
    await user.click(screen.getByRole("button", { name: "common.confirm" }));

    expect(onApply).toHaveBeenCalledWith(
      expect.objectContaining({
        preset: "custom",
        customStartDate: Math.floor(new Date(2026, 8, 27).getTime() / 1000),
      }),
    );
    const applied = onApply.mock.calls[0]![0] as {
      customStartDate: number;
      customEndDate: number;
    };
    expect(applied.customStartDate).toBeLessThan(applied.customEndDate);
  });
});

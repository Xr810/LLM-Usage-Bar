import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TrayUsageFooter } from "./TrayUsageFooter";

describe("TrayUsageFooter", () => {
  it("groups secondary commands and keeps the primary command separate", () => {
    const onOpenDetails = vi.fn();
    const onOpenSettings = vi.fn();
    const onQuit = vi.fn();

    render(
      <TrayUsageFooter
        refreshing
        onOpenDetails={onOpenDetails}
        onRefresh={vi.fn()}
        onOpenSettings={onOpenSettings}
        onQuit={onQuit}
      />,
    );

    const actionGroup = screen.getByRole("group");
    expect(actionGroup).toHaveAccessibleName();
    expect(
      within(actionGroup).getByRole("button", { name: "Refreshing" }),
    ).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Open details" }));
    fireEvent.click(
      within(actionGroup).getByRole("button", { name: "Settings" }),
    );
    fireEvent.click(within(actionGroup).getByRole("button", { name: "Quit" }));

    expect(onOpenDetails).toHaveBeenCalledOnce();
    expect(onOpenSettings).toHaveBeenCalledOnce();
    expect(onQuit).toHaveBeenCalledOnce();
  });
});

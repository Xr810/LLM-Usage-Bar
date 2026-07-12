import { act, renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const checkForUpdateMock = vi.fn();

vi.mock("@/lib/updater", () => ({
  checkForUpdate: (...args: unknown[]) => checkForUpdateMock(...args),
}));

import { UpdateProvider, useUpdate } from "@/contexts/UpdateContext";

const wrapper = ({ children }: PropsWithChildren) => (
  <UpdateProvider>{children}</UpdateProvider>
);

describe("UpdateProvider", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    checkForUpdateMock.mockReset();
    checkForUpdateMock.mockResolvedValue({
      status: "unavailable",
      code: "managed_updates_unavailable",
    });
  });

  it("does not schedule an automatic update check at startup", () => {
    renderHook(() => useUpdate(), { wrapper });

    act(() => {
      vi.advanceTimersByTime(5_000);
    });

    expect(checkForUpdateMock).not.toHaveBeenCalled();
    vi.useRealTimers();
  });
});

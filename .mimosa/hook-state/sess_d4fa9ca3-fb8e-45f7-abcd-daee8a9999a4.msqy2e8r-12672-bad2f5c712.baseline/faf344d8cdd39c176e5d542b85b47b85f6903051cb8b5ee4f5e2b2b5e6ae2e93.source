import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "./dialog";

function renderDialog(variant: "default" | "fullscreen" = "default") {
  render(
    <Dialog open>
      <DialogContent variant={variant}>
        <DialogTitle>Surface title</DialogTitle>
        <DialogDescription>Surface description</DialogDescription>
      </DialogContent>
    </Dialog>,
  );
}

describe("Dialog surface markers", () => {
  it("marks the default content and overlay for adaptive glass styling", () => {
    renderDialog();

    expect(screen.getByRole("dialog")).toHaveAttribute(
      "data-surface-variant",
      "default",
    );
    expect(
      document.querySelector('[data-surface-overlay="default"]'),
    ).toBeInTheDocument();
  });

  it("keeps fullscreen content and overlay independently addressable", () => {
    renderDialog("fullscreen");

    expect(screen.getByRole("dialog")).toHaveAttribute(
      "data-surface-variant",
      "fullscreen",
    );
    expect(
      document.querySelector('[data-surface-overlay="fullscreen"]'),
    ).toBeInTheDocument();
  });
});

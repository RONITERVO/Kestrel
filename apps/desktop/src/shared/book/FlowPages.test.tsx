import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FlowPages } from "./FlowPages";

function layout(width: number, contentWidth: number) {
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) {
    return this.classList.contains("flow-viewport") ? width : 0;
  });
  // Content spreads across columns only once the column width is applied, as in a real layout.
  vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockImplementation(function (this: HTMLElement) {
    if (!this.classList.contains("flow-columns")) return 0;
    return this.style.columnWidth ? contentWidth : width;
  });
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("flowing pages", () => {
  it("counts pages only after the columns take the page width", () => {
    layout(500, 1548);
    const { container } = render(<FlowPages label="Report pages"><p>Long report</p></FlowPages>);
    expect(screen.getByRole("navigation", { name: "Report pages: page 1 of 3" })).toBeInTheDocument();
    expect(container.querySelector(".flow-columns")).toHaveStyle({ columnWidth: "500px" });
  });

  it("scrolls to the page it turns to and stays there", () => {
    layout(500, 1548);
    const { container } = render(<FlowPages label="Report pages"><p>Long report</p></FlowPages>);
    fireEvent.click(screen.getByRole("button", { name: "Next page of Report pages" }));
    fireEvent.click(screen.getByRole("button", { name: "Next page of Report pages" }));
    expect(screen.getByRole("navigation", { name: "Report pages: page 3 of 3" })).toBeInTheDocument();
    expect(container.querySelector(".flow-viewport")!.scrollLeft).toBe(2 * (500 + 48));
  });

  it("keeps its pages while its chapter is hidden", () => {
    layout(0, 0);
    const { container } = render(<FlowPages label="Hidden pages"><p>Long report</p></FlowPages>);
    expect(container.querySelector(".flow-columns")).not.toHaveAttribute("style");
    expect(screen.queryByRole("navigation", { name: /Hidden pages/ })).not.toBeInTheDocument();
  });
});

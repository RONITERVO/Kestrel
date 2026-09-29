import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
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

  it("turns to a spoken word's page by layout, also while the book is tilted on the desk", () => {
    layout(500, 1548);
    // On the desk the on-screen box is projected and would name page 1; the layout says page 3.
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => new DOMRect(40, 0, 10, 10));
    vi.spyOn(HTMLElement.prototype, "offsetLeft", "get").mockImplementation(function (this: HTMLElement) {
      return this.dataset.word ? 2 * (500 + 48) + 20 : 0;
    });
    const controller = { current: null as import("./FlowPages").FlowPagesController | null };
    render(<FlowPages label="Reply pages" controller={controller}><p>Start <mark data-word="yes">here</mark></p></FlowPages>);
    act(() => controller.current!.showElement(screen.getByText("here")));
    expect(screen.getByRole("navigation", { name: "Reply pages: page 3 of 3" })).toBeInTheDocument();
  });

  it("keeps its pages while its chapter is hidden", () => {
    layout(0, 0);
    const { container } = render(<FlowPages label="Hidden pages"><p>Long report</p></FlowPages>);
    expect(container.querySelector(".flow-columns")).not.toHaveAttribute("style");
    expect(screen.queryByRole("navigation", { name: /Hidden pages/ })).not.toBeInTheDocument();
  });
});

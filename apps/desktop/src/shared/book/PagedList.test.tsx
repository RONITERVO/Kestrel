import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PagedList } from "./PagedList";

class StillResizeObserver {
  observe() {}
  disconnect() {}
}

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", StillResizeObserver);
  // A 200 px frame and 40 px items: four items a page after the turner's room.
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(function (this: HTMLElement) {
    return this.classList.contains("paged-list") ? 200 : 0;
  });
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (this: HTMLElement) {
    return this.classList.contains("paged-list-item") ? 40 : 0;
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const takes = Array.from({ length: 10 }, (_, index) => `Take ${index + 1}`);

it("opens the selected item's page once and lets the reader browse away", () => {
  const list = (items: string[]) => <PagedList label="Takes" items={items} itemKey={(item) => item} selectedKey="Take 10" renderItem={(item) => <span>{item}</span>} />;
  const { rerender } = render(list([...takes]));
  expect(screen.getByRole("navigation", { name: "Takes: page 3 of 3" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Previous page of Takes" }));
  expect(screen.getByText("Take 5")).toBeInTheDocument();
  // A parent render with a new array of the same items (audio time updates) keeps the reader's page.
  rerender(list([...takes]));
  expect(screen.getByRole("navigation", { name: "Takes: page 2 of 3" })).toBeInTheDocument();
});

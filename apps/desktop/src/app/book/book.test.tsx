import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { BookShell } from "./BookShell";
import { CHAPTERS } from "./chapters";
import { mapThroughQuad, quadToMatrix3d } from "./homography";

describe("page plane homography", () => {
  it("is the identity for an untransformed page", () => {
    const quad = [{ x: 0, y: 0 }, { x: 400, y: 0 }, { x: 400, y: 300 }, { x: 0, y: 300 }] as const;
    expect(quadToMatrix3d(400, 300, quad)).toBe("matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1)");
  });

  it("maps every corner and the centre of a tilted page onto the projected quad", () => {
    const quad = [{ x: 40, y: 60 }, { x: 380, y: 30 }, { x: 420, y: 330 }, { x: 10, y: 280 }] as const;
    const corners = [{ x: 0, y: 0 }, { x: 800, y: 0 }, { x: 800, y: 600 }, { x: 0, y: 600 }];
    corners.forEach((corner, index) => {
      const mapped = mapThroughQuad(800, 600, quad, corner)!;
      expect(mapped.x).toBeCloseTo(quad[index].x, 6);
      expect(mapped.y).toBeCloseTo(quad[index].y, 6);
    });
    const centre = mapThroughQuad(800, 600, quad, { x: 400, y: 300 })!;
    expect(centre.x).toBeGreaterThan(10);
    expect(centre.x).toBeLessThan(420);
  });

  it("refuses a degenerate page instead of producing an invalid transform", () => {
    const flat = [{ x: 0, y: 0 }, { x: 0, y: 0 }, { x: 0, y: 0 }, { x: 0, y: 0 }] as const;
    expect(quadToMatrix3d(0, 300, flat)).toBeNull();
  });
});

describe("the Kestrel book", () => {
  it("labels every chapter tab and keeps earlier chapters on the left edge", () => {
    const onView = vi.fn();
    render(
      <BookShell view="research" onView={onView} banner={() => <header>banner</header>}>
        <main className="main-stage main-stage-research" />
      </BookShell>,
    );
    const nav = screen.getByRole("navigation", { name: "Kestrel sections" });
    for (const chapter of CHAPTERS) expect(screen.getByRole("button", { name: chapter.label })).toBeInTheDocument();
    expect(nav.querySelector('[data-chapter="setup"]')).toHaveAttribute("data-side", "left");
    expect(nav.querySelector('[data-chapter="control"]')).toHaveAttribute("data-side", "left");
    expect(screen.getByRole("button", { name: "Research" })).toHaveAttribute("aria-current", "page");
    expect(nav.querySelector('[data-chapter="research"]')).toHaveAttribute("data-side", "right");
    expect(nav.querySelector('[data-chapter="system"]')).toHaveAttribute("data-side", "right");
    fireEvent.click(screen.getByRole("button", { name: "Music" }));
    expect(onView).toHaveBeenCalledWith("music");
  });

  it("draws the book in CSS when WebGL is unavailable", () => {
    const { container } = render(
      <BookShell view="setup" onView={() => undefined} banner={({ webgl }) => <header>{webgl ? "3d" : "flat"}</header>}>
        <main className="main-stage main-stage-setup" />
      </BookShell>,
    );
    expect(container.querySelector(".app-shell")).toHaveClass("no-gl");
    expect(screen.getByText("flat")).toBeInTheDocument();
    expect(container.querySelector(".running-heads")).toHaveTextContent("I · Setup");
  });
});

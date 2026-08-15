import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { demoSnapshot } from "./demo";
import { SetupConsole } from "./Setup";

afterEach(cleanup);

describe("SetupConsole", () => {
  it("keeps installed components and editable expert locations visible", () => {
    render(<SetupConsole snapshot={demoSnapshot} onChanged={vi.fn()} onError={vi.fn()} />);
    expect(screen.getByRole("heading", { name: "Kestrel is ready." })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Bonsai assistant" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "MiniMax H3 Movie Studio" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Add another local model" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Observed model downloader" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Use existing files/ })).toBeInTheDocument();
  });

  it("shows one-click essentials on a clean machine", () => {
    const clean = {
      ...demoSnapshot,
      setup: {
        ...demoSnapshot.setup,
        ready: false,
        components: demoSnapshot.setup.components.map((item) => ({ ...item, status: "missing" })),
      },
    };
    render(<SetupConsole snapshot={clean} onChanged={vi.fn()} onError={vi.fn()} />);
    expect(screen.getByRole("button", { name: /Set up essentials/ })).toBeInTheDocument();
    expect(screen.getByText(/Interrupted downloads resume/)).toBeInTheDocument();
  });
});

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MovieStudio } from "./MovieStudio";

afterEach(cleanup);

describe("Kestrel Movie Studio", () => {
  it("presents a one-prompt offline production path", async () => {
    render(<MovieStudio advancedEnabled onError={vi.fn()} />);
    expect(screen.getByText(/Describe the movie/i)).toBeInTheDocument();
    expect(screen.getByText(/Offline Wikipedia is available/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Make movie/i })).toBeDisabled();
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "A tiny film about a lighthouse keeper" } });
    expect(screen.getByRole("button", { name: /Make movie/i })).toBeEnabled();
  });

  it("keeps full-context and expert production controls discoverable", () => {
    render(<MovieStudio advancedEnabled onError={vi.fn()} />);
    expect(screen.getByText("98,304 context")).toBeInTheDocument();
    expect(screen.getByText("32,768 output")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Advanced production controls/i }));
    expect(screen.getByLabelText("Maximum clips")).toHaveAttribute("max", "96");
    expect(screen.getByLabelText("Thinking budget")).toHaveValue(4096);
    expect(screen.getByLabelText("ComfyUI root")).toHaveValue("D:\\AI\\ComfyUI");
  });
});

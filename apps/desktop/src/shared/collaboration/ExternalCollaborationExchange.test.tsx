import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ExternalCollaborationExchange } from "./ExternalCollaborationExchange";

describe("ExternalCollaborationExchange", () => {
  it("keeps external work explicit and applies only a validated response", async () => {
    const apply = vi.fn();
    render(<ExternalCollaborationExchange
      title="Use another chat or agent"
      summary="A bounded exchange"
      buildRequest={() => "bounded request"}
      parseResponse={(text) => ({ text: JSON.parse(text).result })}
      onApply={apply}
    />);
    fireEvent.click(screen.getByText("Use another chat or agent"));
    fireEvent.change(screen.getByLabelText("Use another chat or agent JSON response"), { target: { value: JSON.stringify({ result: "draft" }) } });
    fireEvent.click(screen.getByRole("button", { name: /Validate & use editable draft/i }));
    expect(await screen.findByText(/Loaded as an unsaved editable draft/i)).toBeInTheDocument();
    expect(apply).toHaveBeenCalledWith({ text: "draft" });
  });

  it("disables copy, file, paste, and apply controls together", () => {
    const { container } = render(<ExternalCollaborationExchange title="Use a separate collaborator" summary="A bounded exchange" disabled buildRequest={() => "request"} parseResponse={(text) => text} onApply={vi.fn()} />);
    const exchange = within(container);
    fireEvent.click(exchange.getByText("Use a separate collaborator"));
    expect(exchange.getByRole("button", { name: /Copy request/i })).toBeDisabled();
    expect(exchange.getByLabelText("Choose Use a separate collaborator response")).toBeDisabled();
    expect(exchange.getByLabelText("Use a separate collaborator JSON response")).toBeDisabled();
    expect(exchange.getByRole("button", { name: /Validate & use editable draft/i })).toBeDisabled();
  });
});

import { cleanup, render } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { TextParagraphs } from "./TextParagraphs";

afterEach(cleanup);

it("shows each blank-line paragraph as its own block and keeps single line breaks", () => {
  const { container } = render(<TextParagraphs text={"First thought.\nSame paragraph.\n\n  \nSecond thought.\n\n\nThird."} />);
  const paragraphs = [...container.querySelectorAll(".text-paragraphs > p")].map((paragraph) => paragraph.textContent);
  expect(paragraphs).toEqual(["First thought.\nSame paragraph.", "Second thought.", "Third."]);
});

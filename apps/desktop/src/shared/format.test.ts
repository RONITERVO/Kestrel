import { expect, it } from "vitest";
import { formatMib } from "./format";

it("shows zero memory as a reading and only a missing value as unknown", () => {
  expect(formatMib(0)).toBe("0 MiB");
  expect(formatMib(512)).toBe("512 MiB");
  expect(formatMib(2048)).toBe("2.0 GiB");
  expect(formatMib(undefined)).toBe("—");
  expect(formatMib(null)).toBe("—");
});

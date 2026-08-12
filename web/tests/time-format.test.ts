import { describe, expect, it } from "vitest";

import {
  formatCompactTimestamp,
  formatDetailedTimestamp,
} from "@/lib/pages/time.ts";

describe("timestamp presentation", () => {
  it("formats compact timestamps as a UTC calendar date", () => {
    expect(formatCompactTimestamp("2026-08-10T00:30:00+09:00")).toBe(
      "2026-08-09",
    );
  });

  it("formats detailed timestamps to UTC minute precision", () => {
    expect(formatDetailedTimestamp("2026-08-10T00:30:45+09:00")).toBe(
      "2026-08-09 15:30 UTC",
    );
  });

  it("returns invalid input unchanged", () => {
    expect(formatCompactTimestamp("not-a-timestamp")).toBe("not-a-timestamp");
    expect(formatDetailedTimestamp("not-a-timestamp")).toBe(
      "not-a-timestamp",
    );
  });
});

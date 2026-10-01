import { describe, expect, it } from "vitest";
import { csv, money, permits } from "./api";

describe("analytics presentation", () => {
  it("keeps unknown usage separate from zero cost", () => {
    expect(money(null)).toBe("Unknown");
    expect(money(0)).toBe("$0.00");
    expect(money(1250000000)).toBe("$1.25");
  });
  it("escapes CSV values and spreadsheet formulas", () => {
    const result = csv(
      [{ model: '=HYPERLINK("x")', tokens: 7 }],
      ["model", "tokens"],
    );
    expect(result).toContain("'=");
    expect(result).toContain('""x""');
    expect(result).toContain('"7"');
  });
  it("honors individual scopes and administrator access", () => {
    expect(
      permits(
        { token_id: "reader", scopes: ["analytics:read"], csrf: null },
        "models:write",
      ),
    ).toBe(false);
    expect(
      permits(
        { token_id: "admin", scopes: ["admin"], csrf: null },
        "models:write",
      ),
    ).toBe(true);
  });
});

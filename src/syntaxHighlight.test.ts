import { describe, expect, it } from "vitest";
import { highlightCode } from "./syntaxHighlight";
import { contrastRatio, syntaxVariables } from "./syntaxTheme";

describe("code highlighting", () => {
  it("keeps keywords inside strings and comments from being highlighted as code", () => {
    const { label, tokens } = highlightCode('const text = "return true"; // const false\n', "js");
    expect(label).toBe("JavaScript");
    expect(tokens).toContainEqual({ text: "const", kind: "keyword" });
    expect(tokens).toContainEqual({ text: '"return true"', kind: "string" });
    expect(tokens).toContainEqual({ text: "// const false", kind: "comment" });
  });
  it.each([
    ["python", 'def hello():\n  # comment\n  return "hello 🌍"\n'],
    ["rust", 'fn main() { /* comment */ println!("hello"); }'],
    ["sql", "SELECT * FROM items WHERE name = 'O''Brien'; -- comment"],
    ["json", '{"message": "<script>hello</script>", "value": 12}'],
    ["html", '<div class="hello">hello & goodbye</div>'],
    ["bash", '# hello\necho "hello"\n'],
  ])("preserves every character of every streaming prefix for %s", (language, code) => {
    for (let end = 0; end <= code.length; end++) {
      const prefix = code.slice(0, end);
      expect(highlightCode(prefix, language).tokens.map((token) => token.text).join("")).toBe(prefix);
    }
  });
  it("leaves unknown, unspecified and very large blocks plain", () => {
    expect(highlightCode("const x = 1", "unknown").tokens).toEqual([{ text: "const x = 1" }]);
    expect(highlightCode("const x = 1", "").label).toBe("Plain text");
    const code = "x".repeat(100_001);
    expect(highlightCode(code, "js").tokens).toEqual([{ text: code }]);
  });
  it.each(["#0c0c0e", "#fdfdfd", "#888888", "#ffff00", "#123456"])("maintains text contrast against %s, including custom themes", (background) => {
    for (const mode of ["light", "dark"] as const) {
      for (const color of Object.values(syntaxVariables(mode, background))) {
        expect(contrastRatio(color, background)).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
});

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { MarkdownResponse } from "./MarkdownResponse";
import { responseWithSources, type ResearchSource } from "../research";

const render = (content: string, sources?: ResearchSource[]) =>
  renderToStaticMarkup(createElement(MarkdownResponse, { content, sources }));

describe("math responses", () => {
  it("renders inline math and display fractions, integrals, and matrices", () => {
    const html = render(String.raw`Inline $x^2$.

$$
\int_0^1 x\,dx = \frac{1}{2}, \quad \begin{pmatrix}1 & 0 \\ 0 & 1\end{pmatrix}
$$`);
    expect(html).toContain('class="katex"');
    expect(html).toContain('class="katex-display"');
    expect(html).toContain("<math");
    expect(html).toContain("<mfrac>");
    expect(html).toContain("<mtable");
    expect(html).not.toContain("katex-error");
  });

  it("preserves code examples, escaped currency, and ordinary Markdown", () => {
    const html = render('**Price:** \\$5 and \\$10. Code `$x^2$`.\n\n```latex\n$$x^2$$\n```');
    expect(html).toContain("<strong>Price:</strong> $5 and $10.");
    expect(html).toContain("<code>$x^2$</code>");
    expect(html).toContain('<code class="language-latex">$$x^2$$');
    expect(html).not.toContain('class="katex"');
  });

  it("survives every streamed prefix and shows invalid math without losing surrounding text", () => {
    const content = String.raw`Before $\frac{1}{2}$ after.

$$
\sqrt{x}
$$`;
    for (let end = 1; end <= content.length; end++) {
      expect(() => render(content.slice(0, end))).not.toThrow();
    }
    const invalid = render(String.raw`Before $\notARealCommand{x}$ after.`);
    expect(invalid).toContain("\\notARealCommand");
    expect(invalid).toContain("#fcd34d");
    expect(render(String.raw`Before $\frac{1}{$ after.`)).toContain("katex-error");
    expect(invalid).toContain("Before");
    expect(invalid).toContain("after.");
  });

  it("keeps citations outside math and preserves LaTeX when copying", () => {
    const sources = [{ id: 1, title: "Source", url: "https://example.com/", excerpt: "", publishedAt: null }];
    const content = String.raw`$a[1] + b$ follows [1].`;
    const html = render(content, sources);
    expect(html.match(/href=/g)).toHaveLength(1);
    expect(html).toContain('href="https://example.com/"');
    expect(html).toContain('class="katex"');
    expect(responseWithSources(content, sources)).toMatch(/^\$a\[1\] \+ b\$ follows \[1\]\./);
  });

  it("blocks HTML and untrusted LaTeX links and image commands", () => {
    const html = render(String.raw`<script>alert(1)</script>

$\href{https://untrusted.example}{click}$

$\includegraphics{https://untrusted.example/pixel.png}$`);
    expect(html).not.toContain("<script");
    expect(html).not.toContain("href=");
    expect(html).not.toContain("<img");
  });
});

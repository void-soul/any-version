import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown from "react-markdown";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";

function render(markdown: string) {
  return renderToStaticMarkup(
    <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
      {markdown}
    </ReactMarkdown>,
  );
}

describe("shared Markdown math dependency compatibility", () => {
  it("renders mathematical markup with the upgraded KaTeX renderer", () => {
    const html = render("$E = mc^2$");
    expect(html).toContain('class="katex"');
    expect(html).toContain("<math");
    expect(html).not.toContain("katex-error");
  });

  it("keeps untrusted TeX links disabled", () => {
    const html = render(String.raw`$\href{javascript:alert(1)}{click}$`);
    expect(html).not.toMatch(/<a\b/i);
    expect(html).not.toMatch(/href=["']javascript:/i);
  });
});

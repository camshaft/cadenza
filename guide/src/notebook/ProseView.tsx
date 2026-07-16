/// Render a notebook prose cell's parsed markdown blocks (from parseProse) as React, reusing the guide's
/// Prose.tsx look for headings / paragraphs / inline code. This is the presentational half of the prose
/// path — parseProse.ts owns the (pure, tested) parse; this owns only the JSX mapping.

import type { ReactNode } from "react";
import { parseProse, type Block, type Inline } from "./parseProse.ts";
import { H1, H2, P, C } from "../components/Prose.tsx";

/// Render a list of inline spans to React nodes (bold / italic / code / links / text).
function renderInline(spans: Inline[]): ReactNode[] {
  return spans.map((s, i) => {
    switch (s.t) {
      case "strong":
        return <strong key={i} className="font-semibold text-slate-100">{s.text}</strong>;
      case "em":
        return <em key={i} className="italic">{s.text}</em>;
      case "code":
        return <C key={i}>{s.text}</C>;
      case "link":
        return (
          <a key={i} href={s.href} className="text-cadenza-400 underline hover:text-cadenza-300">
            {s.text}
          </a>
        );
      case "text":
      default:
        return <span key={i}>{s.text}</span>;
    }
  });
}

/// Render one parsed block. Headings h1/h2 reuse Prose's H1/H2; h3–h6 fall back to a styled heading (the
/// guide's Prose only defines H1/H2). Lists + blockquotes get lightweight Tailwind matching the Prose look.
function renderBlock(block: Block, key: number): ReactNode {
  switch (block.t) {
    case "heading": {
      const kids = renderInline(block.spans);
      if (block.level === 1) return <H1 key={key}>{kids}</H1>;
      if (block.level === 2) return <H2 key={key}>{kids}</H2>;
      return (
        <h3 key={key} className="mt-6 mb-2 text-lg font-semibold text-slate-200">
          {kids}
        </h3>
      );
    }
    case "paragraph":
      return <P key={key}>{renderInline(block.spans)}</P>;
    case "list": {
      const items = block.items.map((it, i) => <li key={i}>{renderInline(it)}</li>);
      return block.ordered ? (
        <ol key={key} className="my-4 list-decimal space-y-1 pl-6 text-slate-300">{items}</ol>
      ) : (
        <ul key={key} className="my-4 list-disc space-y-1 pl-6 text-slate-300">{items}</ul>
      );
    }
    case "blockquote":
      return (
        <blockquote key={key} className="my-4 border-l-2 border-slate-600 pl-4 text-slate-400 italic">
          {renderInline(block.spans)}
        </blockquote>
      );
  }
}

/// Render a prose cell's markdown source.
export function ProseView({ markdown }: { markdown: string }) {
  const blocks = parseProse(markdown);
  return <div className="notebook-prose">{blocks.map((b, i) => renderBlock(b, i))}</div>;
}

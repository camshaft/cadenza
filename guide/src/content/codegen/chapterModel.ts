/// The PURE core of the guide's sexp→TSX codegen (cadenza-docs I4): parse a guide `(chapter …)` s-expr into
/// a typed chapter MODEL, then render that model to a TSX chapter-module string. NO filesystem, NO React —
/// `node --test` covers the whole transform directly (same pure-core-first discipline as the explorer's
/// fileModel: the `.mjs` build script that reads `chapters/*.sexp` + writes `chapters/*.tsx` is a thin FS
/// wrapper over these two functions, added in the next increment once the round-trip is pinned here).
///
/// WHY a model in the middle (not sexp→string directly): the registry (chapters.ts) is DERIVED from the
/// same (slug/title/pillar/section/blurb) the chapter carries — a model lets both the TSX renderer and the
/// registry-entry emitter read one parsed structure, so they can't drift (v-guide-editor's zero-drift
/// requirement). The model also makes the editorial-gate contract testable: exercises-count = count of
/// `exercise` blocks, teaching-status = pillar/section — all readable off the model, not re-regexed.
///
/// SCHEMA (ratified w/ v-guide + v-guide-editor, design §2.6/§4.4/§9.5): heads mirror Prose.tsx 1:1.
///   (chapter (slug "…") (title "…") (pillar "language"|"platform") (section "…")? (blurb "…")?
///     (lede <inline>…)? <block>…)
///   block:  (h2 <inline>…) | (p <inline>…) | (note <inline>…)   [runnable/exercise/why deferred to I5]
///   inline: "text" | (em <inline>…) | (c "…") | (br)
///         | (link (slug "chap") <inline>…)        -> internal chapter link (dead-link-checked)
///         | (app-link (route "/x") <inline>…)     -> app-route link (route-coverage-checked)

import { parseSexpr, unquoteAtom, isAtom, isList, type Node } from "../../notebook/sexpr.ts";

export type Pillar = "language" | "platform";

/// An inline (prose-level) node: text, emphasis, inline code, a hard break, or one of the two link kinds.
export type Inline =
  | { kind: "text"; text: string }
  | { kind: "em"; children: Inline[] }
  | { kind: "code"; text: string }
  | { kind: "br" }
  | { kind: "link"; slug: string; children: Inline[] }
  | { kind: "app-link"; route: string; children: Inline[] };

/// A block-level node. (runnable/exercise/why are I5 — not modeled yet; the parser rejects them loudly so
/// a chapter that needs them isn't silently half-converted.)
export type Block =
  | { kind: "h2"; children: Inline[] }
  | { kind: "p"; children: Inline[] }
  | { kind: "note"; children: Inline[] };

/// The parsed chapter model — the single structure both the TSX renderer and the (future) registry emitter
/// read, so title/slug/pillar can't drift between the rendered chapter and its chapters.ts entry.
export interface ChapterModel {
  slug: string;
  title: string;
  pillar: Pillar;
  section?: string;
  blurb?: string;
  lede?: Inline[];
  blocks: Block[];
}

export type ParseResult =
  | { ok: true; model: ChapterModel }
  | { ok: false; reason: string };

/// Parse a guide chapter `.sexp` source into a ChapterModel, or a human-readable reason it can't be parsed
/// (unknown head, missing required attr, malformed link). A discriminated result — the build script surfaces
/// the reason with the file name rather than throwing a bare stack.
export function parseChapter(source: string): ParseResult {
  let root: Node;
  try {
    root = parseSexpr(source);
  } catch (e) {
    return { ok: false, reason: `s-expr parse failed: ${e instanceof Error ? e.message : String(e)}` };
  }
  if (!isList(root) || root.list.length === 0 || !headIs(root, "chapter")) {
    return { ok: false, reason: "top-level form must be a (chapter …)" };
  }
  const forms = root.list.slice(1);

  // Attribute forms (slug/title/pillar/section/blurb) come first; (lede …) and the blocks follow. We scan
  // once, routing each child by its head — attrs fill the metadata, everything else is body content.
  let slug: string | undefined;
  let title: string | undefined;
  let pillar: Pillar = "language";
  let section: string | undefined;
  let blurb: string | undefined;
  let lede: Inline[] | undefined;
  const blocks: Block[] = [];

  for (const form of forms) {
    if (!isList(form) || form.list.length === 0 || !isAtom(form.list[0])) {
      return { ok: false, reason: "every chapter child must be a (head …) form" };
    }
    const head = (form.list[0] as { atom: string }).atom;
    const args = form.list.slice(1);
    switch (head) {
      case "slug": { const s = soleString(args); if (s == null) return attrErr("slug"); slug = s; break; }
      case "title": { const s = soleString(args); if (s == null) return attrErr("title"); title = s; break; }
      case "pillar": {
        const s = soleString(args);
        if (s !== "language" && s !== "platform") return { ok: false, reason: `(pillar …) must be "language" or "platform", got ${s == null ? "nothing" : `"${s}"`}` };
        pillar = s; break;
      }
      case "section": { const s = soleString(args); if (s == null) return attrErr("section"); section = s; break; }
      case "blurb": { const s = soleString(args); if (s == null) return attrErr("blurb"); blurb = s; break; }
      case "lede": { const r = parseInlines(args); if (!r.ok) return r; lede = r.inlines; break; }
      case "h2": case "p": case "note": {
        const r = parseInlines(args);
        if (!r.ok) return r;
        blocks.push({ kind: head, children: r.inlines });
        break;
      }
      case "runnable": case "exercise": case "why":
        return { ok: false, reason: `(${head} …) lowering is deferred to I5 — this chapter can't be converted yet (design §4.4)` };
      default:
        return { ok: false, reason: `unknown chapter head (${head} …)` };
    }
  }

  if (slug == null) return { ok: false, reason: "chapter is missing (slug …)" };
  if (title == null) return { ok: false, reason: "chapter is missing (title …)" };
  return { ok: true, model: { slug, title, pillar, section, blurb, lede, blocks } };
}

type InlinesResult = { ok: true; inlines: Inline[] } | { ok: false; reason: string };

/// Parse a sequence of inline nodes (a block's children): bare string atoms become text, `(head …)` forms
/// become em/c/br/link/app-link. A bare non-string atom (e.g. a stray symbol) is an error — guide prose is
/// quoted strings + inline heads only.
function parseInlines(nodes: Node[]): InlinesResult {
  const out: Inline[] = [];
  for (const n of nodes) {
    if (isAtom(n)) {
      const raw = n.atom;
      if (!(raw.startsWith('"') && raw.endsWith('"'))) {
        return { ok: false, reason: `bare inline token "${raw}" — prose text must be a quoted "string"` };
      }
      out.push({ kind: "text", text: unquoteAtom(raw) });
      continue;
    }
    if (n.list.length === 0 || !isAtom(n.list[0])) return { ok: false, reason: "malformed inline form" };
    const head = (n.list[0] as { atom: string }).atom;
    const args = n.list.slice(1);
    switch (head) {
      case "em": { const r = parseInlines(args); if (!r.ok) return r; out.push({ kind: "em", children: r.inlines }); break; }
      case "c": { const s = soleString(args); if (s == null) return { ok: false, reason: "(c …) needs one string" }; out.push({ kind: "code", text: s }); break; }
      case "br": { if (args.length !== 0) return { ok: false, reason: "(br) takes no arguments" }; out.push({ kind: "br" }); break; }
      case "link": {
        const slug = attrString(args, "slug");
        if (slug == null) return { ok: false, reason: "(link …) needs a (slug \"chapter\") target" };
        const r = parseInlines(argsAfterAttr(args, "slug")); if (!r.ok) return r;
        out.push({ kind: "link", slug, children: r.inlines }); break;
      }
      case "app-link": {
        const route = attrString(args, "route");
        if (route == null) return { ok: false, reason: "(app-link …) needs a (route \"/x\") target" };
        const r = parseInlines(argsAfterAttr(args, "route")); if (!r.ok) return r;
        out.push({ kind: "app-link", route, children: r.inlines }); break;
      }
      default:
        return { ok: false, reason: `unknown inline head (${head} …)` };
    }
  }
  return { ok: true, inlines: out };
}

// ---- small parse helpers ----

function headIs(n: Node, head: string): boolean {
  return isList(n) && n.list.length > 0 && isAtom(n.list[0]) && (n.list[0] as { atom: string }).atom === head;
}

/// The single string argument of an attribute form, or null if it isn't exactly one quoted string.
function soleString(args: Node[]): string | null {
  if (args.length !== 1 || !isAtom(args[0])) return null;
  const raw = (args[0] as { atom: string }).atom;
  if (!(raw.startsWith('"') && raw.endsWith('"'))) return null;
  return unquoteAtom(raw);
}

/// The string inside a leading `(<name> "…")` attribute form among args, or null if absent/malformed.
function attrString(args: Node[], name: string): string | null {
  const first = args[0];
  if (first == null || !isList(first) || !headIs(first, name)) return null;
  return soleString(first.list.slice(1));
}

/// The args after a leading `(<name> …)` attribute form (the inline children of a link node).
function argsAfterAttr(args: Node[], name: string): Node[] {
  const first = args[0];
  if (first != null && isList(first) && headIs(first, name)) return args.slice(1);
  return args;
}

function attrErr(name: string): ParseResult {
  return { ok: false, reason: `(${name} …) must be a single quoted string` };
}

// ---- rendering: ChapterModel -> TSX module string ----

/// The set of Prose components a rendered chapter imports. Kept minimal + sorted so the generated import line
/// is deterministic and tsc-clean (noUnusedLocals ON → we import EXACTLY the heads the chapter uses).
const PROSE_TAGS: Record<Block["kind"] | "h1" | "lede", string> = {
  h1: "H1", lede: "Lede", h2: "H2", p: "P", note: "Note",
};

/// Render a ChapterModel to a `@generated` TSX chapter-module string. Deterministic: the same model always
/// yields byte-identical output (imports sorted, fixed indentation), so the codegen determinism gate holds.
export function renderChapter(model: ChapterModel): string {
  const usedProse = new Set<string>(["H1"]); // H1 always (the title)
  if (model.lede) usedProse.add("Lede");
  for (const b of model.blocks) usedProse.add(PROSE_TAGS[b.kind]);

  // Walk every inline to learn what to import: an inline `(c …)` renders <C> (a Prose import, tracked in
  // usedProse just like the block tags — else a chapter using inline code emits <C> with no import and fails
  // tsc's noUnusedLocals/undefined-name check); a chapter link / app link renders <Link> (react-router-dom).
  // Recurse into `em` since any inline can nest. We import ONLY what's actually used.
  const flags = { link: false, appLink: false };
  const scan = (ins: Inline[]) => {
    for (const i of ins) {
      if (i.kind === "code") usedProse.add("C");
      else if (i.kind === "link") flags.link = true;
      else if (i.kind === "app-link") flags.appLink = true;
      else if (i.kind === "em") scan(i.children);
    }
  };
  if (model.lede) scan(model.lede);
  for (const b of model.blocks) scan(b.children);

  const proseImport = `import { ${[...usedProse].sort().join(", ")} } from "../../components/Prose.tsx";`;
  const lines: string[] = [
    "// @generated DO NOT EDIT — rendered from the chapter's .sexp by the guide sexp→TSX codegen (chapterModel.ts).",
    proseImport,
  ];
  // Links render via the shared styled components (Ch = internal chapter link, AppLink = app route) — NOT a
  // bare react-router <Link> — so a generated chapter styles links exactly as the hand-written ones do. Import
  // only the kinds actually used (tsc noUnusedLocals), sorted for determinism.
  const linkImports = [flags.link && "Ch", flags.appLink && "AppLink"].filter(Boolean).sort();
  if (linkImports.length) lines.push(`import { ${linkImports.join(", ")} } from "../../components/ChapterLink.tsx";`);
  lines.push("");
  lines.push(`export default function ${pascal(model.slug)}() {`);
  lines.push("  return (");
  lines.push("    <article>");
  lines.push(`      <H1>${escapeText(model.title)}</H1>`);
  if (model.lede) lines.push(`      <Lede>${renderInlines(model.lede)}</Lede>`);
  for (const b of model.blocks) {
    const tag = PROSE_TAGS[b.kind];
    lines.push(`      <${tag}>${renderInlines(b.children)}</${tag}>`);
  }
  lines.push("    </article>");
  lines.push("  );");
  lines.push("}");
  return lines.join("\n") + "\n";
}

/// Render inline children to JSX text. `br` → `<br />`; a chapter link → `<Ch to="/slug">`; an app link →
/// `<AppLink to="/route">` (the shared styled components). Text is JSX-escaped (`{`, `}`, `<` would otherwise
/// be parsed as JSX).
function renderInlines(inlines: Inline[]): string {
  return inlines.map(renderInline).join("");
}

function renderInline(i: Inline): string {
  switch (i.kind) {
    case "text": return escapeText(i.text);
    case "em": return `<em>${renderInlines(i.children)}</em>`;
    case "code": return `<C>${escapeText(i.text)}</C>`;
    case "br": return "<br />";
    case "link": return `<Ch to="/${i.slug}">${renderInlines(i.children)}</Ch>`;
    case "app-link": return `<AppLink to="${i.route}">${renderInlines(i.children)}</AppLink>`;
  }
}

/// Escape text for a JSX text node. Wrap a run in a `{"…"}` string expression — so it renders literally —
/// when it contains either (a) a JSX-significant character `{`/`}`/`<`/`>`, or (b) whitespace JSX would
/// COLLAPSE: a run of 2+ consecutive spaces, or a tab/newline. Case (b) is v-guide's (br)-indent requirement
/// made robust: JSX collapses any whitespace run to a single space, so a pseudocode line like
/// `"  run S's reducer"` after a `<br />` would lose its two-space indent as bare text. Wrapping it as
/// `{"  run …"}` preserves the exact spaces regardless of the renderer's line layout. A single boundary
/// space between words/inline elements (`"run "` before `<C>`) is same-line-safe in JSX, so the common case
/// still passes through untouched — only genuine indentation/multi-space formatting is wrapped.
function escapeText(text: string): string {
  if (/[{}<>]/.test(text) || /\s\s|[\t\n]/.test(text)) return `{${JSON.stringify(text)}}`;
  return text;
}

/// A slug → PascalCase component name ("platform-overview" → "PlatformOverview"). Matches the existing
/// hand-written file names so chapters.ts `import("./chapters/<Name>.tsx")` still resolves.
function pascal(slug: string): string {
  return slug.split(/[-_]/).filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join("");
}

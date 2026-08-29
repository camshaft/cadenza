/// TSX→sexp bootstrap CONVERTER (cadenza-docs I5 migration accelerator) — turns a hand-written chapter
/// `.tsx` into the `.sexp` source of truth the guide emit-xtask (xtask-codegen-guide) renders back. This is
/// a ONE-TIME migration tool (not part of the build); each conversion is editor-co-verified + gated by
/// check:codegen-sync (byte .tsx==codegen(.sexp)) + check:examples. Verify a conversion out-of-band with:
///   xtask-codegen-guide <chapter.sexp>  → compare its normalized visible text to the old hand-written .tsx.
///
/// THIS INCREMENT: the INLINE parser — the converter's reusable core. Parses a JSX inline run (text, <C>,
/// <em>, <strong>, <Ch>/<Link>/<AppLink>, <br/>, `{" "}` spaces, `{`…`}` escaped text) into the .sexp inline
/// forms (bare "string", (em …), (c "…"), (strong …), (link (slug "…") …), (app-link (route "…") …), (br)).
/// The block-level + chapters.ts-meta layers wrap this next. Run `--self-test` to check the parser.

/// App-showcase routes: a <Link to=…> to one of these is an (app-link (route …)); any other target is a
/// chapter cross-ref (link (slug …)). All <Ch> are chapter slugs. (Editor's link-split rule.)
const APP_ROUTES = new Set(["/cad", "/calculator", "/notebook", "/playground"]);

/// Escape a plain-text run as a .sexp double-quoted string.
function sexpString(text) {
  return `"${text.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

/// Parse a JSX inline run → an array of .sexp inline-form strings. `s` is the raw JSX between a block tag's
/// `>` and its `</tag>` (already extracted by the block layer). Recursive for nested marks.
export function parseInline(s) {
  const out = [];
  let text = ""; // accumulating plain-text run
  let i = 0;
  const flush = () => {
    // JSX collapses runs of whitespace to a single space; the visible text is the collapsed form.
    const collapsed = text.replace(/\s+/g, " ");
    if (collapsed !== "") out.push(sexpString(collapsed));
    text = "";
  };
  while (i < s.length) {
    const c = s[i];
    if (c === "{") {
      // `{" "}` / `{' '}` → a literal space; `{`…`}` → escaped text (unwrap the template literal).
      const close = matchBrace(s, i);
      const inner = s.slice(i + 1, close).trim();
      if (/^(["'`]) \1$/.test(inner)) {
        text += " ";
      } else if (inner.startsWith("`") && inner.endsWith("`")) {
        text += cookTemplate(inner.slice(1, -1));
      } else {
        // A JSX expression we don't model (rare in prose) — keep its cooked string if it's a quoted literal.
        const m = /^(["'])([\s\S]*)\1$/.exec(inner);
        if (m) text += m[2];
      }
      i = close + 1;
      continue;
    }
    if (c === "<") {
      if (s.startsWith("<br", i)) {
        flush();
        out.push("(br)");
        i = s.indexOf(">", i) + 1;
        continue;
      }
      const tag = /^<([A-Za-z]+)([^>]*)>/.exec(s.slice(i));
      if (tag) {
        flush();
        const name = tag[1];
        const attrs = tag[2];
        const openLen = tag[0].length;
        const closeTag = `</${name}>`;
        const end = findMatchingClose(s, i + openLen, name);
        const inner = s.slice(i + openLen, end);
        out.push(renderMark(name, attrs, inner));
        i = end + closeTag.length;
        continue;
      }
    }
    text += c;
    i++;
  }
  flush();
  return out;
}

/// Render one inline mark `<name attrs>inner</name>` → its .sexp form.
function renderMark(name, attrs, inner) {
  const kids = parseInline(inner).join(" ");
  switch (name) {
    case "em":
      return `(em ${kids})`;
    case "strong":
      return `(strong ${kids})`;
    case "C":
      // <C> holds a code string (its inner is text, possibly a `{`…`}` template).
      return `(c ${sexpString(inlineText(inner))})`;
    case "Ch": {
      const slug = attrTo(attrs).replace(/^\//, "");
      return `(link (slug ${sexpString(slug)}) ${kids})`;
    }
    case "Link": {
      const to = attrTo(attrs);
      if (APP_ROUTES.has(to)) return `(app-link (route ${sexpString(to)}) ${kids})`;
      return `(link (slug ${sexpString(to.replace(/^\//, ""))}) ${kids})`;
    }
    case "AppLink":
      return `(app-link (route ${sexpString(attrTo(attrs))}) ${kids})`;
    default:
      // Unknown mark — keep its children (don't drop content); flag for review.
      return kids;
  }
}

function attrTo(attrs) {
  const m = /\bto=(?:"([^"]*)"|\{`([^`]*)`\}|\{"([^"]*)"\})/.exec(attrs);
  return (m && (m[1] ?? m[2] ?? m[3])) || "";
}

/// The visible text of a simple inline (for <C> content): unwrap `{`…`}` / `{"…"}` or take the raw text.
function inlineText(s) {
  const t = s.trim();
  if (t.startsWith("{`") && t.endsWith("`}")) return cookTemplate(t.slice(2, -2));
  const m = /^\{(["'])([\s\S]*)\1\}$/.exec(t);
  if (m) return m[2];
  return t.replace(/\s+/g, " ");
}

/// Interpret backslash escapes in a captured template-literal body the way JS would (mirrors the guide's
/// cookTemplate: `\\` → `\`, `` \` `` → `` ` ``, `\n` etc.). Minimal — prose templates are mostly literal.
function cookTemplate(raw) {
  return raw.replace(/\\([\s\S])/g, (_, ch) => {
    switch (ch) {
      case "n": return "\n";
      case "t": return "\t";
      case "\\": return "\\"; // \\ → \
      case "`": return "`"; // \` → `
      case "$": return "$"; // \$ → $
      default: return "\\" + ch; // KEEP the backslash for a code escape like \x (not a JS string escape)
    }
  });
}

/// `{ … }` brace matcher (handles nested braces + backtick strings) — returns the index of the matching `}`.
function matchBrace(s, open) {
  let depth = 0;
  for (let i = open; i < s.length; i++) {
    const c = s[i];
    if (c === "`") {
      i = s.indexOf("`", i + 1);
      if (i === -1) return s.length - 1;
    } else if (c === "{") depth++;
    else if (c === "}") {
      depth--;
      if (depth === 0) return i;
    }
  }
  return s.length - 1;
}

/// Find the matching `</name>` for an already-opened `<name …>` at `from`, honoring nested same-name tags.
function findMatchingClose(s, from, name) {
  let depth = 1;
  let i = from;
  const openRe = new RegExp(`<${name}\\b`, "g");
  const closeTag = `</${name}>`;
  while (i < s.length) {
    const nextClose = s.indexOf(closeTag, i);
    openRe.lastIndex = i;
    const openM = openRe.exec(s);
    const nextOpen = openM && openM.index < nextClose ? openM.index : -1;
    if (nextClose === -1) return s.length;
    if (nextOpen !== -1 && nextOpen < nextClose) {
      depth++;
      i = nextOpen + name.length + 1;
    } else {
      depth--;
      if (depth === 0) return nextClose;
      i = nextClose + closeTag.length;
    }
  }
  return s.length;
}

// ---- block layer: a chapter .tsx → .sexp (meta from chapters.ts + article blocks) ----
import { readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { extractFilesProp } from "./example-extract.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));

/// Read the chapters.ts registry entry for a chapter file → { slug, title, pillar, section, blurb }.
function chapterMeta(tsxPath) {
  const base = basename(tsxPath); // e.g. Philosophy.tsx
  const reg = readFileSync(join(HERE, "../src/content/chapters.ts"), "utf8");
  // Find the entry object whose Component imports ./chapters/<base>; the fields precede the Component line.
  const idx = reg.indexOf(`./chapters/${base}`);
  if (idx === -1) throw new Error(`chapters.ts: no entry importing ./chapters/${base}`);
  const start = reg.lastIndexOf("{", idx);
  const entry = reg.slice(start, idx);
  const field = (name) => {
    const m = new RegExp(`${name}:\\s*"([^"]*)"`).exec(entry);
    return m ? m[1] : null;
  };
  return {
    slug: field("slug"),
    title: field("title"),
    pillar: field("pillar") ?? "language", // pillarOf default
    section: field("section"),
    blurb: field("blurb"),
  };
}

/// The `<article>…</article>` inner body of a chapter component.
function articleBody(tsx) {
  const m = /<article>([\s\S]*)<\/article>/.exec(tsx);
  if (!m) throw new Error("no <article>…</article> found");
  return m[1];
}

/// Emit a prose block `(tag <inline>…)` from a block's inner JSX (outer whitespace trimmed — a block edge
/// carries no space, matching the pilot .sexp).
function proseBlock(tag, inner) {
  return `  (${tag} ${parseInline(inner.trim()).join(" ")})`;
}

/// A runnable/exercise attr string (template-cooked), from example-extract's grab-equivalent on the element.
function attr(el, name) {
  const tl = new RegExp(`${name}=\\{\`([\\s\\S]*?)\`\\}`).exec(el);
  if (tl) return cookTemplate(tl[1]);
  const s = new RegExp(`${name}="([^"]*)"`).exec(el);
  return s ? s[1] : null;
}

/// Convert a whole chapter .tsx → its .sexp document (string).
export function convertChapter(tsxPath) {
  const tsx = readFileSync(tsxPath, "utf8");
  const meta = chapterMeta(tsxPath);
  const body = articleBody(tsx);

  // Walk top-level block elements in document order. Elements are self-closing (Runnable/Exercise) or paired
  // (H1/Lede/P/H2/Note/Why). Chunk by the next block-open position (block elements never nest each other).
  const BLOCK = /<(H1|Lede|P|H2|Note|Why|Runnable|Exercise)\b/g;
  const opens = [];
  let m;
  while ((m = BLOCK.exec(body))) opens.push({ tag: m[1], at: m.index });
  const lines = [];
  for (let k = 0; k < opens.length; k++) {
    const { tag, at } = opens[k];
    const end = k + 1 < opens.length ? opens[k + 1].at : body.length;
    const chunk = body.slice(at, end);
    if (tag === "Runnable") {
      lines.push(runnableBlock(chunk, tsxPath));
    } else if (tag === "Exercise") {
      lines.push(exerciseBlock(chunk));
    } else if (tag === "Why") {
      const tenet = /tenet="([^"]*)"/.exec(chunk)?.[1] ?? "";
      const inner = /<Why[^>]*>([\s\S]*?)<\/Why>/.exec(chunk)?.[1] ?? "";
      lines.push(`  (why (tenet ${sexpString(tenet)}) ${parseInline(inner.trim()).join(" ")})`);
    } else {
      const inner = new RegExp(`<${tag}>([\\s\\S]*?)</${tag}>`).exec(chunk)?.[1] ?? "";
      const t = tag === "Lede" ? "lede" : tag.toLowerCase(); // H1→h1, H2→h2, P→p, Note→note, Lede→lede
      if (tag === "H1") continue; // title comes from meta, not a block
      lines.push(proseBlock(t, inner));
    }
  }

  let out = `(chapter\n  (slug ${sexpString(meta.slug)})\n  (title ${sexpString(meta.title)})\n`;
  out += `  (pillar ${sexpString(meta.pillar)})\n`;
  if (meta.section) out += `  (section ${sexpString(meta.section)})\n`;
  if (meta.blurb) out += `  (blurb ${sexpString(meta.blurb)})\n`;
  out += lines.join("\n") + ")\n";
  return out;
}

/// A JSX prop `name={<>…</>}` (prompt/hint) → its inline JSX (fragment inner), or null.
function jsxProp(chunk, name) {
  const m = new RegExp(`${name}=\\{\\s*<>([\\s\\S]*?)</>\\s*\\}`).exec(chunk);
  return m ? m[1] : null;
}

/// Emit a `(runnable …)` from its element chunk (source/expected/expect/id/title/mode/authored-in/wrap +
/// multi-file files). Extracts attrs directly (cooked) — extractExamples is lossy for Runnable's expected.
function runnableBlock(chunk, tsxPath) {
  let s = `  (runnable`;
  if (/files=\{\[/.test(chunk)) {
    const files = extractFilesProp(chunk, tsxPath);
    s += `\n    (files`;
    for (const f of files) {
      s += `\n      (file (name ${sexpString(f.name)}) (source ${sexpString(f.source)}) (surface ${sexpString(f.surface)})`;
      s += f.entry ? ` (entry "true"))` : `)`;
    }
    s += `)`;
  } else {
    s += `\n    (source ${sexpString(attr(chunk, "source") ?? "")})`;
  }
  for (const [a, k] of [["expected", "expected"], ["expect", "expect"], ["id", "id"], ["title", "title"], ["mode", "mode"], ["authoredIn", "authored-in"]]) {
    const v = attr(chunk, a);
    if (v != null) s += `\n    (${k} ${sexpString(v)})`;
  }
  if (/wrap=\{false\}/.test(chunk)) s += `\n    (wrap "false")`;
  return s + `)`;
}

/// Emit an `(exercise …)` from its element chunk (id + prompt/hint JSX + starter/solution/expected attrs).
function exerciseBlock(chunk) {
  let s = `  (exercise`;
  const id = attr(chunk, "id");
  if (id) s += `\n    (id ${sexpString(id)})`;
  const prompt = jsxProp(chunk, "prompt");
  if (prompt) s += `\n    (prompt ${parseInline(prompt.trim()).join(" ")})`;
  for (const name of ["starter", "solution", "expected"]) {
    const v = attr(chunk, name);
    if (v != null) s += `\n    (${name} ${sexpString(v)})`;
  }
  const hint = jsxProp(chunk, "hint");
  if (hint) s += `\n    (hint ${parseInline(hint.trim()).join(" ")})`;
  return s + `)`;
}

// ---- per-chapter fidelity verifier: convert → render (xtask) → compare to the old .tsx ----

/// The PROSE visible text of a chapter .tsx: the concatenated visible text of its H1/Lede/P/H2/Note/Why
/// blocks ONLY (Runnable/Exercise elements are removed first — their fidelity is checked via example-extract,
/// and their attrs/code aren't reader prose). JSX-ish: unwrap `{`…`}`/`{" "}`, drop fragment tokens + tags,
/// collapse whitespace, decode &amp;. Both the OLD hand .tsx and the codegen'd .tsx normalize to the same
/// string iff the converter preserved the reader-visible prose.
function inlineVisible(s) {
  return s
    .replace(/\{`([\s\S]*?)`\}/g, (_, b) => cookTemplate(b)) // {`code`} → cooked code (as the browser renders)
    .replace(/\{" "\}/g, " ") // {" "} → space
    .replace(/\{("(?:[^"\\]|\\.)*")\}/g, (_, j) => { try { return JSON.parse(j); } catch { return j; } }) // {"…"} (escape_text) → decoded text
    .replace(/<\/?>/g, "") // <> and </> fragment tokens
    .replace(/[{}]/g, "") // stray fragment braces
    .replace(/<[^>]+>/g, "") // all tags
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
}

/// The reader-visible PROSE text of a chapter .tsx: POSITIVELY extract each prose block (H1/Lede/P/H2/Note/
/// Why) + each Exercise prompt/hint fragment, in document order, and concatenate their visible text. Positive
/// extraction (vs removing Runnable/Exercise) avoids the self-closing-`/>` / inner-`<br />` removal hazard and
/// still covers the exercise teaching prose (prompt/hint). Runnable/Exercise CODE is checked via example-extract.
function proseText(tsx) {
  const a = /<article>([\s\S]*)<\/article>/.exec(tsx)?.[1] ?? tsx;
  const parts = [];
  for (const tag of ["H1", "Lede", "P", "H2", "Note"]) {
    for (const m of a.matchAll(new RegExp(`<${tag}>([\\s\\S]*?)</${tag}>`, "g"))) parts.push([m.index, inlineVisible(m[1])]);
  }
  for (const m of a.matchAll(/<Why[^>]*>([\s\S]*?)<\/Why>/g)) parts.push([m.index, inlineVisible(m[1])]);
  for (const m of a.matchAll(/(?:prompt|hint)=\{\s*<>([\s\S]*?)<\/>\s*\}/g)) parts.push([m.index, inlineVisible(m[1])]);
  return parts.sort((x, y) => x[0] - y[0]).map((p) => p[1]).join(" ");
}

/// Extract example (source/expected) fidelity fields for a chapter body.
function exampleFields(tsx, label, extractExamples) {
  const body = /<article>([\s\S]*)<\/article>/.exec(tsx)?.[1] ?? tsx;
  return JSON.stringify(extractExamples(body, label).map((e) => ({ k: e.kind, s: e.snippet, x: e.expected })));
}

async function verifyChapter(tsxPath) {
  const { extractExamples } = await import("./example-extract.mjs");
  const oldTsx = readFileSync(tsxPath, "utf8");
  const sexp = convertChapter(tsxPath);
  // Render the .sexp back to TSX via the built emit-xtask binary directly (no cargo-shim; #5606 gotcha).
  const bin = join(HERE, "../../target/debug/xtask-codegen-guide");
  const tmp = join("/tmp", basename(tsxPath, ".tsx") + ".verify.sexp");
  writeFileSync(tmp, sexp);
  const genTsx = execFileSync(bin, [tmp], { encoding: "utf8" });

  const proseOk = proseText(oldTsx) === proseText(genTsx);
  const oldEx = exampleFields(oldTsx, tsxPath, extractExamples);
  const genEx = exampleFields(genTsx, tsxPath, extractExamples);
  const exOk = oldEx === genEx;
  console.log(`${proseOk ? "✓" : "✗"} prose visible text  (${basename(tsxPath)})`);
  console.log(`${exOk ? "✓" : "✗"} example source/expected fidelity`);
  if (!proseOk) {
    const o = proseText(oldTsx), g = proseText(genTsx);
    for (let i = 0; i < Math.max(o.length, g.length); i++)
      if (o[i] !== g[i]) { console.log(`  prose diff @${i}\n   old:…${JSON.stringify(o.slice(i - 40, i + 40))}\n   gen:…${JSON.stringify(g.slice(i - 40, i + 40))}`); break; }
  }
  if (!exOk) console.log(`  old ex: ${oldEx}\n  gen ex: ${genEx}`);
  process.exit(proseOk && exOk ? 0 : 1);
}

// ---- CLI: convert a chapter .tsx → .sexp on stdout; --verify round-trips + compares ----
const cliArg = process.argv.find((x) => x.endsWith(".tsx"));
if (process.argv.includes("--verify") && cliArg) {
  await verifyChapter(cliArg);
} else if (cliArg) {
  process.stdout.write(convertChapter(cliArg));
  process.exit(0);
}

// ---- self-test ----
if (process.argv.includes("--self-test")) {
  // Spaces around inline marks are PRESERVED (the visible-text collapse keeps a single space) — matches the
  // pilot .sexp, e.g. "…Cadenza " (em "the language") ". This…".
  const cases = [
    ["plain text", `Most languages are shaped by history.`, `"Most languages are shaped by history."`],
    ["em", `A <em>test</em> here`, `"A " (em "test") " here"`],
    ["inline code", `Use <C>const</C> now`, `"Use " (c "const") " now"`],
    ["jsx space", `the{" "}platform`, `"the platform"`],
    ["chapter link", `see <Ch to="/lists">lists</Ch> ok`, `"see " (link (slug "lists") "lists") " ok"`],
    ["app link", `open <Link to="/playground">it</Link>`, `"open " (app-link (route "/playground") "it")`],
    ["br", `line<br />next`, `"line" (br) "next"`],
    ["strong", `a <strong>bold</strong> word`, `"a " (strong "bold") " word"`],
    ["C with template", `run <C>{\`(+ 1 2)\`}</C> here`, `"run " (c "(+ 1 2)") " here"`],
    ["nested em+code", `<P>see <em>the <C>x</C></em>!</P>`.replace(/^<P>|<\/P>$/g, ""), `"see " (em "the " (c "x")) "!"`],
  ];
  let fail = 0;
  for (const [name, input, want] of cases) {
    const got = parseInline(input).join(" ");
    const ok = got === want;
    if (!ok) fail++;
    console.log(`${ok ? "✓" : "✗"} ${name}`);
    if (!ok) console.log(`   want: ${want}\n   got:  ${got}`);
  }
  console.log(fail === 0 ? `\n✓ all ${cases.length} inline cases pass` : `\n✗ ${fail}/${cases.length} failed`);
  process.exit(fail === 0 ? 0 : 1);
}

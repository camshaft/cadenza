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

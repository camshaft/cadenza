// Shared example EXTRACTION — the pure text-scan that pulls every `<Runnable>`/`<Exercise>` out of a
// chapter TSX (source/solution/expected/expect/files/mode/authoredIn), with backtick-template cooking.
// Imported by BOTH scripts/check-examples.mjs (the inline gate) AND scripts/shred-examples.mjs (the
// per-example dir dumper for the nix-cached matrix) so the two can NEVER drift in how they extract — the
// same anti-drift invariant that keeps wrapModule/lowerToCompile imported from the guide source. Pure
// (no wasm, no fs) — cookTemplate ⊂ extractFilesProp ⊂ extractExamples; unit-pinned by check-examples'
// cookTemplate + multi-file self-checks.

// Interpret the escapes in a captured template-literal body EXACTLY as JS would when evaluating the `…`
// at runtime — that cooked value is what the live <Runnable> `source` prop receives, so the gate/shred
// must compile the SAME string the browser does (e.g. authored `#\\a` cooks to the `#\a` char literal).
// Only backslash sequences are transformed; backslash-free source passes through byte-identical.
export function cookTemplate(raw) {
  let out = "";
  for (let i = 0; i < raw.length; i++) {
    const c = raw[i];
    if (c !== "\\") {
      out += c;
      continue;
    }
    const n = raw[i + 1];
    if (n === undefined) {
      out += "\\";
      break;
    }
    i++;
    switch (n) {
      case "n": out += "\n"; break;
      case "r": out += "\r"; break;
      case "t": out += "\t"; break;
      case "b": out += "\b"; break;
      case "f": out += "\f"; break;
      case "v": out += "\v"; break;
      case "0": out += "\0"; break;
      case "\n": break; // line continuation: backslash-newline cooks to nothing
      case "x": {
        const hex = raw.slice(i + 1, i + 3);
        if (/^[0-9a-fA-F]{2}$/.test(hex)) { out += String.fromCharCode(parseInt(hex, 16)); i += 2; } else { out += "x"; }
        break;
      }
      case "u": {
        if (raw[i + 1] === "{") {
          const close = raw.indexOf("}", i + 2);
          const hex = close > 0 ? raw.slice(i + 2, close) : "";
          if (close > 0 && /^[0-9a-fA-F]+$/.test(hex)) { out += String.fromCodePoint(parseInt(hex, 16)); i = close; } else { out += "u"; }
        } else {
          const hex = raw.slice(i + 1, i + 5);
          if (/^[0-9a-fA-F]{4}$/.test(hex)) { out += String.fromCharCode(parseInt(hex, 16)); i += 4; } else { out += "u"; }
        }
        break;
      }
      default: out += n; // \\ → \, \` → `, \$ → $, and any other \C → C (backslash dropped), as JS does
    }
  }
  return out;
}

// Extract the `files={[{name, source: `…`, surface, entry}]}` entries of a multi-file <Runnable> (locked
// field order name → source → surface → entry). A `files={[` marker MUST yield ≥2 well-formed entries with
// exactly one `entry: true`, else THROW (a silent 0-extract on a marked runnable is a coverage hole).
export function extractFilesProp(attrs, file) {
  const entryRe =
    /\{\s*name:\s*"([^"]+)"\s*,\s*source:\s*`([\s\S]*?)`\s*(?:,\s*surface:\s*"(ml|sexpr)")?\s*(?:,\s*entry:\s*(true|false))?\s*,?\s*\}/g;
  const files = [];
  let m;
  while ((m = entryRe.exec(attrs))) {
    files.push({ name: m[1], source: cookTemplate(m[2]), surface: m[3] ?? "sexpr", entry: m[4] === "true" });
  }
  if (files.length < 2) {
    throw new Error(
      `${file}: a <Runnable files={[…]}> extracted ${files.length} file(s) — a multi-file example needs ≥2 ` +
        `(entry + ≥1 preloaded). Check the files= entries follow the {name, source: \`…\`, surface, entry} shape ` +
        `in that field order (the extractor + codegen pin that order).`,
    );
  }
  const entries = files.filter((f) => f.entry);
  if (entries.length !== 1) {
    throw new Error(`${file}: a <Runnable files={[…]}> must mark exactly one file \`entry: true\` (found ${entries.length}).`);
  }
  return files;
}

// Extract every `<Runnable>`/`<Exercise>` from a chapter's TSX. Chunk by element-open position (elements are
// self-closing + never nest), so each element's attrs run to the next open tag — robust to a `prompt={<>…</>}`
// fragment that a non-greedy regex would truncate at. Returns example descriptors (kind/snippet/surface/
// expect/expected/noWrap/isTest/prelude/authoredIn, or `files` for multi-file).
export function extractExamples(tsx, file) {
  const out = [];
  const openRe = /<(Runnable|Exercise)\b/g;
  const opens = [];
  let om;
  while ((om = openRe.exec(tsx))) opens.push({ kind: om[1], start: om.index });
  for (let i = 0; i < opens.length; i++) {
    const { kind, start } = opens[i];
    const end = i + 1 < opens.length ? opens[i + 1].start : tsx.length;
    const attrs = tsx.slice(start, end);
    const grab = (name) => {
      const tl = new RegExp(`${name}=\\{\`([\\s\\S]*?)\`\\}`).exec(attrs);
      if (tl) return cookTemplate(tl[1]);
      const s = new RegExp(`${name}="([^"]*)"`).exec(attrs);
      return s ? s[1] : null;
    };
    const expect = grab("expect") ?? "value";
    const expected = grab("expected");
    // Skip a `wrap={false}` example (a full module the author wrote) — still compiled, just not wrapped.
    const noWrap = /wrap=\{false\}/.test(attrs);
    if (kind === "Runnable") {
      // MULTI-FILE Runnable: `files={[…]}` — a file SET compiled together via compile_with_preloaded.
      if (/files=\{\[/.test(attrs)) {
        const mfFiles = extractFilesProp(attrs, file);
        out.push({ file, kind, files: mfFiles, expect, expected, snippet: mfFiles.map((f) => f.source).join("\n") });
        continue;
      }
      const source = grab("source");
      // A `mode="test"` Runnable runs its @test defs as tests; default every @test PASSES, `expect="error"`
      // = at least one @test is meant to FAIL.
      const isTest = /mode="test"/.test(attrs) || /mode=\{"test"\}/.test(attrs);
      // Default the shared assert prelude ON (matches <Runnable> prelude default); `prelude={false}` opts out.
      const prelude = isTest && !/prelude=\{false\}/.test(attrs);
      // `authoredIn` (default s-expr) is the surface the `source` is WRITTEN in.
      const authoredIn = grab("authoredIn") ?? "sexpr";
      if (source != null) out.push({ file, kind, snippet: source, expect, expected: null, noWrap, isTest, prelude, authoredIn });
    } else {
      // Exercise: check the SOLUTION (the starter has a `?` hole by design).
      const solution = grab("solution");
      if (solution != null) out.push({ file, kind, snippet: solution, expect: "value", expected, noWrap });
    }
  }
  return out;
}

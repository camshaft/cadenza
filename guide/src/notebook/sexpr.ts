/// A tiny generic s-expression reader for the notebook's output renderers (table, chart, formula).
///
/// The run worker renders a cell's value as a canonical s-expr `(: <value> <type>)` — we parse the
/// MACHINE surface, never the display surface (an ML render uses commas + backtick-rationals a string
/// parser chokes on; this exact bug broke /cad — noted by v-guide-infra). This module is the shared
/// parse primitive: tokenize + build a nested node tree; the renderers walk it into rows / points.
///
/// PURE (no worker/React) so it's unit-testable under `node --test`. A twin of cad/index.ts's inline
/// tokenizer, extracted for reuse across the notebook's renderers (table + chart both need it).

/// A parsed s-expr node: an atom (a bare token — number, symbol, or a "quoted string") or a list.
export type Node = { atom: string } | { list: Node[] };

export function isAtom(n: Node): n is { atom: string } {
  return "atom" in n;
}
export function isList(n: Node): n is { list: Node[] } {
  return "list" in n;
}

type Tok = "(" | ")" | { atom: string };

/// Tokenize, treating a `"..."` run as one atom (so a string containing spaces / parens stays intact).
/// Backslash escapes inside a string are preserved verbatim in the atom text (the caller unquotes).
function tokenize(text: string): Tok[] {
  const toks: Tok[] = [];
  let cur = "";
  let inStr = false;
  const flush = () => {
    if (cur) {
      toks.push({ atom: cur });
      cur = "";
    }
  };
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (inStr) {
      cur += c;
      if (c === '"' && text[i - 1] !== "\\") {
        inStr = false;
        flush();
      }
      continue;
    }
    if (c === '"') {
      flush();
      inStr = true;
      cur = '"';
      continue;
    }
    if (c === "(" || c === ")") {
      flush();
      toks.push(c);
    } else if (/\s/.test(c)) {
      flush();
    } else {
      cur += c;
    }
  }
  flush();
  return toks;
}

/// Parse an s-expr string into a single Node. Throws on a malformed/empty/multi-root input (the caller
/// treats a throw as "not the shape I expected" and renders a fallback). Bounded recursion via an
/// explicit stack — a deeply nested value can't overflow.
export function parseSexpr(text: string): Node {
  const toks = tokenize(text.trim());
  if (toks.length === 0) throw new SyntaxError("empty s-expr");
  let pos = 0;
  const parseNode = (): Node => {
    const t = toks[pos];
    if (t === undefined) throw new SyntaxError("unexpected end of s-expr");
    if (t === ")") throw new SyntaxError("unexpected `)`");
    if (t === "(") {
      pos++; // consume (
      const list: Node[] = [];
      while (toks[pos] !== ")") {
        if (pos >= toks.length) throw new SyntaxError("unclosed `(`");
        list.push(parseNode());
      }
      pos++; // consume )
      return { list };
    }
    pos++;
    return { atom: (t as { atom: string }).atom };
  };
  const node = parseNode();
  if (pos !== toks.length) throw new SyntaxError("trailing tokens after s-expr");
  return node;
}

/// Unquote an atom that is a `"..."` string literal → its content (backslash escapes resolved). A
/// non-string atom is returned as-is. Used by the renderers to show a String cell without its quotes.
export function unquoteAtom(atom: string): string {
  if (atom.length >= 2 && atom.startsWith('"') && atom.endsWith('"')) {
    return atom.slice(1, -1).replace(/\\"/g, '"').replace(/\\\\/g, "\\");
  }
  return atom;
}

/// Strip the outer `(: <value> <type>)` type-ascription wrapper the run worker adds, returning the
/// inner VALUE node. A value that isn't wrapped (already bare) is returned unchanged.
export function stripAscription(node: Node): Node {
  if (isList(node) && node.list.length === 3 && isAtom(node.list[0]) && node.list[0].atom === ":") {
    return node.list[1];
  }
  return node;
}

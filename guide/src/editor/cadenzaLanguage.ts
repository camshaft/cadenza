/// A lightweight StreamLanguage tokenizer for Cadenza, covering both surfaces well enough for a
/// guide (keywords, comments, strings, numbers, symbols, constructors). Not a full grammar — the
/// binary AST is the source of truth; this is display polish. Graduate to a Lezer grammar later if
/// folding/indentation/semantic coloring is wanted.

import { StreamLanguage, type StringStream } from "@codemirror/language";

// ML-surface keywords + s-expression head forms both syntaxes share.
const KEYWORDS = new Set([
  // ML surface
  "fn", "def", "let", "in", "if", "then", "else", "match", "with", "module",
  "export", "effect", "handle", "host", "op", "type", "do", "guard",
  // s-expression heads that read as keywords
  "lambda", "record", "tuple", "list", "map", "set", "quote", "quasiquote",
]);

const BOOL_UNIT = new Set(["true", "false", "unit"]);

interface State {
  inString: boolean;
}

export const cadenzaLanguage = StreamLanguage.define<State>({
  name: "cadenza",
  startState: () => ({ inString: false }),

  token(stream: StringStream, state: State): string | null {
    // Continue a multi-line-safe string (Cadenza strings are single-line, but be defensive).
    if (state.inString) {
      if (skipString(stream)) state.inString = false;
      return "string";
    }

    if (stream.eatSpace()) return null;

    // Line comments: `//` (ML) and `;` (s-expression).
    if (stream.match("//") || stream.match(";")) {
      stream.skipToEnd();
      return "comment";
    }

    // Doc comments `///` already matched by the `//` above → still a comment; fine.

    // Strings.
    if (stream.peek() === '"') {
      stream.next();
      if (!skipString(stream)) state.inString = true;
      return "string";
    }

    // Char / symbol literals: #\a  #"sym"
    if (stream.peek() === "#") {
      stream.next();
      if (stream.peek() === '"') {
        stream.next();
        skipString(stream);
        return "atom"; // symbol literal
      }
      if (stream.peek() === "\\") {
        stream.next();
        stream.next();
        return "atom"; // char literal
      }
      return "operator";
    }

    // Numbers (int, float, radix, digit separators).
    if (/\d/.test(stream.peek() ?? "")) {
      stream.match(/^0[xXbBoO][0-9a-fA-F_]+/) ||
        stream.match(/^\d[\d_]*(\.[\d_]+)?([eE][+-]?\d+)?/);
      return "number";
    }

    // Punctuation that reads as an operator. Multi-char operators (`|>`, `->`, `=>`, `::`) come first
    // so the greedy alternation matches them before their single-char prefixes.
    if (stream.match(/^(\|>|->|=>|::|[-+*/<>=(){}\[\].,:|])/)) {
      return "operator";
    }

    // Identifiers / heads. A `Capitalized` head is a constructor/type; a dotted `A.b` splits.
    const word = stream.match(/^[A-Za-z_][A-Za-z0-9_-]*/);
    if (word) {
      const w = String(word);
      if (KEYWORDS.has(w)) return "keyword";
      if (BOOL_UNIT.has(w)) return "atom";
      if (/^[A-Z]/.test(w)) return "typeName"; // constructor / type
      return "variableName";
    }

    stream.next();
    return null;
  },
});

/// Consume until an unescaped closing quote on this line. Returns true if the string closed.
function skipString(stream: StringStream): boolean {
  let escaped = false;
  let ch: string | void;
  while ((ch = stream.next()) != null) {
    if (ch === '"' && !escaped) return true;
    escaped = ch === "\\" && !escaped;
  }
  return false;
}

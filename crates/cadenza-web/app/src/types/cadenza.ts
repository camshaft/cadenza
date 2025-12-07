// TypeScript types for cadenza-web WASM module outputs

export interface Token {
  kind: string;
  start: number;
  end: number;
  text: string;
}

export interface LexResult {
  tokens: Token[];
  success: boolean;
}

export interface CstNode {
  kind: string;
  start: number;
  end: number;
  text: string | null;
  children: CstNode[];
}

export interface ParseError {
  start: number;
  end: number;
  message: string;
}

export interface ParseResult {
  tree: CstNode;
  errors: ParseError[];
  success: boolean;
}

export interface AstNode {
  type: string;
  start: number;
  end: number;
  value: string | null;
  children: AstNode[];
}

export interface AstResult {
  nodes: AstNode[];
  errors: ParseError[];
  success: boolean;
}

export interface EvalValue {
  type: string;
  display: string;
}

export interface EvalDiagnostic {
  level: string;
  message: string;
  start: number | null;
  end: number | null;
}

export interface EvalResult {
  values: EvalValue[];
  diagnostics: EvalDiagnostic[];
  success: boolean;
}

export interface Example {
  id: string;
  name: string;
  source: string;
}

// LSP types
export interface LspDiagnostic {
  start_line: number;
  start_character: number;
  end_line: number;
  end_character: number;
  message: string;
  severity: 'error' | 'warning' | 'info' | 'hint';
}

export interface LspHoverInfo {
  content: string;
  found: boolean;
}

export interface LspCompletionItem {
  label: string;
  kind: string;
  detail: string | null;
}

// WASM module interface (will be loaded dynamically)
export interface CadenzaWasm {
  lex: (source: string) => LexResult;
  parse_source: (source: string) => ParseResult;
  ast: (source: string) => AstResult;
  eval_source: (source: string) => EvalResult;
  get_token_kinds: () => string[];
  lsp_diagnostics: (source: string) => LspDiagnostic[];
  lsp_hover: (source: string, line: number, character: number) => LspHoverInfo;
  lsp_completions: (source: string, line: number, character: number) => LspCompletionItem[];
}

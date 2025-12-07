// WASM bindings for cadenza-web
// This module loads the actual WASM module built by wasm-pack

import type { LexResult, ParseResult, AstResult, EvalResult, CadenzaWasm, LspDiagnostic, LspHoverInfo, LspCompletionItem, Syntax, SyntaxInfo } from '../types/cadenza';

// The WASM module will be loaded from the pkg directory
let wasmModule: typeof import('../../pkg/cadenza_web') | null = null;

// Load the WASM module
export async function loadWasm(): Promise<CadenzaWasm> {
  if (wasmModule) {
    return createWasmBindings(wasmModule);
  }

  try {
    // Dynamic import of the WASM module
    const module = await import('../../pkg/cadenza_web');
    await module.default();
    wasmModule = module;
    return createWasmBindings(module);
  } catch (error) {
    console.error('Failed to load WASM module:', error);
    console.warn('Falling back to mock implementations');
    return mockWasm;
  }
}

// Create bindings from the loaded WASM module
function createWasmBindings(module: typeof import('../../pkg/cadenza_web')): CadenzaWasm {
  return {
    lex: (source: string): LexResult => {
      return module.lex(source) as LexResult;
    },
    parse_source: (source: string, syntax: Syntax): ParseResult => {
      return module.parse_source(source, syntax) as ParseResult;
    },
    ast: (source: string, syntax: Syntax): AstResult => {
      return module.ast(source, syntax) as AstResult;
    },
    eval_source: (source: string, syntax: Syntax): EvalResult => {
      return module.eval_source(source, syntax) as EvalResult;
    },
    get_token_kinds: (): string[] => {
      return module.get_token_kinds() as string[];
    },
    get_syntaxes: (): SyntaxInfo[] => {
      return module.get_syntaxes() as SyntaxInfo[];
    },
    lsp_diagnostics: (source: string): LspDiagnostic[] => {
      return module.lsp_diagnostics(source) as LspDiagnostic[];
    },
    lsp_hover: (source: string, line: number, character: number): LspHoverInfo => {
      return module.lsp_hover(source, line, character) as LspHoverInfo;
    },
    lsp_completions: (source: string, line: number, character: number): LspCompletionItem[] => {
      return module.lsp_completions(source, line, character) as LspCompletionItem[];
    },
  };
}

// Fallback mock implementations (used when WASM module is not available)
function mockLex(source: string): LexResult {
  const tokens: LexResult['tokens'] = [];
  let pos = 0;
  
  while (pos < source.length) {
    const char = source[pos];
    
    if (/\s/.test(char)) {
      const start = pos;
      while (pos < source.length && /\s/.test(source[pos])) {
        pos++;
      }
      tokens.push({
        kind: source[start] === '\n' ? 'Newline' : 'Space',
        start,
        end: pos,
        text: source.slice(start, pos),
      });
      continue;
    }
    
    if (/\d/.test(char)) {
      const start = pos;
      while (pos < source.length && /[\d._]/.test(source[pos])) {
        pos++;
      }
      const text = source.slice(start, pos);
      tokens.push({
        kind: text.includes('.') ? 'Float' : 'Integer',
        start,
        end: pos,
        text,
      });
      continue;
    }
    
    if (/[a-zA-Z_]/.test(char)) {
      const start = pos;
      while (pos < source.length && /\w/.test(source[pos])) {
        pos++;
      }
      tokens.push({
        kind: 'Identifier',
        start,
        end: pos,
        text: source.slice(start, pos),
      });
      continue;
    }
    
    const operators: Record<string, string> = {
      '+': 'Plus', '-': 'Minus', '*': 'Star', '/': 'Slash',
      '=': 'Equal', '<': 'Less', '>': 'Greater',
      '(': 'LParen', ')': 'RParen', '[': 'LBracket', ']': 'RBracket',
      '{': 'LBrace', '}': 'RBrace', ',': 'Comma', '.': 'Dot',
      ':': 'Colon', ';': 'Semicolon',
    };
    
    if (operators[char]) {
      tokens.push({ kind: operators[char], start: pos, end: pos + 1, text: char });
      pos++;
      continue;
    }
    
    tokens.push({ kind: 'Unknown', start: pos, end: pos + 1, text: char });
    pos++;
  }
  
  return { tokens, success: true };
}

function mockParse(source: string, _syntax: Syntax): ParseResult {
  const lexResult = mockLex(source);
  return {
    tree: {
      kind: 'Root',
      start: 0,
      end: source.length,
      text: null,
      children: lexResult.tokens.map((tok) => ({
        kind: tok.kind, start: tok.start, end: tok.end, text: tok.text, children: [],
      })),
    },
    errors: [],
    success: true,
  };
}

function mockAst(source: string, syntax: Syntax): AstResult {
  const parseResult = mockParse(source, syntax);
  return {
    nodes: parseResult.tree.children
      .filter((c) => !['Space', 'Newline'].includes(c.kind))
      .map((c) => ({
        type: c.kind === 'Integer' || c.kind === 'Float' ? 'Literal' :
              c.kind === 'Identifier' ? 'Ident' : c.kind,
        start: c.start, end: c.end, value: c.text, children: [],
      })),
    errors: [],
    success: true,
  };
}

function mockEval(source: string, _syntax: Syntax): EvalResult {
  const values: EvalResult['values'] = [];
  const diagnostics: EvalResult['diagnostics'] = [];
  
  for (const line of source.split('\n').filter((l) => l.trim())) {
    const trimmed = line.trim();
    const num = parseFloat(trimmed.replace(/_/g, ''));
    if (!isNaN(num)) {
      values.push({ type: trimmed.includes('.') ? 'float' : 'integer', display: String(num) });
      continue;
    }
    diagnostics.push({ level: 'error', message: `Cannot evaluate: ${trimmed}`, start: null, end: null });
    values.push({ type: 'nil', display: 'nil' });
  }
  
  return { values, diagnostics, success: diagnostics.every((d) => d.level !== 'error') };
}

export const mockWasm: CadenzaWasm = {
  lex: mockLex,
  parse_source: mockParse,
  ast: mockAst,
  eval_source: mockEval,
  get_token_kinds: () => [
    'Identifier', 'Integer', 'Float', 'StringStart', 'StringContent', 'StringEnd',
    'Plus', 'Minus', 'Star', 'Slash', 'Equal', 'Less', 'Greater',
    'LParen', 'RParen', 'LBracket', 'RBracket', 'LBrace', 'RBrace',
    'Comma', 'Dot', 'Colon', 'Semicolon', 'Space', 'Newline',
  ],
  get_syntaxes: () => [
    { id: 'cadenza', name: 'Cadenza' },
    { id: 'markdown', name: 'Markdown' },
    { id: 'sql', name: 'SQL' },
    { id: 'gcode', name: 'GCode' },
  ],
  lsp_diagnostics: (_source: string): LspDiagnostic[] => {
    // Mock: return empty diagnostics
    return [];
  },
  lsp_hover: (_source: string, _line: number, _character: number): LspHoverInfo => {
    // Mock: return no hover info
    return { content: '', found: false };
  },
  lsp_completions: (_source: string, _line: number, _character: number): LspCompletionItem[] => {
    // Mock: return basic completions
    return [
      { label: 'let', kind: 'keyword', detail: 'Variable binding' },
      { label: 'fn', kind: 'keyword', detail: 'Function definition' },
    ];
  },
};

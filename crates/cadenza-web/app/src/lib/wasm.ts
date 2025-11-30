// Mock implementations for cadenza-web WASM functions
// These will be replaced with actual WASM calls once the module is built

import type { LexResult, ParseResult, AstResult, EvalResult, CadenzaWasm } from '../types/cadenza';

// Simple tokenizer mock
function mockLex(source: string): LexResult {
  const tokens: LexResult['tokens'] = [];
  let pos = 0;
  
  while (pos < source.length) {
    const char = source[pos];
    
    // Skip whitespace but record it
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
    
    // Numbers
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
    
    // Identifiers
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
    
    // Operators
    const operators: Record<string, string> = {
      '+': 'Plus',
      '-': 'Minus',
      '*': 'Star',
      '/': 'Slash',
      '=': 'Equal',
      '<': 'Less',
      '>': 'Greater',
      '(': 'LParen',
      ')': 'RParen',
      '[': 'LBracket',
      ']': 'RBracket',
      '{': 'LBrace',
      '}': 'RBrace',
      ',': 'Comma',
      '.': 'Dot',
      ':': 'Colon',
      ';': 'Semicolon',
    };
    
    if (operators[char]) {
      tokens.push({
        kind: operators[char],
        start: pos,
        end: pos + 1,
        text: char,
      });
      pos++;
      continue;
    }
    
    // String literals
    if (char === '"') {
      const start = pos;
      pos++; // skip opening quote
      tokens.push({
        kind: 'StringStart',
        start,
        end: start + 1,
        text: '"',
      });
      
      const contentStart = pos;
      while (pos < source.length && source[pos] !== '"') {
        pos++;
      }
      tokens.push({
        kind: 'StringContent',
        start: contentStart,
        end: pos,
        text: source.slice(contentStart, pos),
      });
      
      if (pos < source.length) {
        tokens.push({
          kind: 'StringEnd',
          start: pos,
          end: pos + 1,
          text: '"',
        });
        pos++;
      }
      continue;
    }
    
    // Unknown character
    tokens.push({
      kind: 'Unknown',
      start: pos,
      end: pos + 1,
      text: char,
    });
    pos++;
  }
  
  return { tokens, success: true };
}

// Simple parser mock
function mockParse(source: string): ParseResult {
  const lexResult = mockLex(source);
  
  // Build a simple CST
  const tree: ParseResult['tree'] = {
    kind: 'Root',
    start: 0,
    end: source.length,
    text: null,
    children: lexResult.tokens.map((tok) => ({
      kind: tok.kind,
      start: tok.start,
      end: tok.end,
      text: tok.text,
      children: [],
    })),
  };
  
  return { tree, errors: [], success: true };
}

// Simple AST mock
function mockAst(source: string): AstResult {
  const parseResult = mockParse(source);
  
  // Convert tokens to simple AST nodes, filtering whitespace
  const nodes: AstResult['nodes'] = parseResult.tree.children
    .filter((child) => !['Space', 'Newline'].includes(child.kind))
    .map((child) => ({
      type: child.kind === 'Integer' || child.kind === 'Float' ? 'Literal' : 
            child.kind === 'Identifier' ? 'Ident' : child.kind,
      start: child.start,
      end: child.end,
      value: child.text,
      children: [],
    }));
  
  return { nodes, errors: [], success: true };
}

// Simple eval mock
function mockEval(source: string): EvalResult {
  const values: EvalResult['values'] = [];
  const diagnostics: EvalResult['diagnostics'] = [];
  
  // Try to evaluate simple expressions
  const lines = source.split('\n').filter((line) => line.trim());
  
  for (const line of lines) {
    const trimmed = line.trim();
    
    // Try to evaluate as a number
    const num = parseFloat(trimmed.replace(/_/g, ''));
    if (!isNaN(num)) {
      values.push({
        type: trimmed.includes('.') ? 'float' : 'integer',
        display: String(num),
      });
      continue;
    }
    
    // Try to evaluate simple arithmetic
    const match = trimmed.match(/^(\d+)\s*([+\-*/])\s*(\d+)$/);
    if (match) {
      const [, a, op, b] = match;
      const na = parseFloat(a);
      const nb = parseFloat(b);
      let result: number;
      switch (op) {
        case '+': result = na + nb; break;
        case '-': result = na - nb; break;
        case '*': result = na * nb; break;
        case '/': result = na / nb; break;
        default: result = NaN;
      }
      values.push({
        type: 'integer',
        display: String(result),
      });
      continue;
    }
    
    // Unknown expression
    diagnostics.push({
      level: 'error',
      message: `Cannot evaluate: ${trimmed}`,
      start: null,
      end: null,
    });
    values.push({
      type: 'nil',
      display: 'nil',
    });
  }
  
  return {
    values,
    diagnostics,
    success: diagnostics.filter((d) => d.level === 'error').length === 0,
  };
}

// Mock WASM module
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
};

// Will be replaced with actual WASM module loading
export async function loadWasm(): Promise<CadenzaWasm> {
  // For now, return mock implementations
  // TODO: Load actual WASM module when built with wasm-pack
  return mockWasm;
}

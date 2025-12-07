// Monaco Editor language configuration and LSP integration for Cadenza

import * as monaco from 'monaco-editor';
import type { CadenzaWasm, LspDiagnostic } from '../types/cadenza';

// Language configuration for Cadenza
export const CADENZA_LANGUAGE_ID = 'cadenza';

export function registerCadenzaLanguage() {
  // Register the language
  monaco.languages.register({ id: CADENZA_LANGUAGE_ID });

  // Set up language configuration
  monaco.languages.setLanguageConfiguration(CADENZA_LANGUAGE_ID, {
    comments: {
      lineComment: '//',
      blockComment: ['/*', '*/'],
    },
    brackets: [
      ['{', '}'],
      ['[', ']'],
      ['(', ')'],
    ],
    autoClosingPairs: [
      { open: '{', close: '}' },
      { open: '[', close: ']' },
      { open: '(', close: ')' },
      { open: '"', close: '"' },
      { open: "'", close: "'" },
    ],
    surroundingPairs: [
      { open: '{', close: '}' },
      { open: '[', close: ']' },
      { open: '(', close: ')' },
      { open: '"', close: '"' },
      { open: "'", close: "'" },
    ],
  });

  // Set up syntax highlighting
  monaco.languages.setMonarchTokensProvider(CADENZA_LANGUAGE_ID, {
    keywords: [
      'let', 'fn', 'if', 'else', 'match', 'for', 'while', 'return',
      'true', 'false', 'nil',
    ],
    operators: [
      '=', '>', '<', '!', '~', '?', ':',
      '==', '<=', '>=', '!=', '&&', '||',
      '+', '-', '*', '/', '&', '|', '^', '%',
      '<<', '>>', '>>>', '+=', '-=', '*=', '/=',
    ],
    symbols: /[=><!~?:&|+\-*\/\^%]+/,
    tokenizer: {
      root: [
        // Identifiers and keywords
        [/[a-zA-Z_]\w*/, {
          cases: {
            '@keywords': 'keyword',
            '@default': 'identifier'
          }
        }],
        
        // Whitespace
        { include: '@whitespace' },
        
        // Numbers
        [/\d+\.\d+([eE][\-+]?\d+)?/, 'number.float'],
        [/\d+/, 'number'],
        
        // Strings
        [/"/, 'string', '@string'],
        
        // Delimiters and operators
        [/[{}()\[\]]/, '@brackets'],
        [/@symbols/, {
          cases: {
            '@operators': 'operator',
            '@default': ''
          }
        }],
      ],
      
      whitespace: [
        [/[ \t\r\n]+/, ''],
        [/\/\/.*$/, 'comment'],
        [/\/\*/, 'comment', '@comment'],
      ],
      
      comment: [
        [/[^\/*]+/, 'comment'],
        [/\*\//, 'comment', '@pop'],
        [/[\/*]/, 'comment']
      ],
      
      string: [
        [/[^\\"]+/, 'string'],
        [/\\./, 'string.escape'],
        [/"/, 'string', '@pop']
      ],
    },
  });
}

// LSP integration for diagnostics
export function setupDiagnostics(
  wasm: CadenzaWasm,
  model: monaco.editor.ITextModel
): () => void {
  let decorations: string[] = [];
  let markers: monaco.editor.IMarkerData[] = [];
  
  const updateDiagnostics = () => {
    const source = model.getValue();
    const diagnostics = wasm.lsp_diagnostics(source);
    
    // Convert to Monaco markers
    markers = diagnostics.map((diag: LspDiagnostic) => ({
      severity: diagSeverityToMonaco(diag.severity),
      startLineNumber: diag.start_line + 1,  // Monaco is 1-based
      startColumn: diag.start_character + 1,
      endLineNumber: diag.end_line + 1,
      endColumn: diag.end_character + 1,
      message: diag.message,
      source: 'cadenza',
    }));
    
    monaco.editor.setModelMarkers(model, 'cadenza', markers);
  };
  
  // Initial update
  updateDiagnostics();
  
  // Update on content change
  const disposable = model.onDidChangeContent(() => {
    updateDiagnostics();
  });
  
  return () => {
    disposable.dispose();
    monaco.editor.setModelMarkers(model, 'cadenza', []);
  };
}

// LSP integration for hover
export function setupHover(wasm: CadenzaWasm): monaco.IDisposable {
  return monaco.languages.registerHoverProvider(CADENZA_LANGUAGE_ID, {
    provideHover: (model, position) => {
      const source = model.getValue();
      const hoverInfo = wasm.lsp_hover(
        source,
        position.lineNumber - 1,  // Monaco is 1-based, LSP is 0-based
        position.column - 1
      );
      
      if (hoverInfo.found) {
        return {
          contents: [{ value: hoverInfo.content }],
        };
      }
      
      return null;
    },
  });
}

// LSP integration for completions
export function setupCompletions(wasm: CadenzaWasm): monaco.IDisposable {
  return monaco.languages.registerCompletionItemProvider(CADENZA_LANGUAGE_ID, {
    triggerCharacters: ['.'],
    provideCompletionItems: (model, position) => {
      const source = model.getValue();
      const completions = wasm.lsp_completions(
        source,
        position.lineNumber - 1,
        position.column - 1
      );
      
      return {
        suggestions: completions.map((item) => ({
          label: item.label,
          kind: completionKindToMonaco(item.kind),
          detail: item.detail || undefined,
          insertText: item.label,
        })),
      };
    },
  });
}

// Helper functions
function diagSeverityToMonaco(severity: string): monaco.MarkerSeverity {
  switch (severity) {
    case 'error':
      return monaco.MarkerSeverity.Error;
    case 'warning':
      return monaco.MarkerSeverity.Warning;
    case 'info':
      return monaco.MarkerSeverity.Info;
    case 'hint':
      return monaco.MarkerSeverity.Hint;
    default:
      return monaco.MarkerSeverity.Error;
  }
}

function completionKindToMonaco(kind: string): monaco.languages.CompletionItemKind {
  switch (kind) {
    case 'keyword':
      return monaco.languages.CompletionItemKind.Keyword;
    case 'function':
      return monaco.languages.CompletionItemKind.Function;
    case 'variable':
      return monaco.languages.CompletionItemKind.Variable;
    case 'class':
      return monaco.languages.CompletionItemKind.Class;
    default:
      return monaco.languages.CompletionItemKind.Text;
  }
}

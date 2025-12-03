import { useState, useEffect, useCallback, useMemo } from 'react'
import { SourceEditor } from './components/SourceEditor'
import { TokensPanel } from './components/TokensPanel'
import { CstPanel } from './components/CstPanel'
import { AstPanel } from './components/AstPanel'
import { EvalPanel } from './components/EvalPanel'
import { loadWasm } from './lib/wasm'
import type { CadenzaWasm, LexResult, ParseResult, AstResult, EvalResult, Example } from './types/cadenza'
import './index.css'

type Tab = 'tokens' | 'cst' | 'ast' | 'eval';

const DEFAULT_SOURCE = `# Welcome to Cadenza Compiler Explorer!
# Try some expressions:

42
3.14159
1 + 2
10 * 5
"hello world"
`;

const STORAGE_KEY = 'cadenza-compiler-explorer-source';
const STORAGE_EXAMPLE_KEY = 'cadenza-compiler-explorer-example';

function App() {
  const [source, setSource] = useState(DEFAULT_SOURCE);
  const [activeTab, setActiveTab] = useState<Tab>('tokens');
  const [wasm, setWasm] = useState<CadenzaWasm | null>(null);
  const [loading, setLoading] = useState(true);
  const [examples, setExamples] = useState<Example[]>([]);
  const [selectedExample, setSelectedExample] = useState<string>('');
  const [isUserEdited, setIsUserEdited] = useState(false);

  // Load WASM module and examples on mount
  useEffect(() => {
    loadWasm().then((module) => {
      setWasm(module);
      
      // Load examples
      let examplesList: Example[] = [];
      try {
        examplesList = module.get_examples();
        setExamples(examplesList);
      } catch (e) {
        console.error('Failed to load examples:', e);
      }
      
      // Try to load saved source from localStorage
      try {
        const savedSource = localStorage.getItem(STORAGE_KEY);
        const savedExample = localStorage.getItem(STORAGE_EXAMPLE_KEY);
        if (savedSource) {
          setSource(savedSource);
          setIsUserEdited(true);
          setSelectedExample('custom');
        } else if (savedExample && examplesList.length > 0) {
          // Load the saved example if available
          const example = examplesList.find((ex: Example) => ex.id === savedExample);
          if (example) {
            setSource(example.source);
            setSelectedExample(savedExample);
          }
        }
      } catch (e) {
        console.error('Failed to load from localStorage:', e);
      }
      
      setLoading(false);
    });
  }, []);

  // Compute results when source changes
  const lexResult = useMemo<LexResult | null>(() => {
    if (!wasm) return null;
    try {
      return wasm.lex(source);
    } catch (e) {
      console.error('Lex error:', e);
      return null;
    }
  }, [wasm, source]);

  const parseResult = useMemo<ParseResult | null>(() => {
    if (!wasm) return null;
    try {
      return wasm.parse_source(source);
    } catch (e) {
      console.error('Parse error:', e);
      return null;
    }
  }, [wasm, source]);

  const astResult = useMemo<AstResult | null>(() => {
    if (!wasm) return null;
    try {
      return wasm.ast(source);
    } catch (e) {
      console.error('AST error:', e);
      return null;
    }
  }, [wasm, source]);

  const evalResult = useMemo<EvalResult | null>(() => {
    if (!wasm) return null;
    try {
      return wasm.eval_source(source);
    } catch (e) {
      console.error('Eval error:', e);
      return null;
    }
  }, [wasm, source]);

  const handleSourceChange = useCallback((value: string) => {
    setSource(value);
    setIsUserEdited(true);
    setSelectedExample('custom');
    
    // Save to localStorage
    try {
      localStorage.setItem(STORAGE_KEY, value);
      localStorage.removeItem(STORAGE_EXAMPLE_KEY);
    } catch (e) {
      console.error('Failed to save to localStorage:', e);
    }
  }, []);

  const handleExampleChange = useCallback((exampleId: string) => {
    if (exampleId === 'custom') {
      // Keep current source
      setSelectedExample('custom');
      return;
    }
    
    const example = examples.find(ex => ex.id === exampleId);
    if (example) {
      setSource(example.source);
      setSelectedExample(exampleId);
      setIsUserEdited(false);
      
      // Save example selection to localStorage
      try {
        localStorage.setItem(STORAGE_EXAMPLE_KEY, exampleId);
        localStorage.removeItem(STORAGE_KEY);
      } catch (e) {
        console.error('Failed to save to localStorage:', e);
      }
    }
  }, [examples]);

  const tabs: { id: Tab; label: string; count?: number }[] = [
    { id: 'tokens', label: 'Tokens', count: lexResult?.tokens.length },
    { id: 'cst', label: 'CST', count: parseResult?.errors.length || undefined },
    { id: 'ast', label: 'AST', count: astResult?.nodes.length },
    { id: 'eval', label: 'Eval', count: evalResult?.values.length },
  ];

  if (loading) {
    return (
      <div className="min-h-screen bg-gray-900 text-white flex items-center justify-center">
        <div className="text-xl">Loading Cadenza Compiler...</div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-900 text-white flex flex-col">
      {/* Header */}
      <header className="bg-gray-800 border-b border-gray-700 px-4 py-2 md:py-3">
        <div className="flex items-center justify-between">
          <h1 className="text-lg md:text-xl font-bold text-purple-400">
            Cadenza Compiler Explorer
          </h1>
          <div className="hidden md:block text-sm text-gray-400">
            Interactive compiler development tool
          </div>
        </div>
      </header>

      {/* Main content - stacks vertically on mobile, side by side on desktop */}
      <main className="flex-1 flex flex-col md:flex-row min-h-0">
        {/* Editor panel */}
        <div className="h-[40vh] md:h-auto md:w-1/2 border-b md:border-b-0 md:border-r border-gray-700 flex flex-col">
          <div className="bg-gray-800 px-4 py-2 border-b border-gray-700 flex items-center justify-between gap-2">
            <span className="text-sm text-gray-400 whitespace-nowrap">Source Code</span>
            {examples.length > 0 && (
              <div className="flex items-center gap-2 flex-1 justify-end">
                <label htmlFor="example-select" className="text-xs text-gray-500 whitespace-nowrap">
                  Example:
                </label>
                <select
                  id="example-select"
                  value={selectedExample || ''}
                  onChange={(e) => handleExampleChange(e.target.value)}
                  className="text-sm bg-gray-700 text-gray-200 border border-gray-600 rounded px-2 py-1 focus:outline-none focus:ring-2 focus:ring-purple-500 max-w-[200px]"
                >
                  {isUserEdited && (
                    <option value="custom">Custom Code</option>
                  )}
                  {!selectedExample && !isUserEdited && (
                    <option value="">Select an example...</option>
                  )}
                  {examples.map((ex) => (
                    <option key={ex.id} value={ex.id}>
                      {ex.name}
                    </option>
                  ))}
                </select>
              </div>
            )}
          </div>
          <div className="flex-1 min-h-0">
            <SourceEditor value={source} onChange={handleSourceChange} />
          </div>
        </div>

        {/* Output panel */}
        <div className="flex-1 md:w-1/2 flex flex-col min-h-0">
          {/* Tab bar - horizontally scrollable on mobile */}
          <div className="bg-gray-800 border-b border-gray-700 flex overflow-x-auto">
            {tabs.map((tab) => (
              <button
                key={tab.id}
                onClick={() => setActiveTab(tab.id)}
                className={`px-3 md:px-4 py-2 text-sm font-medium transition-colors whitespace-nowrap flex-shrink-0 ${
                  activeTab === tab.id
                    ? 'text-purple-400 border-b-2 border-purple-400 bg-gray-900'
                    : 'text-gray-400 hover:text-gray-200 hover:bg-gray-700/50'
                }`}
              >
                {tab.label}
                {tab.count !== undefined && (
                  <span className="ml-1.5 px-1.5 py-0.5 text-xs rounded-full bg-gray-700">
                    {tab.count}
                  </span>
                )}
              </button>
            ))}
          </div>

          {/* Tab content */}
          <div className="flex-1 min-h-0 overflow-auto bg-gray-900">
            {activeTab === 'tokens' && <TokensPanel result={lexResult} />}
            {activeTab === 'cst' && <CstPanel result={parseResult} />}
            {activeTab === 'ast' && <AstPanel result={astResult} />}
            {activeTab === 'eval' && <EvalPanel result={evalResult} />}
          </div>
        </div>
      </main>

      {/* Footer */}
      <footer className="bg-gray-800 border-t border-gray-700 px-4 py-2">
        <div className="flex items-center justify-between text-xs text-gray-500">
          <span>
            {source.length} chars • {source.split('\n').length} lines
          </span>
          <span className="hidden sm:inline">
            Powered by Rust + WebAssembly
          </span>
        </div>
      </footer>
    </div>
  );
}

export default App

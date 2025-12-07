import { useState, useEffect, useRef } from 'react';
import Editor, { loader, OnMount } from '@monaco-editor/react';
import type * as monaco from 'monaco-editor';
import {
  CADENZA_LANGUAGE_ID,
  registerCadenzaLanguage,
  setupDiagnostics,
  setupHover,
  setupCompletions,
} from '../lib/monaco-cadenza';
import type { CadenzaWasm } from '../types/cadenza';

interface SourceEditorProps {
  value: string;
  onChange: (value: string) => void;
  wasm?: CadenzaWasm;
}

// Fallback textarea editor for when Monaco fails to load
function FallbackEditor({ value, onChange }: SourceEditorProps) {
  return (
    <textarea
      className="w-full h-full bg-gray-900 text-gray-100 p-4 font-mono text-sm resize-none focus:outline-none focus:ring-1 focus:ring-purple-500 border-0"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      spellCheck={false}
      placeholder="Enter your code here..."
    />
  );
}

// Timeout in ms for Monaco to load before falling back
const MONACO_LOAD_TIMEOUT = 10000;

export function SourceEditor({ value, onChange, wasm }: SourceEditorProps) {
  const [loadState, setLoadState] = useState<'loading' | 'loaded' | 'error'>('loading');
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mountedRef = useRef(true);
  const cleanupRef = useRef<Array<() => void>>([]);

  useEffect(() => {
    mountedRef.current = true;
    
    // Set a timeout to fall back to textarea if Monaco doesn't mount in time
    timeoutRef.current = setTimeout(() => {
      if (mountedRef.current) {
        setLoadState((current) => {
          if (current === 'loading') {
            console.warn('Monaco Editor load timeout - falling back to textarea');
            return 'error';
          }
          return current;
        });
      }
    }, MONACO_LOAD_TIMEOUT);

    // Try to initialize Monaco and catch any errors
    // Note: We don't clear the timeout here since init() success doesn't guarantee
    // the editor will mount. The timeout is only cleared in onMount or on error.
    loader.init().catch((error) => {
      console.error('Monaco Editor failed to load:', error);
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
      if (mountedRef.current) {
        setLoadState('error');
      }
    });

    return () => {
      mountedRef.current = false;
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
      // Cleanup LSP features
      cleanupRef.current.forEach((cleanup) => cleanup());
      cleanupRef.current = [];
    };
  }, []);

  const handleEditorDidMount: OnMount = (editor, monaco) => {
    // Editor mounted successfully - clear timeout and mark as loaded
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current);
    }
    if (mountedRef.current) {
      setLoadState('loaded');
    }

    // Register Cadenza language
    try {
      registerCadenzaLanguage();
    } catch (error) {
      console.error('Failed to register Cadenza language:', error);
    }

    // Set up LSP features if WASM is available
    if (wasm) {
      try {
        const model = editor.getModel();
        if (model) {
          // Set up diagnostics
          const cleanupDiag = setupDiagnostics(wasm, model);
          cleanupRef.current.push(cleanupDiag);

          // Set up hover provider
          const hoverDisposable = setupHover(wasm);
          cleanupRef.current.push(() => hoverDisposable.dispose());

          // Set up completion provider
          const completionDisposable = setupCompletions(wasm);
          cleanupRef.current.push(() => completionDisposable.dispose());
        }
      } catch (error) {
        console.error('Failed to set up LSP features:', error);
      }
    }
  };

  // If Monaco failed to load, show fallback textarea
  if (loadState === 'error') {
    return (
      <div className="h-full flex flex-col">
        <div className="bg-yellow-900/30 border-b border-yellow-700 px-3 py-1.5 text-yellow-400 text-xs">
          Rich editor unavailable. Using basic editor.
        </div>
        <div className="flex-1 min-h-0">
          <FallbackEditor value={value} onChange={onChange} />
        </div>
      </div>
    );
  }

  return (
    <div className="h-full">
      <Editor
        height="100%"
        defaultLanguage={CADENZA_LANGUAGE_ID}
        theme="vs-dark"
        value={value}
        onChange={(value) => onChange(value ?? '')}
        onMount={handleEditorDidMount}
        loading={
          <div className="flex items-center justify-center h-full text-gray-400">
            <div className="text-center">
              <div className="animate-pulse">Loading editor...</div>
            </div>
          </div>
        }
        options={{
          minimap: { enabled: false },
          fontSize: 14,
          lineNumbers: 'on',
          scrollBeyondLastLine: false,
          automaticLayout: true,
          tabSize: 2,
          wordWrap: 'on',
        }}
      />
    </div>
  );
}

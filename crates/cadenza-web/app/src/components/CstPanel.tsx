import type { ParseResult, CstNode } from '../types/cadenza';

interface CstPanelProps {
  result: ParseResult | null;
}

function getNodeColor(kind: string): string {
  if (kind === 'Root') return 'text-purple-400';
  if (kind === 'Apply') return 'text-blue-400';
  if (kind === 'Literal') return 'text-green-400';
  if (kind === 'Identifier') return 'text-cyan-400';
  if (kind.startsWith('String')) return 'text-yellow-400';
  if (['Space', 'Newline'].includes(kind)) return 'text-gray-600';
  return 'text-gray-300';
}

function CstNodeView({ node, depth = 0 }: { node: CstNode; depth?: number }) {
  const hasChildren = node.children.length > 0;
  const isWhitespace = ['Space', 'Newline'].includes(node.kind);
  
  if (isWhitespace && !hasChildren) {
    // Compact display for whitespace tokens
    return null;
  }

  return (
    <div className={`${depth > 0 ? 'ml-4 border-l border-gray-700 pl-2' : ''}`}>
      <div className="flex items-baseline gap-2 py-0.5 hover:bg-gray-800/50 rounded">
        <span className={`font-semibold ${getNodeColor(node.kind)}`}>
          {node.kind}
        </span>
        <span className="text-gray-500 text-xs">
          {node.start}..{node.end}
        </span>
        {node.text && (
          <span className="text-gray-400 text-sm">
            {JSON.stringify(node.text)}
          </span>
        )}
      </div>
      {hasChildren && (
        <div>
          {node.children.map((child, i) => (
            <CstNodeView key={i} node={child} depth={depth + 1} />
          ))}
        </div>
      )}
    </div>
  );
}

export function CstPanel({ result }: CstPanelProps) {
  if (!result) {
    return (
      <div className="p-4 text-gray-500 italic">
        Enter some code to see the CST...
      </div>
    );
  }

  return (
    <div className="p-4 overflow-auto h-full font-mono text-sm">
      {result.errors.length > 0 && (
        <div className="mb-4 p-2 bg-red-900/30 border border-red-700 rounded">
          <div className="text-red-400 font-semibold mb-1">Parse Errors:</div>
          {result.errors.map((err, i) => (
            <div key={i} className="text-red-300">
              [{err.start}..{err.end}] {err.message}
            </div>
          ))}
        </div>
      )}
      <CstNodeView node={result.tree} />
    </div>
  );
}

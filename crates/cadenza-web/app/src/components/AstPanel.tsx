import type { AstResult, AstNode } from '../types/cadenza';

interface AstPanelProps {
  result: AstResult | null;
}

function getNodeColor(type: string): string {
  if (type === 'Apply') return 'text-blue-400';
  if (type === 'Literal') return 'text-green-400';
  if (type === 'Ident') return 'text-cyan-400';
  if (type === 'Op') return 'text-purple-400';
  if (type === 'Attr') return 'text-yellow-400';
  if (type === 'Synthetic') return 'text-orange-400';
  if (type === 'Error') return 'text-red-400';
  return 'text-gray-300';
}

function AstNodeView({ node, depth = 0 }: { node: AstNode; depth?: number }) {
  const hasChildren = node.children.length > 0;

  return (
    <div className={`${depth > 0 ? 'ml-4 border-l border-gray-700 pl-2' : ''}`}>
      <div className="flex items-baseline gap-2 py-0.5 hover:bg-gray-800/50 rounded">
        <span className={`font-semibold ${getNodeColor(node.type)}`}>
          {node.type}
        </span>
        <span className="text-gray-500 text-xs">
          {node.start}..{node.end}
        </span>
        {node.value && (
          <span className="text-gray-400 text-sm">
            = {JSON.stringify(node.value)}
          </span>
        )}
      </div>
      {hasChildren && (
        <div>
          {node.children.map((child, i) => (
            <AstNodeView key={i} node={child} depth={depth + 1} />
          ))}
        </div>
      )}
    </div>
  );
}

export function AstPanel({ result }: AstPanelProps) {
  if (!result) {
    return (
      <div className="p-4 text-gray-500 italic">
        Enter some code to see the AST...
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
      {result.nodes.length === 0 ? (
        <div className="text-gray-500 italic">No AST nodes</div>
      ) : (
        result.nodes.map((node, i) => (
          <AstNodeView key={i} node={node} />
        ))
      )}
    </div>
  );
}

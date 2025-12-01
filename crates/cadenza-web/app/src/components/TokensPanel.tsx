import type { LexResult } from '../types/cadenza';

interface TokensPanelProps {
  result: LexResult | null;
}

function getTokenColor(kind: string): string {
  // Color coding for different token types
  if (kind === 'Identifier') return 'text-blue-400';
  if (kind === 'Integer' || kind === 'Float') return 'text-green-400';
  if (kind.startsWith('String')) return 'text-yellow-400';
  if (['Plus', 'Minus', 'Star', 'Slash'].includes(kind)) return 'text-purple-400';
  if (['Equal', 'Less', 'Greater'].includes(kind)) return 'text-red-400';
  if (['LParen', 'RParen', 'LBracket', 'RBracket', 'LBrace', 'RBrace'].includes(kind)) return 'text-orange-400';
  if (['Space', 'Newline'].includes(kind)) return 'text-gray-500';
  return 'text-gray-300';
}

export function TokensPanel({ result }: TokensPanelProps) {
  if (!result) {
    return (
      <div className="p-4 text-gray-500 italic">
        Enter some code to see tokens...
      </div>
    );
  }

  return (
    <div className="p-4 overflow-auto h-full">
      <table className="w-full text-sm font-mono">
        <thead>
          <tr className="text-left text-gray-400 border-b border-gray-700">
            <th className="pb-2 pr-4">Kind</th>
            <th className="pb-2 pr-4">Span</th>
            <th className="pb-2">Text</th>
          </tr>
        </thead>
        <tbody>
          {result.tokens.map((token, i) => (
            <tr key={i} className="border-b border-gray-800 hover:bg-gray-800/50">
              <td className={`py-1 pr-4 ${getTokenColor(token.kind)}`}>
                {token.kind}
              </td>
              <td className="py-1 pr-4 text-gray-500">
                {token.start}..{token.end}
              </td>
              <td className="py-1 text-gray-300">
                {token.kind === 'Newline' ? '↵' : 
                 token.kind === 'Space' ? '·'.repeat(token.text.length) : 
                 JSON.stringify(token.text)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

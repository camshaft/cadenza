import type { EvalResult } from '../types/cadenza';

interface EvalPanelProps {
  result: EvalResult | null;
}

function getValueColor(type: string): string {
  if (type === 'integer' || type === 'float') return 'text-green-400';
  if (type === 'string') return 'text-yellow-400';
  if (type === 'bool') return 'text-purple-400';
  if (type === 'nil') return 'text-gray-500';
  if (type === 'symbol') return 'text-cyan-400';
  if (type === 'list') return 'text-blue-400';
  return 'text-gray-300';
}

function getDiagnosticColor(level: string): string {
  if (level === 'error') return 'text-red-400 bg-red-900/30 border-red-700';
  if (level === 'warning') return 'text-yellow-400 bg-yellow-900/30 border-yellow-700';
  if (level === 'hint') return 'text-blue-400 bg-blue-900/30 border-blue-700';
  return 'text-gray-400 bg-gray-900/30 border-gray-700';
}

export function EvalPanel({ result }: EvalPanelProps) {
  if (!result) {
    return (
      <div className="p-4 text-gray-500 italic">
        Enter some code to see evaluation results...
      </div>
    );
  }

  return (
    <div className="p-4 overflow-auto h-full font-mono text-sm">
      {/* Diagnostics (errors, warnings, hints) */}
      {result.diagnostics.length > 0 && (
        <div className="mb-4 space-y-2">
          <div className="text-gray-400 font-semibold">Diagnostics:</div>
          {result.diagnostics.map((diag, i) => (
            <div
              key={i}
              className={`p-2 rounded border ${getDiagnosticColor(diag.level)}`}
            >
              <div className="flex items-center gap-2">
                <span className="uppercase text-xs font-bold">
                  {diag.level}
                </span>
                {diag.start !== null && (
                  <span className="text-gray-500 text-xs">
                    at {diag.start}..{diag.end}
                  </span>
                )}
              </div>
              <div className="mt-1">{diag.message}</div>
            </div>
          ))}
        </div>
      )}

      {/* Values */}
      <div>
        <div className="text-gray-400 font-semibold mb-2">Results:</div>
        {result.values.length === 0 ? (
          <div className="text-gray-500 italic">No values</div>
        ) : (
          <div className="space-y-1">
            {result.values.map((value, i) => (
              <div
                key={i}
                className="flex items-center gap-2 py-1 px-2 bg-gray-800/50 rounded"
              >
                <span className="text-gray-500 w-6">{i + 1}.</span>
                <span className={`${getValueColor(value.type)}`}>
                  {value.display}
                </span>
                <span className="text-gray-600 text-xs">: {value.type}</span>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Success indicator */}
      <div className="mt-4 pt-4 border-t border-gray-700">
        {result.success ? (
          <span className="text-green-400">✓ Evaluation successful</span>
        ) : (
          <span className="text-red-400">✗ Evaluation had errors</span>
        )}
      </div>
    </div>
  );
}

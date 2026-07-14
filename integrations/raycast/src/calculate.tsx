/// Raycast "Calculate" command for the Cadenza calculator.
///
/// A live-result list: as you type an expression in the Raycast bar, this shells out to
/// `cdz-calc --once --plain <expr>` and shows the value (Enter copies it, ⌘-Enter pastes it into the
/// frontmost app). Exact by default — `1 / 3` is `1/3`, not `0`. See ../README.md for the Command+Space
/// story (Spotlight itself has no API; Raycast is the extensible replacement).

import { List, ActionPanel, Action, Icon, getPreferenceValues } from "@raycast/api";
import { useState } from "react";
import { execFile } from "node:child_process";

interface Prefs {
  cdzCalcPath: string;
  cadenzaStore?: string;
  exact: boolean;
}

interface Result {
  /** The rendered value (on success) or the error/trap message (on failure). */
  text: string;
  ok: boolean;
}

/// Evaluate `expr` via `cdz-calc --once --plain`. Resolves to the bare value on success, or the stderr
/// message on a parse/type error or a runtime trap (a non-zero exit). Never rejects — a failure is a
/// `{ ok: false }` result the UI shows as a dimmed, non-copyable row.
function evaluate(expr: string, prefs: Prefs): Promise<Result> {
  const args = ["--plain"];
  if (!prefs.exact) args.push("--no-exact");
  args.push("--once", expr);
  const env = { ...process.env };
  if (prefs.cadenzaStore) env.CADENZA_STORE = prefs.cadenzaStore;
  return new Promise((resolve) => {
    execFile(prefs.cdzCalcPath || "cdz-calc", args, { env, timeout: 5000 }, (err, stdout, stderr) => {
      if (err) {
        const msg = (stderr || String(err)).trim();
        resolve({ text: msg || "not a valid expression", ok: false });
      } else {
        resolve({ text: stdout.trim(), ok: true });
      }
    });
  });
}

export default function Command() {
  const prefs = getPreferenceValues<Prefs>();
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<Result | null>(null);

  async function onChange(text: string) {
    setQuery(text);
    if (!text.trim()) {
      setResult(null);
      return;
    }
    setResult(await evaluate(text, prefs));
  }

  return (
    <List
      searchBarPlaceholder="1 / 3 + 1 / 3 + 1 / 3   ·   0.1 + 0.2   ·   1000000 * 1000000"
      onSearchTextChange={onChange}
      throttle
    >
      {result === null ? (
        <List.EmptyView
          icon={Icon.Calculator}
          title="Cadenza calculator"
          description="Type an expression — exact fractions, units, and big integers."
        />
      ) : result.ok ? (
        <List.Item
          icon={Icon.Calculator}
          title={result.text}
          subtitle={query}
          accessories={[{ text: "= " + query }]}
          actions={
            <ActionPanel>
              <Action.CopyToClipboard title="Copy Result" content={result.text} />
              <Action.Paste title="Paste Result" content={result.text} />
            </ActionPanel>
          }
        />
      ) : (
        <List.Item
          icon={Icon.ExclamationMark}
          title={query}
          subtitle={result.text}
        />
      )}
    </List>
  );
}

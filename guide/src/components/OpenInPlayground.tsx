/// "Open in playground" — hands the current inline-editor buffer to the full `/playground` IDE by
/// encoding it into the share hash and navigating there. Same encoding the playground's Share uses,
/// so the round-trip is lossless.

import { useNavigate } from "react-router-dom";
import { encodeShareHash } from "../playground/share.ts";
import type { Surface } from "../compiler/client.ts";

interface Props {
  getText: () => string;
  surface: () => Surface;
}

export function OpenInPlayground({ getText, surface }: Props) {
  const navigate = useNavigate();
  return (
    <button
      onClick={() => navigate(`/playground#${encodeShareHash({ s: surface(), src: getText() })}`)}
      title="Open this snippet in the full playground"
      className="rounded px-2 py-1 text-xs text-slate-500 transition hover:bg-slate-700/60 hover:text-slate-300"
    >
      Open in playground ↗
    </button>
  );
}

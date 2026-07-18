/// The registry that lets a clickable "change X to Y → result" prose span (`<TryChange>`) reach the
/// `<Runnable>` it refers to and drive it. Each `<Runnable id="…">` privately owns its editor state
/// (the `useCadenzaEditor` hook), so a SIBLING prose element in the same chapter has no direct handle
/// on it — this context is the bridge: a Runnable registers a small handle under its `id` on mount, and
/// `<TryChange example="id">` looks it up on click to apply a variant + re-run.
///
/// Follows the guide's context house-style (`SyntaxContext` / `ProgressContext`): a null-initialized
/// context, a Provider mounted once at the root (`main.tsx`, wrapping the router so every route can
/// register), and a hook. Unlike those, the registry hook does NOT throw outside a provider — a
/// `<Runnable>` rendered in a test or a stray context must still work, and a `<TryChange>` with no
/// provider degrades to plain prose (see `useRunnableHandle`). The registry is a plain mutable Map in a
/// ref (not React state): registration is a side effect that must not re-render the whole tree, and
/// lookups happen imperatively at click time, so there's nothing to render-subscribe to.

import { createContext, useContext, useMemo, useRef, type ReactNode } from "react";
import type { EditorOutcome } from "./useCadenzaEditor.ts";
import type { Surface } from "../syntax/SyntaxContext.tsx";

/// The imperative handle a `<Runnable>` exposes to clickable prose. A superset of the two apply paths:
/// a full authored `variant` and a one-token `find`/`replace` patch (see `TryChange`). Both apply to the
/// live buffer + run, and mirror the reader's changes into the visible editor.
export interface RunnableHandle {
  /** Replace the buffer with an authored variant (in `srcSurface`) and run it — the full-variant path. */
  applyVariant: (authoredSrc: string, srcSurface: Surface) => Promise<EditorOutcome>;
  /** Replace the single occurrence of `find` with `replace` in the buffer and run — the one-token patch.
   *  Resolves to null WITHOUT running if `find` doesn't occur exactly once (the authoring gate rejects
   *  such patches at build time; this is the runtime backstop). */
  applyPatch: (find: string, replace: string) => Promise<EditorOutcome | null>;
  /** Restore the original snippet + clear the result — the "put it back" teaching affordance. */
  reset: () => void;
}

interface Registry {
  register: (id: string, handle: RunnableHandle) => void;
  unregister: (id: string) => void;
  lookup: (id: string) => RunnableHandle | undefined;
}

const RunnableRegistryCtx = createContext<Registry | null>(null);

export function RunnableRegistryProvider({ children }: { children: ReactNode }) {
  // A plain Map in a ref: registration/unregistration are side effects that must NOT re-render the tree
  // (a chapter has many Runnables; a state Map would re-render every one on each mount), and lookups are
  // imperative at click time — nothing render-subscribes, so there is no state to hold.
  const map = useRef(new Map<string, RunnableHandle>());
  const value = useMemo<Registry>(
    () => ({
      register: (id, handle) => map.current.set(id, handle),
      unregister: (id) => map.current.delete(id),
      lookup: (id) => map.current.get(id),
    }),
    [],
  );
  return <RunnableRegistryCtx.Provider value={value}>{children}</RunnableRegistryCtx.Provider>;
}

/// The raw registry, or null outside a provider. Prefer the role-specific hooks below.
export function useRunnableRegistry(): Registry | null {
  return useContext(RunnableRegistryCtx);
}

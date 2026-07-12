/// Exercise progress tracking — persists which exercises the reader has completed, to build momentum
/// (a checkmark in the sidebar, a per-chapter count). Stored in localStorage so it survives reloads.
///
/// An exercise is identified by a stable string id (`chapterSlug:n`). The `Exercise` component reports
/// its id + chapter when the reader passes its Check; the sidebar reads the per-chapter tallies.

import { createContext, useCallback, useContext, useMemo, useState } from "react";
import type { ReactNode } from "react";

const STORAGE_KEY = "cadenza.progress";

function load(): Record<string, true> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as Record<string, true>) : {};
  } catch {
    return {};
  }
}

interface ProgressState {
  /** Mark an exercise (by stable id) complete. Idempotent. */
  complete: (id: string) => void;
  /** Has this exercise been completed? */
  isComplete: (id: string) => boolean;
  /** How many completed exercises' ids start with `${chapterSlug}:`. */
  countFor: (chapterSlug: string) => number;
  /** Forget all progress (a "reset progress" affordance). */
  clear: () => void;
}

const Ctx = createContext<ProgressState | null>(null);

export function ProgressProvider({ children }: { children: ReactNode }) {
  const [done, setDone] = useState<Record<string, true>>(load);

  const persist = useCallback((next: Record<string, true>) => {
    setDone(next);
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
    } catch {
      // storage full / disabled — progress is a nicety, not load-bearing; ignore.
    }
  }, []);

  const complete = useCallback(
    (id: string) => {
      setDone((prev) => {
        if (prev[id]) return prev; // already done — no state churn
        const next = { ...prev, [id]: true as const };
        try {
          localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
        } catch {
          /* ignore */
        }
        return next;
      });
    },
    [],
  );

  const value = useMemo<ProgressState>(
    () => ({
      complete,
      isComplete: (id) => !!done[id],
      countFor: (slug) => Object.keys(done).filter((k) => k.startsWith(`${slug}:`)).length,
      clear: () => persist({}),
    }),
    [done, complete, persist],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useProgress(): ProgressState {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useProgress must be used within a ProgressProvider");
  return ctx;
}

# Foreign levy before an abort (2026-08-18)

- `abl1.sexp` — the inner arm levies the outer handler then answers
  WITHOUT resuming: (do (T.levy) (+ s 900)). The body's pending (+ ... 7)
  is abandoned, but the levy's state write SURVIVES the abandonment and
  surfaces in the outer audit (90106 = inner 901 x100 + audit t0+5).
  Split oracle: dropping the doomed frame's levy shifts only the audit
  digit; a lowering that DOES resume (running the +7) shifts the
  hundreds. Boundary note: the same shape with a resume in the OTHER
  branch of an if (mixed abort/resume + levy) declines at the fold
  boundary (pya1-class, /tmp ladder) — pure-abort arms fold. PASS x3 at
  600e3f74f.

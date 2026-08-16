# Rest-pattern list destructures in arms (2026-08-11)

Angle: rest patterns ((list a b .. r)) are pinned in pure position; the ARM
destructuring a DISPATCHED list, and the REST binder itself crossing back out
and into a SECOND dispatch, were uncovered.

GREEN x3:
- lr2: heads+rest destructure of the op payload per dispatch (rest LENGTH +
  heads pinned) — 6700123/700123
- lr3: the rest binder resumes OUT of grab and re-crosses INTO sum — the rest
  view survives two boundary crossings; recursive rest-walk sums it — 60/60

Notes: an arm performing ITS OWN effect with no outer handler correctly
rejects CDZ0401 (re-confirmed while drafting lr3 — the forward model).
A plain binder in element position is a FIXED-ARITY pattern, not a rest
((list _rest a b c) = 4-element match; rest needs the `..` form).

Pin candidates: 248 pool.

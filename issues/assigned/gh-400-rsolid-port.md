# Mirrored from GitHub #400 — "Rsolid port"

- **Issue:** https://github.com/camshaft/cadenza/issues/400
- **Author:** camshaft (the operator)
- **Created:** 2026-07-15T21:27:56Z
- **Labels:** (none)
- **Routing:** NOT-YET-DESIGNED capability → concierge `backlog` note recommending a `design` agent
  (per the github-liaison role: a "wouldn't it be cool if…" feature, not a concrete bug/fix). The
  liaison does NOT spin up agents — that judgment stays with the concierge/operator.

## Operator's text (verbatim)
> I want to get a new vertical dedicated to building out a new tool using cadenza. I want to build a
> openscad like environment where you describe models in code and have it rendered. We should use the
> manifold library. We should use the existing ide infrastructure in the browser (but I don't want it
> to be limited to the browser, similar to the calculator). You can look at
> https://github.com/camshaft/rsolid and my printing repo for kind of what I'm after. It would be super
> interesting to actually use some of those as examples of how to build things up. So at a high level
> it would be to port rsolid to cadenza and make a really nice interface for it. I don't really care
> about the openscad backend - I think manifold is a lot faster route. And I'm pretty sure it compiles
> to wasm too

## Liaison notes (for the design agent / concierge)
A large new vertical, not a bugfix. Key design points extracted from the operator's text:
- **Goal:** an OpenSCAD-like CAD environment — describe 3D models in Cadenza code, render them.
- **Geometry backend:** the `manifold` library (NOT the openscad backend — operator explicitly prefers
  manifold for speed; believes it compiles to wasm — worth confirming as a peer/host component, which
  fits Cadenza's cross-component interop story).
- **Frontend:** reuse the existing browser IDE infra (like the guide playground / calculator), but ALSO
  work natively — not browser-limited (the calculator's multi-surface model is the precedent).
- **Reference material:** https://github.com/camshaft/rsolid (the thing to port) + the operator's
  "printing repo" — usable as worked examples of building models up.
- **Scope:** "port rsolid to Cadenza + a really nice interface." Needs a design pass to carve into
  vertical-ready increments (geometry primitives → CSG ops → manifold wasm binding → render surface →
  IDE integration).

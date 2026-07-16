# PR review comment — mirrored from GitHub PR #451 (Copilot inline)

- **PR:** #451 "fleet: seventy-first batch (guide /cad 3D-preview scaffold, …)" (MERGED)
- **File:** `guide/src/cad/MeshView.tsx:29` (`useGeometry`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592990975
- **Link:** https://github.com/camshaft/cadenza/pull/451#discussion_r3592990975

## Comment (verbatim)
> `useGeometry` creates a new `THREE.BufferGeometry` when buffers change (e.g. each Run), but the old geometry is never disposed. Over time that can leak GPU/CPU memory in a long-lived [preview].

## Liaison triage
The new CAD 3D-preview (guide `/cad`): `useGeometry` allocates a fresh `THREE.BufferGeometry` each time
the buffers change (each Run), but never calls `.dispose()` on the previous one — Three.js geometries
hold GPU buffers that aren't garbage-collected, so a long-lived preview session leaks GPU/CPU memory
across repeated Runs. FIX: dispose the previous geometry (a `useEffect` cleanup calling
`geometry.dispose()`, or dispose-on-replace). Guide territory (v-guide owns guide/src/cad). Fix on
`trunk`. Quote + link in queue file.

/// The CAD mesh driver for the /cad browser route — v-cad's module (G3b). A TS port of the Rust cdz-cad
/// (implementation/seed/crates/cdz-cad): parse a rendered EXACT `Solid` s-expr (Rational `n/d` coords) into
/// a CSG tree, walk it into a manifold-3d solid, triangulate, and return the flat triangle buffers the /cad
/// route feeds to three.js. v-guide-infra owns the route/canvas/deps; this module is v-cad's (per the split).
///
/// The run worker produces the model's rendered value like:
///   (: (Difference (Cube (: (tuple 50/1 30/1 5/1) Vec3)) (Sphere 127/20)) Solid)
/// The exact `Solid` model (implementation/cad) uses Rational coordinates (`n/d`); this parser reads the
/// numerator/denominator and evaluates to a JS number (n/d) at the mesh leaf — the MODEL is exact, the
/// geometry kernel (manifold) is float. Full-size Cube/Cylinder, no Rotate (no exact rotation).

import Module from "manifold-3d";
import type { ManifoldToplevel } from "manifold-3d";

/// The result of meshing a Solid: triangle buffers ready for a three.js BufferGeometry, or a typed error
/// to render (never throws). Flat shape — matches the /cad route's contract exactly.
///   - `positions`: flat vertex XYZ, 3 floats per vertex.
///   - `indices`: triangle indices into `positions` (3 per triangle).
///   - `normals`: optional flat per-vertex normals; the route computes them if omitted.
export type MeshResult =
  | { ok: true; positions: Float32Array; indices: Uint32Array; normals?: Float32Array }
  | { ok: false; error: string };

/// Tessellation of curved primitives (sphere/cylinder), matching the Rust driver's DEFAULT_SEGMENTS.
const SEGMENTS = 32;

/// The maximum Solid nesting depth (matches cdz-cad's MAX_DEPTH) — an adversarially deep input Errs cleanly
/// instead of overflowing the recursive descent.
const MAX_DEPTH = 256;

type Vec3 = [number, number, number];

/// The parsed CSG tree — mirror of cdz-cad's `Solid` enum, coordinates already evaluated (n/d → number).
type Solid =
  | { t: "empty" }
  | { t: "cube"; s: Vec3 }
  | { t: "sphere"; r: number }
  | { t: "cylinder"; h: number; r: number }
  | { t: "union"; a: Solid; b: Solid }
  | { t: "difference"; a: Solid; b: Solid }
  | { t: "intersection"; a: Solid; b: Solid }
  | { t: "translate"; v: Vec3; of: Solid }
  | { t: "scale"; v: Vec3; of: Solid };

// ── s-expr tokenizer + recursive-descent parser (twin of cdz-cad's lib.rs) ──────────────────────────

type Tok = "(" | ")" | { atom: string };

function tokenize(text: string): Tok[] {
  const toks: Tok[] = [];
  let cur = "";
  const flush = () => {
    if (cur) {
      toks.push({ atom: cur });
      cur = "";
    }
  };
  for (const c of text) {
    if (c === "(") {
      flush();
      toks.push("(");
    } else if (c === ")") {
      flush();
      toks.push(")");
    } else if (/\s/.test(c)) {
      flush();
    } else {
      cur += c;
    }
  }
  flush();
  return toks;
}

class Parser {
  pos = 0;
  depth = 0;
  toks: Tok[];
  constructor(toks: Tok[]) {
    this.toks = toks;
  }

  private bump(): Tok | undefined {
    return this.toks[this.pos++];
  }
  private isAtom(t: Tok | undefined, a?: string): t is { atom: string } {
    return typeof t === "object" && (a === undefined || t.atom === a);
  }
  private expectOpen(): void {
    if (this.bump() !== "(") throw new Error("expected `(`");
  }
  private expectClose(): void {
    if (this.bump() !== ")") throw new Error("expected `)`");
  }
  private expectAtom(): string {
    const t = this.bump();
    if (!this.isAtom(t)) throw new Error("expected an atom");
    return t.atom;
  }
  /// Is the upcoming form a `(: … …)` type annotation?
  private annotationAhead(): boolean {
    return this.toks[this.pos] === "(" && this.isAtom(this.toks[this.pos + 1], ":");
  }

  /// Parse a value at a Solid position, transparently unwrapping a `(: <value> Type>)` annotation.
  parseSolid(): Solid {
    if (++this.depth > MAX_DEPTH) {
      throw new Error(`solid nests deeper than the limit (${MAX_DEPTH})`);
    }
    let r: Solid;
    if (this.annotationAhead()) {
      this.expectOpen(); // (
      this.expectAtom(); // :
      r = this.parseSolid(); // the annotated value
      this.expectAtom(); // Type name (Solid)
      this.expectClose(); // )
    } else {
      r = this.parseNode();
    }
    this.depth--;
    return r;
  }

  private parseNode(): Solid {
    this.expectOpen();
    const head = this.expectAtom();
    let node: Solid;
    switch (head) {
      case "Empty":
        this.expectAtom(); // `unit` payload
        node = { t: "empty" };
        break;
      case "Cube":
        node = { t: "cube", s: this.parseVec3() };
        break;
      case "Sphere":
        node = { t: "sphere", r: this.parseRational() };
        break;
      case "Cylinder":
        node = { t: "cylinder", h: this.parseRational(), r: this.parseRational() };
        break;
      case "Union":
        node = { t: "union", a: this.parseSolid(), b: this.parseSolid() };
        break;
      case "Difference":
        node = { t: "difference", a: this.parseSolid(), b: this.parseSolid() };
        break;
      case "Intersection":
        node = { t: "intersection", a: this.parseSolid(), b: this.parseSolid() };
        break;
      case "Translate":
        node = { t: "translate", v: this.parseVec3(), of: this.parseSolid() };
        break;
      case "Scale":
        node = { t: "scale", v: this.parseVec3(), of: this.parseSolid() };
        break;
      default:
        throw new Error(`unknown Solid constructor \`${head}\``);
    }
    this.expectClose();
    return node;
  }

  private parseVec3(): Vec3 {
    if (this.annotationAhead()) {
      this.expectOpen(); // (
      this.expectAtom(); // :
      const v = this.parseVec3(); // inner (tuple …)
      this.expectAtom(); // Vec3
      this.expectClose(); // )
      return v;
    }
    this.expectOpen();
    if (this.expectAtom() !== "tuple") throw new Error("expected a Vec3 `(tuple …)`");
    const v: Vec3 = [this.parseRational(), this.parseRational(), this.parseRational()];
    this.expectClose();
    return v;
  }

  /// A RATIONAL number leaf `n/d` → the JS number n/d (division at the leaf; the model stays exact). A bare
  /// integer or float is accepted defensively; a `nan` atom maps to NaN; a zero denominator errs.
  private parseRational(): number {
    const a = this.expectAtom();
    // Cadenza renders a NaN float as the LOWERCASE atom `nan` (the compiler's `Prim::FloatNan`), so match
    // case-insensitively — an uppercase-only check missed a real `nan` value and threw. This MUST stay before
    // the `Number(a)` fallthrough below: `Number("nan")` is NaN, which `Number.isFinite` then rejects → throw.
    if (a.toLowerCase() === "nan") return NaN;
    const slash = a.indexOf("/");
    if (slash >= 0) {
      const n = Number(a.slice(0, slash));
      const d = Number(a.slice(slash + 1));
      if (!Number.isFinite(n) || !Number.isFinite(d)) throw new Error(`bad rational \`${a}\``);
      if (d === 0) throw new Error(`rational \`${a}\` has a zero denominator`);
      return n / d;
    }
    const x = Number(a);
    if (!Number.isFinite(x)) throw new Error(`expected a rational \`n/d\`, found \`${a}\``);
    return x;
  }
}

/// Parse a rendered `Solid` s-expression into a CSG tree (throws on a malformed form).
function parseSolid(text: string): Solid {
  const p = new Parser(tokenize(text.trim()));
  const s = p.parseSolid();
  if (p.pos !== p.toks.length) throw new Error("trailing tokens after the solid");
  return s;
}

// ── mesh walk: Solid → manifold-3d Manifold (twin of cdz-cad's mesh.rs) ─────────────────────────────

type ManifoldStatic = ManifoldToplevel["Manifold"];
type ManifoldObj = InstanceType<ManifoldStatic>;

function toManifold(M: ManifoldStatic, s: Solid): ManifoldObj {
  switch (s.t) {
    case "empty":
      // An empty solid → manifold's canonical EMPTY (no geometry), matching the Rust cdz-cad driver's
      // `Manifold::empty()`. The encapsulated API has no bare empty ctor, so use a zero-size cube — which
      // manifold documents (and we verified: `isEmpty()===true`, 0 triangles) as returning an empty Manifold,
      // NOT degenerate geometry. This composes correctly when nested (an empty arm of a union/difference).
      return M.cube([0, 0, 0], true);
    case "cube":
      return M.cube(s.s, true); // FULL size, centred (matches Cube semantics)
    case "sphere":
      return M.sphere(s.r, SEGMENTS);
    case "cylinder":
      return M.cylinder(s.h, s.r, s.r, SEGMENTS, true); // constant-radius, FULL height, centred
    case "union":
      return toManifold(M, s.a).add(toManifold(M, s.b));
    case "difference":
      return toManifold(M, s.a).subtract(toManifold(M, s.b));
    case "intersection":
      return toManifold(M, s.a).intersect(toManifold(M, s.b));
    case "translate":
      return toManifold(M, s.of).translate(s.v);
    case "scale":
      return toManifold(M, s.of).scale(s.v);
  }
}

// The manifold-3d WASM module is initialized once (async) and memoized — v-cad owns this init; the whole
// module is lazy-loaded behind the /cad route so the wasm never touches the guide's critical path.
let toplevelPromise: Promise<ManifoldToplevel> | null = null;
async function manifoldStatic(): Promise<ManifoldStatic> {
  if (!toplevelPromise) {
    toplevelPromise = Module().then((tl) => {
      tl.setup();
      return tl;
    });
  }
  return (await toplevelPromise).Manifold;
}

/// Mesh a rendered `Solid` value into triangle buffers (never throws — a typed error on parse/mesh failure).
export async function meshFromSolid(solidText: string): Promise<MeshResult> {
  let tree: Solid;
  try {
    tree = parseSolid(solidText);
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
  try {
    const M = await manifoldStatic();
    const gl = toManifold(M, tree).getMesh();
    // MeshGL packs vertex properties as [x, y, z, …extra] per vertex, stride = numProp. With numProp === 3
    // it IS the flat position array; otherwise strip out the first 3 (position) props per vertex.
    const numProp = gl.numProp;
    const positions =
      numProp === 3
        ? gl.vertProperties
        : Float32Array.from(
            { length: (gl.vertProperties.length / numProp) * 3 },
            (_, i) => gl.vertProperties[Math.floor(i / 3) * numProp + (i % 3)],
          );
    return { ok: true, positions, indices: gl.triVerts };
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

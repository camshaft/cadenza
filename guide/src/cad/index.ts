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
type Vec2 = [number, number];

/// A 2-D path segment (mirror of cdz-cad's `PathSeg`) — absolute (`*Abs`) or relative (`*Rel`) to the
/// running cursor. A cubic Bézier carries end + two control points.
type PathSeg =
  | { k: "moveAbs"; p: Vec2 }
  | { k: "moveRel"; d: Vec2 }
  | { k: "lineAbs"; p: Vec2 }
  | { k: "lineRel"; d: Vec2 }
  | { k: "cubicAbs"; e: Vec2; c0: Vec2; c1: Vec2 }
  | { k: "cubicRel"; e: Vec2; c0: Vec2; c1: Vec2 };

/// A 2-D cross-section (mirror of cdz-cad's `Profile`) — the input an extrude/revolve lifts.
type Profile =
  | { p: "rect"; w: number; h: number }
  | { p: "circle"; r: number }
  | { p: "path"; segs: PathSeg[] };

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
  | { t: "scale"; v: Vec3; of: Solid }
  | { t: "extrudeLinear"; profile: Profile; height: number }
  | { t: "revolve"; profile: Profile; degrees: number };

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
      case "ExtrudeLinear":
        node = { t: "extrudeLinear", profile: this.parseProfile(), height: this.parseRational() };
        break;
      case "Revolve":
        node = { t: "revolve", profile: this.parseProfile(), degrees: this.parseRational() };
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

  /// A `Vec2` — `(: (tuple x y) Vec2R)` annotation or bare `(tuple x y)` (type-name atom discarded).
  private parseVec2(): Vec2 {
    if (this.annotationAhead()) {
      this.expectOpen();
      this.expectAtom(); // :
      const v = this.parseVec2();
      this.expectAtom(); // Vec2R
      this.expectClose();
      return v;
    }
    this.expectOpen();
    if (this.expectAtom() !== "tuple") throw new Error("expected a Vec2 `(tuple …)`");
    const v: Vec2 = [this.parseRational(), this.parseRational()];
    this.expectClose();
    return v;
  }

  /// A `Profile` — `(Rect <Vec2>)` / `(Circle <r>)` / `(PathProfile <Path>)` (outer annotation discarded).
  private parseProfile(): Profile {
    if (this.annotationAhead()) {
      this.expectOpen();
      this.expectAtom(); // :
      const p = this.parseProfile();
      this.expectAtom(); // ProfileR
      this.expectClose();
      return p;
    }
    this.expectOpen();
    const head = this.expectAtom();
    let prof: Profile;
    switch (head) {
      case "Rect": {
        const [w, h] = this.parseVec2();
        prof = { p: "rect", w, h };
        break;
      }
      case "Circle":
        prof = { p: "circle", r: this.parseRational() };
        break;
      case "PathProfile":
        prof = { p: "path", segs: this.parsePath() };
        break;
      default:
        throw new Error(`unknown Profile constructor \`${head}\``);
    }
    this.expectClose();
    return prof;
  }

  /// A `Path` — `(: (list <seg…>) PathR)` or bare `(list <seg…>)` (type-name atom discarded).
  private parsePath(): PathSeg[] {
    if (this.annotationAhead()) {
      this.expectOpen();
      this.expectAtom(); // :
      const segs = this.parsePath();
      this.expectAtom(); // PathR
      this.expectClose();
      return segs;
    }
    this.expectOpen();
    if (this.expectAtom() !== "list") throw new Error("expected a Path `(list …)`");
    const segs: PathSeg[] = [];
    while (this.toks[this.pos] === "(") segs.push(this.parsePathSeg());
    this.expectClose();
    return segs;
  }

  /// One `PathSeg` — `(MoveToAbs <Vec2>)` / `LineToRel` / `(CubicToAbs <Vec2> <Vec2> <Vec2>)` etc.
  private parsePathSeg(): PathSeg {
    this.expectOpen();
    const head = this.expectAtom();
    let seg: PathSeg;
    switch (head) {
      case "MoveToAbs":
        seg = { k: "moveAbs", p: this.parseVec2() };
        break;
      case "MoveToRel":
        seg = { k: "moveRel", d: this.parseVec2() };
        break;
      case "LineToAbs":
        seg = { k: "lineAbs", p: this.parseVec2() };
        break;
      case "LineToRel":
        seg = { k: "lineRel", d: this.parseVec2() };
        break;
      case "CubicToAbs":
        seg = { k: "cubicAbs", e: this.parseVec2(), c0: this.parseVec2(), c1: this.parseVec2() };
        break;
      case "CubicToRel":
        seg = { k: "cubicRel", e: this.parseVec2(), c0: this.parseVec2(), c1: this.parseVec2() };
        break;
      default:
        throw new Error(`unknown PathSeg constructor \`${head}\``);
    }
    this.expectClose();
    return seg;
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
type CrossSectionStatic = ManifoldToplevel["CrossSection"];
type CrossSectionObj = InstanceType<CrossSectionStatic>;

/// Build a 2-D `CrossSection` from a [`Profile`] — the twin of cdz-cad's `profile_to_cross_section`. `rect`/
/// `circle` map onto manifold's centred `square`/`circle`; a `path` is sampled to a polygon (`samplePath`)
/// then built as a single-contour CrossSection.
function profileToCrossSection(CS: CrossSectionStatic, p: Profile): CrossSectionObj {
  switch (p.p) {
    case "rect":
      return CS.square([p.w, p.h], true);
    case "circle":
      return CS.circle(p.r, SEGMENTS);
    case "path":
      return new CS([samplePath(p.segs)]);
  }
}

/// Sample a path's segments to a flat polygon (`[x,y]` points), walking the cursor — the twin of cdz-cad's
/// `sample_path`. A line/move contributes its (absolute) endpoint; a cubic Bézier is sampled at `SEGMENTS`
/// points via the Bernstein form; relative segments offset the running cursor.
function samplePath(segs: PathSeg[]): Vec2[] {
  const pts: Vec2[] = [];
  let cur: Vec2 = [0, 0];
  const add = (a: Vec2, b: Vec2): Vec2 => [a[0] + b[0], a[1] + b[1]];
  for (const seg of segs) {
    switch (seg.k) {
      case "moveAbs":
      case "lineAbs":
        cur = seg.p;
        pts.push(cur);
        break;
      case "moveRel":
      case "lineRel":
        cur = add(cur, seg.d);
        pts.push(cur);
        break;
      case "cubicAbs":
        sampleCubic(pts, cur, seg.c0, seg.c1, seg.e);
        cur = seg.e;
        break;
      case "cubicRel": {
        const e = add(cur, seg.e);
        sampleCubic(pts, cur, add(cur, seg.c0), add(cur, seg.c1), e);
        cur = e;
        break;
      }
    }
  }
  return pts;
}

/// Push `SEGMENTS` sample points of the cubic Bézier `p0→p3` (controls `p1`,`p2`) for `t` in (0,1] — `p0` is
/// assumed already emitted by the prior segment, so we sample the interior + endpoint (Bernstein form).
function sampleCubic(pts: Vec2[], p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2): void {
  const n = Math.max(1, SEGMENTS);
  for (let i = 1; i <= n; i++) {
    const t = i / n;
    const u = 1 - t;
    const b0 = u * u * u;
    const b1 = 3 * u * u * t;
    const b2 = 3 * u * t * t;
    const b3 = t * t * t;
    pts.push([
      b0 * p0[0] + b1 * p1[0] + b2 * p2[0] + b3 * p3[0],
      b0 * p0[1] + b1 * p1[1] + b2 * p2[1] + b3 * p3[1],
    ]);
  }
}

function toManifold(M: ManifoldStatic, CS: CrossSectionStatic, s: Solid): ManifoldObj {
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
      return toManifold(M, CS, s.a).add(toManifold(M, CS, s.b));
    case "difference":
      return toManifold(M, CS, s.a).subtract(toManifold(M, CS, s.b));
    case "intersection":
      return toManifold(M, CS, s.a).intersect(toManifold(M, CS, s.b));
    case "translate":
      return toManifold(M, CS, s.of).translate(s.v);
    case "scale":
      return toManifold(M, CS, s.of).scale(s.v);
    case "extrudeLinear":
      // lift the profile straight up +z by `height`, centred (matches the origin-centred primitives + the
      // Rust driver: extrude runs 0..height then shift down height/2).
      return profileToCrossSection(CS, s.profile)
        .extrude(s.height, 0, 0, 1, true);
    case "revolve":
      // sweep the profile about the y-axis by `degrees` (SEGMENTS = sweep tessellation).
      return M.revolve(profileToCrossSection(CS, s.profile), SEGMENTS, s.degrees);
  }
}

// The manifold-3d WASM module is initialized once (async) and memoized — v-cad owns this init; the whole
// module is lazy-loaded behind the /cad route so the wasm never touches the guide's critical path.
let toplevelPromise: Promise<ManifoldToplevel> | null = null;
async function manifoldStatics(): Promise<{ M: ManifoldStatic; CS: CrossSectionStatic }> {
  if (!toplevelPromise) {
    toplevelPromise = Module().then((tl) => {
      tl.setup();
      return tl;
    });
  }
  const tl = await toplevelPromise;
  return { M: tl.Manifold, CS: tl.CrossSection };
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
    const { M, CS } = await manifoldStatics();
    const gl = toManifold(M, CS, tree).getMesh();
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

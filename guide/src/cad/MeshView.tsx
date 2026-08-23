/// The 3D canvas for a CAD mesh — react-three-fiber over three.js. Owned by this vertical (the render
/// surface); it takes the triangle buffers v-cad's `meshFromSolid` produces and draws them with a
/// slow auto-rotate, orbit controls, and simple lighting. Pulled in only by CadPage (behind the lazy
/// /cad route), so three.js/@react-three code-splits off the guide's first paint.

import { useEffect, useMemo, useRef } from "react";
import { Canvas, useFrame } from "@react-three/fiber";
import { OrbitControls, Bounds } from "@react-three/drei";
import * as THREE from "three";

interface Props {
  positions: Float32Array;
  indices: Uint32Array;
  normals?: Float32Array;
}

interface ViewProps extends Props {
  /// Auto-rotate the model. DEFAULT OFF (the operator called the constant spin "annoying" + it fought
  /// manual orbit); a fixed view is the default, toggled on demand by the caller.
  spin?: boolean;
}

/// Build a three.js BufferGeometry from the mesh buffers. Memoized on the buffers so a re-render
/// (e.g. a resize) doesn't rebuild the geometry — and DISPOSED when it changes or the view unmounts: a
/// BufferGeometry holds GPU buffers the JS GC never reclaims, so re-Running (new buffers → new geometry)
/// would leak the old one across a long preview session without an explicit dispose.
///
/// NORMALS: the material uses `flatShading` (below), so we do NOT compute per-vertex normals here. The
/// mesh driver returns an INDEXED mesh (manifold shares vertices at edges); `computeVertexNormals` would
/// AVERAGE the normal across a shared vertex, smoothing a hard CSG edge into a Gouraud-blended curve — the
/// "rippling / curvy surfaces instead of crisp planes" the operator flagged (v-cad root-caused: the
/// geometry itself is exact + watertight; the artifact was smooth shading). With `flatShading`, three.js
/// derives a per-FRAGMENT face normal from screen-space derivatives, so flat CSG faces stay crisp and
/// edges stay sharp regardless of the indexed vertex sharing — the correct look for CSG. Driver-supplied
/// normals (if ever present) are still honored; otherwise flat shading needs none.
function useGeometry({ positions, indices, normals }: Props): THREE.BufferGeometry {
  const geometry = useMemo(() => {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    g.setIndex(new THREE.BufferAttribute(indices, 1));
    if (normals) g.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
    return g;
  }, [positions, indices, normals]);
  // Dispose the PREVIOUS geometry when a new one replaces it (the cleanup closes over the old geometry),
  // and on unmount — freeing its GPU buffers.
  useEffect(() => () => geometry.dispose(), [geometry]);
  return geometry;
}

function Solid({ spin, ...props }: ViewProps) {
  const geometry = useGeometry(props);
  const ref = useRef<THREE.Mesh>(null);
  // Optional gentle idle spin (OFF by default — operator found the constant spin annoying). When on, rotate
  // the mesh; when off, leave the vantage entirely to the reader's OrbitControls drag.
  useFrame((_, delta) => {
    if (spin && ref.current) ref.current.rotation.y += delta * 0.3;
  });
  return (
    <mesh ref={ref} geometry={geometry}>
      {/* flatShading: crisp per-face shading for CSG — three.js derives a per-fragment face normal, so hard
          edges stay sharp + flat planes stay flat on the indexed manifold mesh (no Gouraud rippling). */}
      <meshStandardMaterial color="#38bdf8" metalness={0.1} roughness={0.6} flatShading />
    </mesh>
  );
}

export function MeshView({ spin = false, ...props }: ViewProps) {
  return (
    <Canvas camera={{ position: [4, 3, 5], fov: 45 }} style={{ width: "100%", height: "100%" }}>
      <ambientLight intensity={0.6} />
      <directionalLight position={[5, 8, 5]} intensity={1.1} />
      <directionalLight position={[-5, -3, -5]} intensity={0.3} />
      {/* Auto-FIT the camera to the mesh's bounds so a model of ANY size frames correctly — the fixed
          [4,3,5] camera only suited single-digit-mm models; a real-scale part (e.g. a Ø90mm stand spanning
          x,y ∈ [-45,45]) would ENGULF that camera, showing back-face-culled interior walls (an "empty"
          preview). `observe` re-fits when the geometry changes (a slider drag re-meshes), `clip` sets the
          near/far planes to the model, `margin` leaves a little padding. OrbitControls `makeDefault` lets
          Bounds drive the same camera/controls the reader then orbits. */}
      <Bounds fit clip observe margin={1.2}>
        <Solid spin={spin} {...props} />
      </Bounds>
      {/* OrbitControls with OpenSCAD-style mouse mapping (operator ask): LEFT drag = rotate, RIGHT drag = PAN,
          MIDDLE/wheel = dolly (zoom). `enablePan` on + `mouseButtons` remaps the right button from its default
          (pan is normally the RIGHT button in three.js already, but the default was disabled via
          `enablePan={false}`); we enable pan and pin the mapping explicitly so it matches OpenSCAD regardless
          of three.js defaults. Touch: one-finger rotate, two-finger dolly+pan (drei defaults, left as-is). */}
      <OrbitControls
        makeDefault
        enablePan
        mouseButtons={{
          LEFT: THREE.MOUSE.ROTATE,
          MIDDLE: THREE.MOUSE.DOLLY,
          RIGHT: THREE.MOUSE.PAN,
        }}
      />
    </Canvas>
  );
}

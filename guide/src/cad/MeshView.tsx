/// The 3D canvas for a CAD mesh — react-three-fiber over three.js. Owned by this vertical (the render
/// surface); it takes the triangle buffers v-cad's `meshFromSolid` produces and draws them with a
/// slow auto-rotate, orbit controls, and simple lighting. Pulled in only by CadPage (behind the lazy
/// /cad route), so three.js/@react-three code-splits off the guide's first paint.

import { useEffect, useMemo, useRef } from "react";
import { Canvas, useFrame } from "@react-three/fiber";
import { OrbitControls } from "@react-three/drei";
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

/// Build a three.js BufferGeometry from the mesh buffers. Computes vertex normals when the driver didn't
/// supply them (flat/faceted shading otherwise looks unlit). Memoized on the buffers so a re-render
/// (e.g. a resize) doesn't rebuild the geometry — and DISPOSED when it changes or the view unmounts: a
/// BufferGeometry holds GPU buffers the JS GC never reclaims, so re-Running (new buffers → new geometry)
/// would leak the old one across a long preview session without an explicit dispose.
function useGeometry({ positions, indices, normals }: Props): THREE.BufferGeometry {
  const geometry = useMemo(() => {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    g.setIndex(new THREE.BufferAttribute(indices, 1));
    if (normals) g.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
    else g.computeVertexNormals();
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
      <meshStandardMaterial color="#38bdf8" metalness={0.1} roughness={0.6} />
    </mesh>
  );
}

export function MeshView({ spin = false, ...props }: ViewProps) {
  return (
    <Canvas camera={{ position: [4, 3, 5], fov: 45 }} style={{ width: "100%", height: "100%" }}>
      <ambientLight intensity={0.6} />
      <directionalLight position={[5, 8, 5]} intensity={1.1} />
      <directionalLight position={[-5, -3, -5]} intensity={0.3} />
      <Solid spin={spin} {...props} />
      <OrbitControls enablePan={false} />
    </Canvas>
  );
}

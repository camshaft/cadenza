//! Deployment boot (`DESIGN-http-outpost.md` §0.1 "deploy once, install software on it") — behind the
//! `host` feature.
//!
//! Stands up the gateway as a runnable server from a local deployment directory: a `route-table.bin` frame
//! (the same `route-table` the control server ships) + the `*.wasm` component files it references (the
//! value-heap runtime, NFC, and each handler — a local content store standing in for the CAS the real
//! gateway downloads from). This is the v0 deployable: run the node once, drop new handler `.wasm` + an
//! updated `route-table.bin` in the dir to change what it serves (the "swap by hash, no redeploy" property,
//! here as a restart). The live ws control link (P3) replaces the local dir with a dialed control server.

use crate::control::assemble_edge;
use bytes::Bytes;
use cdz_platform::{HostId, ReducerId};
use std::net::SocketAddr;
use std::path::Path;

/// Load a deployment directory: the `route-table.bin` frame + every `*.wasm` component (the value-heap
/// runtime, NFC, and the handler components). Returns `(frame, components)`.
///
/// # Errors
/// Propagates any I/O error reading the directory, the frame, or a component file.
pub fn load_deployment(dir: &Path) -> std::io::Result<(Bytes, Vec<Bytes>)> {
    let frame = Bytes::from(std::fs::read(dir.join("route-table.bin"))?);
    let mut components = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "wasm") {
            components.push(Bytes::from(std::fs::read(&path)?));
        }
    }
    Ok((frame, components))
}

/// Boot + serve a deployment from `dir` on `addr`, blocking until an accept error. The gateway's node
/// identity is a v0 default; each route's delivered contract-id comes from the route-table frame.
///
/// # Errors
/// An I/O error loading the deployment or binding the listener, or `InvalidData` if the route-table frame
/// is malformed.
pub async fn serve(addr: SocketAddr, dir: &Path) -> std::io::Result<()> {
    let (frame, components) = load_deployment(dir)?;
    let component_count = components.len();
    let edge = assemble_edge(
        &frame,
        &components,
        HostId::of(b"cdz-http-gateway"),
        ReducerId::of(b"router"),
    )
    .await
    .ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "malformed route-table frame (a handler hash is not a valid content hash)",
        )
    })?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("cdz-http-gateway: listening on {addr} ({component_count} components loaded)");
    edge.serve(listener).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Method, RouteFrame, encode_route_table};

    /// The loader reads the frame + every `.wasm` (and nothing else) from a deployment dir.
    #[test]
    fn loads_the_frame_and_wasm_components() {
        let dir = std::env::temp_dir().join(format!("cdz-http-boot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let frame = encode_route_table(&[RouteFrame {
            method: Method::Get,
            path: "/".to_string(),
            handler: Bytes::from_static(b"h"),
            contract: Bytes::from_static(b"c"),
        }]);
        std::fs::write(dir.join("route-table.bin"), &frame).unwrap();
        std::fs::write(dir.join("runtime.wasm"), b"fake-runtime").unwrap();
        std::fs::write(dir.join("handler.wasm"), b"fake-handler").unwrap();
        std::fs::write(dir.join("README.txt"), b"ignored").unwrap();

        let (loaded_frame, components) = load_deployment(&dir).unwrap();
        assert_eq!(loaded_frame, frame, "the route-table frame round-trips");
        assert_eq!(
            components.len(),
            2,
            "both .wasm files load, the .txt is ignored"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}

//! Schema resolution — the kernel's `schema-hash → declared-name` lookup seam (envelope D14=A, slice 1).
//!
//! D14=A eliminates the `ContentType` struct: the wire content-identity of an effect/event becomes a BARE
//! schema-hash (a single [`Hash`]), and the stable family/name — the routing key and the Cedar authz action
//! — relocates INTO the self-describing schema (the `(effect Weather …)` head is the name, right there in
//! the schema AST the hash addresses). So routing and authz can no longer read a `family` field off the
//! message; they resolve `schema-hash → schema AST → declared name` instead.
//!
//! This module is the kernel-side SEAM for that resolution — a trait the kernel calls, the HOST implements.
//! Per minimize-kernel (operator directive, concierge-confirmed 2026-08-09): the kernel holds NO schema
//! storage or cache; the host owns the schema store (its s3fifo blob cache) and implements this seam so the
//! kernel just calls a lookup. This mirrors the [`crate::authz::Authorize`] swap-seam and the userspace-
//! effects shared-`NameStore` `(A)` decision (the store/cache is host-owned; the kernel calls in).
//!
//! **Pure lookup (replay-safety).** A resolution is a deterministic function of content: a given schema-hash
//! addresses exactly one schema, whose declared name is fixed. The seam therefore holds NO kernel state and
//! performs NO mutation — a resolve on replay yields the same answer as it did live (the schema store is
//! external, content-addressed, and never rebuilt from a session's log; the same class as `NameStore`, see
//! [`crate::kernel::Session`]'s `name_store` field). Keeping it a pure lookup is what makes slice 2's wire
//! flip (route/authz through this seam) replay-safe for free. If a future need pushes kernel-held state into
//! this seam, that crosses the minimize-kernel line and needs an operator ruling — do not add it silently.

use crate::hash::Hash;

/// The schema-resolution SEAM (envelope D14=A): map a wire schema-hash to the STABLE DECLARED NAME the
/// routing/authz layer keys on. Taken by the kernel as `&dyn ResolveSchema` so the schema store is swappable
/// and host-owned WITHOUT the kernel holding any schema state (minimize-kernel). One async trait (operator
/// ruling: "one async trait only") — a host impl may `.await` a cache/blob fetch; a pure in-memory test impl
/// just returns.
///
/// Contract: total + PURE — it inspects the content-addressed schema store and returns a name (or `None`),
/// mutating nothing. `Some(name)` = the schema for `schema_hash` is known and declares `name` (the stable
/// routing/Cedar key, unchanged across contract versions of the same effect). `None` = the hash resolves to
/// no known schema, or the schema is not a named effect-schema — the FAIL-CLOSED answer: routing treats it as
/// no-handler and authz as no-match, so an unknown/garbage schema-hash never spuriously routes or is granted.
/// Must not panic (§17).
///
/// **Object-safe via `async-trait`.** Called through `&dyn ResolveSchema`, so `#[async_trait(?Send)]`;
/// `?Send` for the single-threaded kernel (a host impl may hold a non-`Send` cache handle).
#[async_trait::async_trait(?Send)]
pub trait ResolveSchema {
    /// Resolve `schema_hash` to the stable declared name of the schema it addresses (the `(effect <Name> …)`
    /// head), or `None` if the hash names no known schema / a non-effect schema (fail-closed). PURE — no
    /// mutation, deterministic from content, so replay-safe. The host impl reads its content-addressed schema
    /// store (s3fifo blob cache) and extracts the declared name via `cadenza-ast` (the schema-AST reader).
    async fn resolve_declared_name(&self, schema_hash: Hash) -> Option<std::sync::Arc<str>>;
}

/// A resolver that knows NO schemas — every resolution is `None` (fail-closed). The inert default for a
/// kernel loop wired before a real host schema store is attached (mirrors [`crate::reducer::InertReducer`]):
/// with it, every schema-hash resolves to "unknown", so routing finds no handler and authz no match — the
/// safe pre-wiring behavior, never a spurious route/grant.
pub struct InertSchemaResolver;

#[async_trait::async_trait(?Send)]
impl ResolveSchema for InertSchemaResolver {
    async fn resolve_declared_name(&self, _schema_hash: Hash) -> Option<std::sync::Arc<str>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // A fixed-map test resolver — a stand-in for the host's content-addressed schema store. Proves the seam
    // is object-safe (drivable through `&dyn ResolveSchema`) and returns the declared name for a known hash,
    // `None` for an unknown one (the fail-closed contract).
    struct MapResolver {
        by_hash: HashMap<Hash, std::sync::Arc<str>>,
    }

    #[async_trait::async_trait(?Send)]
    impl ResolveSchema for MapResolver {
        async fn resolve_declared_name(&self, schema_hash: Hash) -> Option<std::sync::Arc<str>> {
            self.by_hash.get(&schema_hash).cloned()
        }
    }

    fn poll_ready<F: std::future::Future>(fut: F) -> F::Output {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn noop_raw() -> RawWaker {
            fn no_op(_: *const ()) {}
            fn clone(_: *const ()) -> RawWaker {
                noop_raw()
            }
            RawWaker::new(
                std::ptr::null(),
                &RawWakerVTable::new(clone, no_op, no_op, no_op),
            )
        }
        let waker = unsafe { Waker::from_raw(noop_raw()) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = std::pin::pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("poll_ready: future was not immediately ready"),
        }
    }

    #[test]
    fn resolver_is_object_safe_and_resolves_a_known_hash_none_for_unknown() {
        let weather = Hash::of(b"weather-schema-v1");
        let mut by_hash: HashMap<Hash, std::sync::Arc<str>> = HashMap::new();
        by_hash.insert(weather, "weather".into());
        let resolver = MapResolver { by_hash };
        let dyn_resolver: &dyn ResolveSchema = &resolver;

        // A known schema-hash resolves to its declared name (the stable routing/Cedar key).
        assert_eq!(
            poll_ready(dyn_resolver.resolve_declared_name(weather)).as_deref(),
            Some("weather")
        );
        // An unknown hash resolves to None — the fail-closed answer (no route, no grant).
        assert_eq!(
            poll_ready(dyn_resolver.resolve_declared_name(Hash::of(b"unknown"))),
            None
        );
    }

    #[test]
    fn inert_resolver_resolves_everything_to_none_fail_closed() {
        let dyn_resolver: &dyn ResolveSchema = &InertSchemaResolver;
        assert_eq!(
            poll_ready(dyn_resolver.resolve_declared_name(Hash::of(b"anything"))),
            None
        );
    }
}

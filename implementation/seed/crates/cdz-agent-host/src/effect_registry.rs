//! Host-side read helpers over the canonical userspace-effect registry (design DESIGN-userspace-effects). A
//! handler session CLAIMS an effect family by setting an `effect/<family>` pointer in the canonical
//! [`NameStore`](cdz_kernel::name_store::NameStore) at its own `SessionId` (= its genesis hash — the store
//! holds the `Hash`, the host addresses the session by that hash's hex). These are the two pure lookups the
//! host's userspace-effect wiring reads off that registry:
//!
//! - [`resolve_handler_session`] — family -> the handler [`SessionId`] registered for it. The primitive the
//!   I3 delegating executor's LIVE `HandlerResolver` (reading the host-owned canonical store) is built on:
//!   "is a handler registered for this family, and which session is it?"
//! - [`effect_families_owned_by`] — the inverse: every family a given handler session currently owns. The
//!   primitive the I6 terminate-prune reads to drop a terminating handler's registrations (so a later request
//!   in one of its families resolves to nothing + falls through rather than routing to a dead session).
//!
//! Both are PURE reads over the landed public `NameStore` surface ([`resolve_effect_handler`] +
//! [`to_set_entries`]) — no kernel edit, no interior mutability. They live host-side (not in the kernel)
//! because the `Hash`->`SessionId` (genesis-hash-hex) addressing is the HOST's identity convention; the
//! kernel store speaks `Hash`. Keeping them as free functions over `&NameStore` means the I3 live resolver
//! and the I6 prune share ONE registry-read definition rather than each re-deriving the `effect/`-prefix +
//! hash->hex mapping.

use crate::host::SessionId;
use cdz_kernel::effect::effect_ct;
use cdz_kernel::name_store::NameStore;

/// Resolve a userspace-effect `family` to the handler [`SessionId`] registered for it in `canonical`, or
/// `None` if no handler has claimed it. `family` is the bare family (`weather`), NOT the `effect/<family>`
/// store-name — [`NameStore::resolve_effect_handler`] prepends the prefix. The registered handler is stored
/// as a genesis [`Hash`](cdz_kernel::hash::Hash); a session's id IS its genesis-hash-hex (the host's identity
/// convention, precedent `child_ids` in [`host`](crate::host)), so this maps hash -> hex -> [`SessionId`].
///
/// This is the LIVE `HandlerResolver` primitive: the I3 delegating executor decides `handles_family` +
/// routes a forwarded request by calling this against the host-owned canonical store (which reflects handler
/// registrations as they happen — distinct from an executor's spawn-time by-value store copy).
pub fn resolve_handler_session(canonical: &NameStore, family: &str) -> Option<SessionId> {
    canonical
        .resolve_effect_handler(family)
        .map(|h| SessionId::new(h.to_hex()))
}

/// Every userspace-effect family currently registered to `handler` in `canonical` — the inverse of
/// [`resolve_handler_session`]. Scans the `effect/<family>` registration namespace for pointers at
/// `handler`'s genesis hash (matched via the SessionId = genesis-hash-hex convention) and returns each bare
/// family (the `effect/` prefix stripped). Empty if the session owns no families (or isn't a handler).
///
/// The I6 terminate-prune primitive: when a handler session terminates, the host reads this to know which
/// `effect/<family>` registrations to drop from the canonical store, so a later request in one of those
/// families resolves to `None` + falls through to the built-in partitions rather than forwarding to a dead
/// session. Pure over [`NameStore::to_set_entries`] (the full name->hash set); the prune WRITE (removing each
/// registration) is a separate loop-side step that owns the mutation.
pub fn effect_families_owned_by(canonical: &NameStore, handler: &SessionId) -> Vec<String> {
    let prefix = effect_ct::EFFECT_REGISTRY_PREFIX;
    canonical
        .to_set_entries()
        .into_iter()
        .filter_map(|(name, hash)| {
            // A registration is `effect/<family>` pointing at the handler's genesis hash. Match the pointer
            // by the SessionId = genesis-hash-hex convention (the hash's hex IS the handler's id), and strip
            // the `effect/` prefix to yield the bare family. A non-`effect/` name or a pointer at a different
            // session is not this handler's registration.
            let family = name.strip_prefix(prefix)?;
            if hash.to_hex() == handler.as_str() {
                Some(family.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// The LIVE [`HandlerResolver`](crate::userspace_effect_exec::HandlerResolver) for the userspace-effect
/// fallback executor: wraps the host's shared canonical [`SharedNameStore`](crate::host::SharedNameStore)
/// handle and resolves `effect/<family>` off it at PERFORM time (v-agent-harness ruling A). Because it holds
/// a CLONE of the host's `Rc<RefCell<NameStore>>` (not a spawn-time by-value copy), a handler a peer
/// registered THIS turn is visible immediately — the property the by-value session store couldn't give. Each
/// resolve takes a short `.borrow()`; there is no `.await` across it (resolution is a synchronous store read),
/// so the RefCell borrow is always released before any await in the caller.
pub struct CanonicalResolver {
    canonical: crate::host::SharedNameStore,
}

impl CanonicalResolver {
    /// Build the resolver over a clone of the host's shared canonical-store handle (from
    /// [`AgentHost::shared_canonical_store`](crate::host::AgentHost::shared_canonical_store)).
    pub fn new(canonical: crate::host::SharedNameStore) -> Self {
        CanonicalResolver { canonical }
    }
}

impl crate::userspace_effect_exec::HandlerResolver for CanonicalResolver {
    fn resolve_handler(&self, family: &str) -> Option<SessionId> {
        resolve_handler_session(&self.canonical.borrow(), family)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::hash::Hash;

    /// A NameStore with `effect/<family>` -> `handler-hash` registrations applied as set-events, plus any
    /// extra non-effect names, mirroring how a live canonical store accretes handler claims. Built by
    /// replaying set-entries so the store is in the same shape `resolve_effect_handler` reads.
    fn store_with(regs: &[(&str, Hash)], other: &[(&str, Hash)]) -> NameStore {
        let mut entries: Vec<(String, Hash)> = Vec::new();
        for (family, h) in regs {
            entries.push((
                format!("{}{}", effect_ct::EFFECT_REGISTRY_PREFIX, family),
                *h,
            ));
        }
        for (name, h) in other {
            entries.push((name.to_string(), *h));
        }
        NameStore::replay_set_entries(entries.iter().map(|(n, h)| (n.as_str(), *h)))
            .expect("scoped names replay total")
    }

    /// The SessionId a handler genesis hash addresses (id = genesis-hash-hex).
    fn sid_of(h: Hash) -> SessionId {
        SessionId::new(h.to_hex())
    }

    #[test]
    fn resolve_handler_session_maps_a_registered_family_to_its_handler_id() {
        let weather = Hash::of(b"weather-handler-genesis");
        let store = store_with(&[("weather", weather)], &[]);
        assert_eq!(
            resolve_handler_session(&store, "weather"),
            Some(sid_of(weather)),
            "a registered family resolves to the handler SessionId (= its genesis-hash-hex)"
        );
        assert_eq!(
            resolve_handler_session(&store, "stocks"),
            None,
            "an unregistered family resolves to None (falls through to built-ins)"
        );
    }

    #[test]
    fn effect_families_owned_by_returns_all_that_handlers_families_only() {
        let h = Hash::of(b"multi-family-handler");
        let other = Hash::of(b"another-handler");
        // `h` owns weather + geo; `other` owns stocks; plus an unrelated non-effect (but scoped, so
        // writable) name to prove the scan filters names outside the `effect/` namespace.
        let store = store_with(
            &[("weather", h), ("geo", h), ("stocks", other)],
            &[("session/some-session/kv", Hash::of(b"a-value"))],
        );
        let mut owned = effect_families_owned_by(&store, &sid_of(h));
        owned.sort();
        assert_eq!(
            owned,
            vec!["geo".to_string(), "weather".to_string()],
            "only the families registered to this handler, bare (effect/ prefix stripped)"
        );
        // The other handler's families + non-effect names are excluded.
        assert_eq!(
            effect_families_owned_by(&store, &sid_of(other)),
            vec!["stocks".to_string()],
            "the inverse is per-handler: another handler's families are its own"
        );
    }

    #[test]
    fn effect_families_owned_by_is_empty_for_a_non_handler_session() {
        let h = Hash::of(b"a-handler");
        let store = store_with(&[("weather", h)], &[]);
        // A session that has registered nothing owns no families.
        let stranger = SessionId::new("never-registered-anything");
        assert!(
            effect_families_owned_by(&store, &stranger).is_empty(),
            "a session with no registrations owns no effect families"
        );
    }

    #[test]
    fn resolve_and_owned_are_consistent_inverses() {
        let h = Hash::of(b"handler");
        let store = store_with(&[("weather", h), ("tides", h)], &[]);
        // Every family `owned_by` reports resolves back to the same handler.
        for family in effect_families_owned_by(&store, &sid_of(h)) {
            assert_eq!(
                resolve_handler_session(&store, &family),
                Some(sid_of(h)),
                "a family owned_by a handler resolves back to that handler"
            );
        }
    }

    #[test]
    fn an_empty_store_has_no_registrations() {
        let store = NameStore::new();
        assert_eq!(resolve_handler_session(&store, "weather"), None);
        assert!(effect_families_owned_by(&store, &SessionId::new("h")).is_empty());
    }

    #[test]
    fn canonical_resolver_sees_a_registration_written_after_construction_live() {
        use crate::userspace_effect_exec::HandlerResolver;
        use cdz_kernel::name_store::SetEntry;
        use std::cell::RefCell;
        use std::rc::Rc;

        // The crux of ruling A: the resolver holds the SHARED store handle, so a handler registered LATER
        // (by a peer, this turn) is visible immediately — NOT a spawn-time snapshot. Build the resolver over
        // an EMPTY shared store, THEN register effect/weather, THEN resolve → it must see the fresh handler.
        let shared: crate::host::SharedNameStore = Rc::new(RefCell::new(NameStore::new()));
        let resolver = CanonicalResolver::new(shared.clone());

        // Nothing registered yet → unresolved.
        assert_eq!(
            resolver.resolve_handler("weather"),
            None,
            "no handler before registration"
        );

        // A peer registers effect/weather AFTER the resolver was constructed (writing through the SAME Rc).
        let handler = Hash::of(b"weather-handler-genesis");
        shared
            .borrow_mut()
            .set(
                &format!("{}{}", effect_ct::EFFECT_REGISTRY_PREFIX, "weather"),
                SetEntry::unsigned(handler),
            )
            .expect("effect/ is a scoped writable prefix");

        // The resolver sees it LIVE (proves it reads the shared store, not a captured copy).
        assert_eq!(
            resolver.resolve_handler("weather"),
            Some(SessionId::new(handler.to_hex())),
            "the resolver resolves a registration written after its construction (live shared-store read)"
        );
        // An unregistered family still resolves to None (falls through to the built-in path).
        assert_eq!(resolver.resolve_handler("stocks"), None);
    }
}

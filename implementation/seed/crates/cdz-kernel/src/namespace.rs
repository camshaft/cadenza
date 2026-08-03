//! Mutable-name namespaces — the WRITE-AUTHORITY half of the global store's anti-hijack model
//! (design §4c). The content-addressed blob store ([`crate::blob`]) is the immutable half: the hash IS
//! the authorization, no write-auth needed. A **mutable name → hash pointer** is the ONLY thing needing
//! write control (the compiler-hijack surface: repoint `system/compiler/latest` at an evil hash). §4c's
//! rule: **the name's PREFIX determines who may `set` it.** This module is the prefix→authority parse —
//! the signature-independent foundation the signed `set(name, hash)` log builds on.
//!
//! This deliberately carries NO cryptography: it answers "which authority governs this name" (a total,
//! pure classification of the name string), ORTHOGONAL to "is a given `set` actually signed by that
//! authority" (the signing layer, which rides the event envelope's producer/signature — §10, sequenced
//! next). Keeping the two apart means the pending key-management decision can't rework this parse.
//!
//! Namespaces (§4c point 2), by prefix:
//! - `system/…` — only a system/release authority may set (e.g. `system/compiler/latest`); a random
//!   agent's set is REJECTED (injection fails).
//! - `team/<team>/…` — team membership governs.
//! - `session/<id>/…` — owned by that session (its own delegated identity).
//! - `memory/…` — the memory-promotion authority (§9 graduation gate).
//! - anything else — UNSCOPED: no known authority owns the prefix, so no one may set it (fail-closed —
//!   an unrecognized namespace is never writable rather than defaulting open).

/// Which authority governs writes to a mutable name — the parse of its PREFIX (§4c point 2). This names
/// the authority ABSTRACTLY (who, structurally); whether a concrete `set` is signed by a key holding that
/// authority is the signing layer's job, checked separately against the event's producer identity.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NameAuthority {
    /// `system/…` — the operator/release root. The highest-trust namespace (compiler pointer, kernel
    /// bootstrap pointers). Only a system-rooted grant may set.
    System,
    /// `team/<team>/…` — governed by membership in the named team.
    Team(std::sync::Arc<str>),
    /// `session/<id>/…` — owned by the session with that id (writes via its own delegated identity).
    Session(std::sync::Arc<str>),
    /// `memory/…` — the memory-promotion authority (a promotion is an authorized set to a `memory/*`
    /// name, §9f/§9g). Distinct from `system` so memory graduation and release pointers have separate
    /// authorities.
    Memory,
    /// No recognized authority owns this name's prefix — FAIL-CLOSED: nobody may set it. An unknown
    /// namespace is never writable (an unrecognized prefix must not default to writable, or the whole
    /// anti-hijack posture inverts).
    Unscoped,
}

/// The `system/` namespace prefix — the operator/release root authority.
pub const SYSTEM_PREFIX: &str = "system/";
/// The `team/<team>/` namespace prefix.
pub const TEAM_PREFIX: &str = "team/";
/// The `session/<id>/` namespace prefix.
pub const SESSION_PREFIX: &str = "session/";
/// The `memory/` namespace prefix — the memory-promotion authority.
pub const MEMORY_PREFIX: &str = "memory/";

/// Parse a mutable name into the [`NameAuthority`] that governs writes to it (§4c point 2). Total + pure
/// — every string maps to some authority (unknown prefixes → [`NameAuthority::Unscoped`], fail-closed).
/// For `team/<team>/…` and `session/<id>/…` the SECOND path segment names the specific team/session; a
/// bare `team/` or `session/` with no second segment is `Unscoped` (there is no team/session to own it).
pub fn authority_of(name: &str) -> NameAuthority {
    if let Some(rest) = name.strip_prefix(SYSTEM_PREFIX) {
        // `system/` with anything after it is system-governed; a bare `system/` (empty rest) still needs
        // the system authority (there is no sub-owner), so System covers it.
        let _ = rest;
        return NameAuthority::System;
    }
    if let Some(rest) = name.strip_prefix(TEAM_PREFIX) {
        // The team is the first segment of the remainder; empty → no team owns it (Unscoped).
        return match first_segment(rest) {
            Some(team) => NameAuthority::Team(team.into()),
            None => NameAuthority::Unscoped,
        };
    }
    if let Some(rest) = name.strip_prefix(SESSION_PREFIX) {
        return match first_segment(rest) {
            Some(id) => NameAuthority::Session(id.into()),
            None => NameAuthority::Unscoped,
        };
    }
    if name.strip_prefix(MEMORY_PREFIX).is_some() {
        return NameAuthority::Memory;
    }
    NameAuthority::Unscoped
}

/// The first `/`-delimited segment of `s`, or `None` if `s` is empty or starts with `/` (no segment).
fn first_segment(s: &str) -> Option<&str> {
    let seg = s.split('/').next().unwrap_or("");
    if seg.is_empty() {
        None
    } else {
        Some(seg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_names_are_system_governed() {
        assert_eq!(
            authority_of("system/compiler/latest"),
            NameAuthority::System
        );
        assert_eq!(
            authority_of("system/reducer/bootstrap"),
            NameAuthority::System
        );
        // A bare `system/` is still system-governed (no sub-owner).
        assert_eq!(authority_of("system/"), NameAuthority::System);
    }

    #[test]
    fn team_and_session_take_their_owner_from_the_second_segment() {
        assert_eq!(
            authority_of("team/rust-backend/gate-baseline"),
            NameAuthority::Team("rust-backend".into())
        );
        assert_eq!(
            authority_of("session/abc123/scratch"),
            NameAuthority::Session("abc123".into())
        );
        // The owner is exactly the first segment, not the whole remainder.
        assert_eq!(
            authority_of("session/s1/deep/key"),
            NameAuthority::Session("s1".into())
        );
    }

    #[test]
    fn memory_names_are_memory_governed() {
        assert_eq!(
            authority_of("memory/rust-traps/stale-store"),
            NameAuthority::Memory
        );
    }

    #[test]
    fn unknown_prefixes_and_bare_scoped_prefixes_are_unscoped_fail_closed() {
        // No recognized authority → Unscoped (nobody may set — fail-closed).
        assert_eq!(authority_of("compiler-latest"), NameAuthority::Unscoped);
        assert_eq!(authority_of("random/name"), NameAuthority::Unscoped);
        assert_eq!(authority_of(""), NameAuthority::Unscoped);
        // A scoped prefix with NO owner segment is Unscoped (there is no team/session to own it).
        assert_eq!(authority_of("team/"), NameAuthority::Unscoped);
        assert_eq!(authority_of("session/"), NameAuthority::Unscoped);
    }
}

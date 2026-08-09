//! The reply-token registry — the shared host-side core of the userspace-effects reply path (design
//! DESIGN-userspace-effects I3/I4). When a `UserspaceEffectExecutor` (I3) FORWARDS an effect-request to a
//! handler session, it MINTS a per-forward reply-token bound to `(caller SessionId, EffectId)` and threads it
//! in the forwarded Inbound's framing. When the handler answers via an `effect/reply` effect (I4), the host
//! looks the token up here to recover WHICH `(caller, effect-id)` to settle, then `settle_effect_result`s the
//! caller's pending effect. This module owns ONLY the token table + its mint / one-shot validate-consume — the
//! I3 forward-plumbing + the I4 loop-side settle are separate slices that USE it (kept split so the token
//! security core is hermetically testable with no loop / no kernel session, like `ws_socket` splits the
//! registry from the live listener).
//!
//! **The token is a CAPABILITY, not a lookup key (reply-forgery defense, design §12c).** Possession of a valid
//! reply-token IS the authority to settle exactly the `(caller, effect-id)` it was minted for — a handler
//! cannot forge a reply to any OTHER session/effect (confused-deputy defense). So the token is:
//! - UNGUESSABLE: minted from OS entropy hashed into a [`Hash`] (the same content-addressed identity scheme as
//!   session ids + ws conn-ids — the operator's unification: every host-managed handle is a `Hash`), so a
//!   handler can't fabricate a token for a `(caller, effect-id)` it was never handed.
//! - ONE-SHOT: consumed on the first valid reply, so a handler can't settle the same effect twice (a second
//!   reply with the same token finds nothing → refused). At-most-once settle is ALSO enforced by the kernel
//!   (`settle_effect_result` is idempotent on an already-settled id), but consuming here refuses the duplicate
//!   BEFORE it reaches the kernel + closes the token so it can't be replayed against a re-used effect-id.
//!
//! **THIN MECHANISM (host is INEVOLVABLE).** This carries no policy — it maps a token to its bound identity +
//! enforces unguessable/one-shot. WHICH families have handlers (I1) + WHETHER a handler may serve a caller
//! (authz) are decided elsewhere; this only makes the reply routable + unforgeable.

use crate::host::SessionId;
use cdz_kernel::effect::EffectId;
use cdz_kernel::hash::Hash;
use std::cell::RefCell;
use std::collections::HashMap;

/// The `(caller, effect-id)` a reply-token authorizes settling — recovered when a handler replies so the host
/// `settle_effect_result`s the RIGHT pending effect on the RIGHT caller session.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReplyTarget {
    /// The session whose pending (deferred) effect this reply settles.
    pub caller: SessionId,
    /// The caller's open effect id — the settle key `settle_effect_result` folds the outcome onto.
    pub effect_id: EffectId,
}

/// The host-side reply-token table: the token [`Hash`] -> the `(caller, effect-id)` it settles. Lives on the
/// single-threaded host loop (`RefCell`, not a `Mutex` — mint on I3 forward + consume on I4 reply are both on
/// the one loop thread). One-shot: a token is REMOVED on the first valid consume. Keyed by the BINARY `Hash`
/// (a `Copy` [u8;32], cheaply clonable — the operator's binary-everywhere/cheaply-clonable rule: NO hex string
/// on the identity/routing/storage path; hex is for LOGGING only). The token rides the `effect/reply`
/// `req.target` (`Arc<[u8]>`) as its RAW 32 bytes — the guest echoes the bytes verbatim, `validate_and_consume`
/// reconstitutes the `Hash` from them; zero hex anywhere (operator zero-hex directive).
#[derive(Default)]
pub struct ReplyTokenRegistry {
    tokens: RefCell<HashMap<Hash, ReplyTarget>>,
}

impl ReplyTokenRegistry {
    /// A fresh empty registry.
    pub fn new() -> Self {
        ReplyTokenRegistry {
            tokens: RefCell::new(HashMap::new()),
        }
    }

    /// Mint a fresh reply-token for a forwarded effect-request, bound to `(caller, effect_id)`. Returns the
    /// token as a [`Hash`] (its RAW bytes are what the I3 forward threads into the handler's Inbound framing +
    /// the handler echoes as the `effect/reply` target). The token is UNGUESSABLE: 32 OS-random bytes hashed into
    /// a `Hash` (mirrors `mint_spawn_nonce` / `mint_conn_id` — one identity scheme for every host handle), so
    /// a handler can't fabricate a token for a binding it was never given. `getrandom` failing is unsurvivable
    /// (no entropy = can't mint an unforgeable token), so it is a hard error, not a weak-token fallback.
    pub fn mint(&self, caller: SessionId, effect_id: EffectId) -> Hash {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).expect("OS entropy (getrandom) for an effect reply-token");
        let token = Hash::of(&bytes);
        self.tokens
            .borrow_mut()
            .insert(token, ReplyTarget { caller, effect_id });
        token
    }

    /// Validate a reply-token (the RAW 32 token bytes a handler echoed on its `effect/reply` `req.target`,
    /// which is `Arc<[u8]>`) and CONSUME it one-shot: reconstitute the token [`Hash`] from the bytes, return
    /// the bound [`ReplyTarget`], and remove the token so a second reply with the same token finds nothing.
    /// `None` = the bytes aren't a 32-byte token, unknown (never minted / forged), OR already consumed (a
    /// duplicate/replayed reply) — either way the reply is refused (the host does NOT settle anything). This
    /// is the reply-forgery and double-settle defense, enforced BEFORE the kernel `settle_effect_result`.
    /// Takes RAW bytes (no hex): the token rides the effect-target as opaque `Arc<[u8]>` end to end (operator
    /// zero-hex directive — the token was always a binary `Hash`, the hex round-trip was gratuitous).
    pub fn validate_and_consume(&self, token_bytes: &[u8]) -> Option<ReplyTarget> {
        let raw = <[u8; 32]>::try_from(token_bytes).ok()?;
        self.tokens.borrow_mut().remove(&Hash::from_bytes(raw))
    }

    /// Drop all tokens bound to `caller` (their pending effects can no longer be replied to) — the I6
    /// terminate-prune path calls this when a CALLER session terminates, so a later handler reply to a dead
    /// caller's effect finds no token + is refused (rather than attempting a settle on a terminated session,
    /// which the kernel would no-op anyway — this just prunes the table so it doesn't leak entries for gone
    /// callers). Returns the number of tokens dropped (for observability / tests).
    pub fn drop_caller(&self, caller: &SessionId) -> usize {
        let mut tokens = self.tokens.borrow_mut();
        let before = tokens.len();
        tokens.retain(|_, t| &t.caller != caller);
        before - tokens.len()
    }

    /// The count of outstanding (minted, not-yet-consumed) tokens — for status/metrics + tests.
    pub fn len(&self) -> usize {
        self.tokens.borrow().len()
    }

    /// Whether there are no outstanding tokens.
    pub fn is_empty(&self) -> bool {
        self.tokens.borrow().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller(s: &str) -> SessionId {
        SessionId::new(Hash::of(s.as_bytes()))
    }

    #[test]
    fn mint_then_validate_consume_recovers_the_binding_one_shot() {
        let reg = ReplyTokenRegistry::new();
        let token = reg.mint(caller("outpost"), EffectId(7));
        assert_eq!(reg.len(), 1);
        // First reply with the token recovers the exact (caller, effect-id) binding.
        let target = reg
            .validate_and_consume(token.as_bytes())
            .expect("a freshly-minted token validates");
        assert_eq!(target.caller, caller("outpost"));
        assert_eq!(target.effect_id, EffectId(7));
        // One-shot: the token is now consumed, so a SECOND reply with it is refused (double-settle defense).
        assert!(reg.is_empty());
        assert!(
            reg.validate_and_consume(token.as_bytes()).is_none(),
            "a consumed reply-token cannot settle a second time"
        );
    }

    #[test]
    fn an_unknown_or_forged_token_is_refused() {
        let reg = ReplyTokenRegistry::new();
        reg.mint(caller("a"), EffectId(1));
        // A token that was never minted (a handler fabricating a reply) recovers nothing → refused.
        let forged = Hash::of(b"not-a-real-token");
        assert!(
            reg.validate_and_consume(forged.as_bytes()).is_none(),
            "a forged/never-minted token cannot settle any effect (reply-forgery defense)"
        );
        // The real token is untouched by the forged attempt.
        assert_eq!(reg.len(), 1);
        // A non-hex / malformed token (a handler echoed garbage on effect/reply's target) is refused too —
        // it parses to no Hash, so no settle (never panics on a bad boundary string).
        assert!(reg.validate_and_consume(b"not-32-bytes").is_none());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn distinct_mints_are_unguessable_and_independent() {
        let reg = ReplyTokenRegistry::new();
        let t1 = reg.mint(caller("a"), EffectId(1));
        let t2 = reg.mint(caller("a"), EffectId(2));
        // Two forwards (even same caller) get different unguessable tokens.
        assert_ne!(t1, t2, "each forward mints a fresh unguessable reply-token");
        // Consuming one leaves the other valid (independent bindings).
        assert!(reg.validate_and_consume(t1.as_bytes()).is_some());
        let left = reg
            .validate_and_consume(t2.as_bytes())
            .expect("the other token still validates");
        assert_eq!(left.effect_id, EffectId(2));
    }

    #[test]
    fn drop_caller_prunes_all_that_callers_tokens_only() {
        let reg = ReplyTokenRegistry::new();
        reg.mint(caller("gone"), EffectId(1));
        reg.mint(caller("gone"), EffectId(2));
        let keep = reg.mint(caller("alive"), EffectId(3));
        // Terminating "gone" drops both its tokens, leaves "alive"'s.
        assert_eq!(reg.drop_caller(&caller("gone")), 2);
        assert_eq!(reg.len(), 1);
        assert!(
            reg.validate_and_consume(keep.as_bytes()).is_some(),
            "an unrelated caller's token survives the prune"
        );
    }
}

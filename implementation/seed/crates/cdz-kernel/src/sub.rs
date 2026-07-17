//! Subscriptions over the log (agent-runtime L3a) — the one reactive primitive (vision §8).
//!
//! Vision §8/§15 rung 3: *everything reactive is a subscription.* A [`Subscription`] is
//! `{id, predicate, program_ref, capability}` — a durable log event (`kind == "subscribe"`) that says
//! "when an event matching `predicate` lands, schedule `program_ref` under `capability`". The owner's fold
//! dispatches them (an event lands → which active subscriptions' predicates match → those handler programs
//! are scheduled), so the agent loop ("wake me on messages addressed to me"), reporters, auto-compaction,
//! and the compute router are all *the same primitive* — no separate daemon/poller.
//!
//! **L3a is the type + its PREDICATE + its ENCODING** (this module): a [`Subscription`] with a concrete,
//! matchable [`Predicate`] (NOT a general expression language — that's a later rung, vision "open leaf-
//! level"), a pure [`Predicate::matches`] over an [`crate::Event`], and a dependency-free binary-safe codec
//! to/from the event payload (the same length-prefixed discipline as `msg`/`file_log`/`dynamo_log`). So it
//! is unit-tested with no log/network. L3b folds `subscribe` events into the active-subscription set; L3c
//! dispatches a landed event against them; L3d recasts the agent loop as a `MessageTo(me)` subscription.

use crate::Event;
use anyhow::{anyhow, Result};

/// The event `kind` tag for a subscription event (a [`Subscription`] in its payload).
pub const SUBSCRIBE: &str = "subscribe";
/// The event `kind` tag for an unsubscribe event (its payload is the revoked subscription `id`).
pub const UNSUBSCRIBE: &str = "unsubscribe";

/// A matchable predicate over the event stream — the "when" of a subscription. Deliberately a small set of
/// CONCRETE variants for L3a (mirrors how L2's inbox is just a `to == agent` filter): a general predicate
/// EXPRESSION language (semantic match, boolean combinators) is a later rung. Each variant is a pure
/// [`Predicate::matches`] test against a single landed [`Event`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Predicate {
    /// Matches a `message` event addressed `to` this agent — i.e. exactly what [`crate::msg::inbox_for`]
    /// surfaces (L3d proves the agent loop IS this subscription). Requires decoding the message payload.
    MessageTo(String),
    /// Matches any event whose `kind` equals this tag (e.g. `"model-response"`, `"merged"`) — the coarse
    /// "wake me on events of this kind" predicate, no payload decode needed.
    EventKind(String),
}

impl Predicate {
    /// Whether `event` satisfies this predicate (pure). `MessageTo(agent)` decodes the message payload and
    /// tests `to == agent` (a non-`message` event, or one whose payload fails to decode, does NOT match —
    /// a corrupt event never spuriously fires a handler); `EventKind(k)` tests `event.kind == k`.
    pub fn matches(&self, event: &Event) -> bool {
        match self {
            Predicate::MessageTo(agent) => {
                event.kind == crate::msg::MESSAGE
                    && crate::msg::Message::decode(&event.payload).is_ok_and(|m| &m.to == agent)
            }
            Predicate::EventKind(kind) => &event.kind == kind,
        }
    }

    /// The 1-byte discriminant tag for the wire encoding (kept explicit so a new variant is a deliberate,
    /// reviewable choice of tag, never an accidental reorder-induced shift).
    fn tag(&self) -> u8 {
        match self {
            Predicate::MessageTo(_) => 0,
            Predicate::EventKind(_) => 1,
        }
    }
}

/// A subscription — the reactive primitive as a durable log event (vision §8). `id` is a stable handle (a
/// later `unsubscribe`/superseding event references it — L3b); `predicate` is the "when"; `program_ref`
/// names the handler Cadenza program (an opaque ref — resolving + RUNNING it is L4/L5, the fold owner +
/// capabilities); `capability` is the effect-type the handler is authorized for (capability = effect-type,
/// vision §? — carried here, enforced later). Rides the L1 [`Event`] as a `subscribe` event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscription {
    pub id: String,
    pub predicate: Predicate,
    pub program_ref: String,
    pub capability: String,
}

impl Subscription {
    /// Encode to the [`Event`] payload bytes: `id` (length-prefixed), then the predicate (a 1-byte tag +
    /// its single length-prefixed string operand), then `program_ref` + `capability` (length-prefixed).
    /// Binary-safe + dependency-free — the inverse of [`Subscription::decode`].
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_str(&mut out, &self.id);
        out.push(self.predicate.tag());
        match &self.predicate {
            Predicate::MessageTo(s) | Predicate::EventKind(s) => put_str(&mut out, s),
        }
        put_str(&mut out, &self.program_ref);
        put_str(&mut out, &self.capability);
        out
    }

    /// Decode a [`Subscription`] from a payload produced by [`Subscription::encode`]. Errors on a truncated
    /// / malformed payload or an unknown predicate tag (the log is the source of truth — a bad decode is
    /// loud, not a silent default).
    pub fn decode(bytes: &[u8]) -> Result<Subscription> {
        let mut i = 0usize;
        let id = get_str(bytes, &mut i)?;
        let tag = *bytes.get(i).ok_or_else(|| {
            anyhow!("truncated subscription: expected a predicate tag at offset {i}")
        })?;
        i += 1;
        let operand = get_str(bytes, &mut i)?;
        let predicate = match tag {
            0 => Predicate::MessageTo(operand),
            1 => Predicate::EventKind(operand),
            other => return Err(anyhow!("unknown predicate tag {other}")),
        };
        let program_ref = get_str(bytes, &mut i)?;
        let capability = get_str(bytes, &mut i)?;
        if i != bytes.len() {
            return Err(anyhow!(
                "subscription payload has {} trailing bytes after decode",
                bytes.len() - i
            ));
        }
        Ok(Subscription {
            id,
            predicate,
            program_ref,
            capability,
        })
    }
}

// ── length-prefixed codec helpers (u32-LE lengths; binary-safe; no deps) ────────────────────────────────
// Same discipline as `msg`/`file_log`/`dynamo_log`; kept module-local so `sub` is self-contained.

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn get_u32(bytes: &[u8], i: &mut usize) -> Result<u32> {
    let end = *i + 4;
    let slice = bytes
        .get(*i..end)
        .ok_or_else(|| anyhow!("truncated subscription: expected a 4-byte length at offset {i}"))?;
    let v = u32::from_le_bytes(slice.try_into().expect("slice is exactly 4 bytes"));
    *i = end;
    Ok(v)
}

fn get_str(bytes: &[u8], i: &mut usize) -> Result<String> {
    let len = get_u32(bytes, i)? as usize;
    let end = *i + len;
    let slice = bytes
        .get(*i..end)
        .ok_or_else(|| anyhow!("truncated subscription: expected {len} bytes at offset {i}"))?;
    *i = end;
    String::from_utf8(slice.to_vec())
        .map_err(|e| anyhow!("subscription field not valid UTF-8: {e}"))
}

/// The ACTIVE subscriptions as a PROJECTION over the log (agent-runtime L3b) — a fold, mirroring L2b's
/// [`crate::msg::inbox_for`]. Folds `events`: each `subscribe` event registers (or, by the same `id`,
/// REPLACES — supersession) a [`Subscription`]; each `unsubscribe` event revokes the subscription with that
/// `id`. Returns the live set paired with the `seq` of the *latest* `subscribe` that defined each (the seq a
/// supersession/dispatch correlates against), in ascending order of that seq. A `subscribe` event whose
/// payload fails to decode is skipped (the projection stays readable — same discipline as the inbox fold).
///
/// Supersession-by-id + explicit unsubscribe are the *only* revocation for L3b (vision "open leaf-level":
/// richer lifecycle — TTLs, one-shot subscriptions — is a later rung). This is "the active reactive set is a
/// fold over the log", the L3 counterpart of "the inbox is a fold".
pub fn active_subscriptions(events: &[Event]) -> Vec<(crate::Seq, Subscription)> {
    // A small insertion-ordered map keyed by subscription id: later `subscribe`s replace earlier ones in
    // place (supersession), `unsubscribe` removes, and we keep the latest defining seq. A Vec keeps it
    // dependency-free + preserves first-seen order deterministically (the set is small — one entry per live
    // subscription, not per event).
    let mut live: Vec<(crate::Seq, Subscription)> = Vec::new();
    for e in events {
        match e.kind.as_str() {
            SUBSCRIBE => {
                let Ok(s) = Subscription::decode(&e.payload) else {
                    continue; // corrupt subscribe event — skip, keep the projection readable
                };
                if let Some(slot) = live.iter_mut().find(|(_, cur)| cur.id == s.id) {
                    *slot = (e.seq, s); // supersede: replace the prior definition + its seq
                } else {
                    live.push((e.seq, s));
                }
            }
            UNSUBSCRIBE => {
                if let Ok(id) = String::from_utf8(e.payload.clone()) {
                    live.retain(|(_, cur)| cur.id != id);
                }
            }
            _ => {}
        }
    }
    // Return in ascending order of the latest defining seq (a superseded sub sorts by its NEW seq).
    live.sort_by_key(|(seq, _)| *seq);
    live
}

/// DISPATCH a newly-landed event against the active subscriptions (agent-runtime L3c) — the core reactive
/// step: an event lands → which active subscriptions' predicates [`Predicate::matches`] it → those are the
/// handler programs to schedule. Given the `events` seen so far (the log up to and *including* `new_event`)
/// and the `new_event` that just landed, folds the active set with [`active_subscriptions`] and returns the
/// subset whose predicate matches `new_event`, in the same seq order — the SCHEDULABLE set.
///
/// This is *only* the match/selection step: it returns WHICH subscriptions fire, paired with the seq that
/// defined each (so a handler run can be recorded/correlated). Actually RUNNING a matched handler program
/// under its capability is the fold owner + capability rung (L4/L5) — L3c is pure and deterministic, so the
/// dispatch decision itself is replayable. L3d then shows a `MessageTo(me)` subscription's dispatch reproduces
/// exactly what [`crate::msg::inbox_for`] surfaces, unifying the agent loop with this primitive.
pub fn dispatch(events: &[Event], new_event: &Event) -> Vec<(crate::Seq, Subscription)> {
    active_subscriptions(events)
        .into_iter()
        .filter(|(_, sub)| sub.predicate.matches(new_event))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::{Message, ACK, MESSAGE};

    fn sub(pred: Predicate) -> Subscription {
        Subscription {
            id: "sub-1".into(),
            predicate: pred,
            program_ref: "agent-loop.wasm".into(),
            capability: "model".into(),
        }
    }

    #[test]
    fn subscription_round_trips_both_predicate_variants() {
        for p in [
            Predicate::MessageTo("v-agent-harness".into()),
            Predicate::EventKind("model-response".into()),
        ] {
            let s = sub(p);
            assert_eq!(
                Subscription::decode(&s.encode()).unwrap(),
                s,
                "subscription encode/decode is identity"
            );
        }
    }

    #[test]
    fn subscription_round_trips_empty_and_binary_ish_fields() {
        // Empty operand + empty id/program/capability all survive the length-prefixed codec.
        let s = Subscription {
            id: "".into(),
            predicate: Predicate::EventKind("".into()),
            program_ref: "".into(),
            capability: "".into(),
        };
        assert_eq!(Subscription::decode(&s.encode()).unwrap(), s);
    }

    #[test]
    fn subscribe_tag_is_stable() {
        assert_eq!(SUBSCRIBE, "subscribe");
    }

    #[test]
    fn decode_rejects_truncated_and_unknown_tag() {
        let full = sub(Predicate::MessageTo("me".into())).encode();
        assert!(
            Subscription::decode(&full[..full.len() - 2]).is_err(),
            "a truncated subscription must not decode"
        );
        // Corrupt the predicate tag byte (it sits right after the length-prefixed `id`).
        let mut bad = full.clone();
        let tag_off = 4 + "sub-1".len(); // u32 len prefix + the id bytes
        bad[tag_off] = 99;
        assert!(
            Subscription::decode(&bad).is_err(),
            "an unknown predicate tag must not decode"
        );
    }

    // ── the pure predicate match (the heart of L3c dispatch) ──────────────────────────────────────────

    fn msg_event(seq: crate::Seq, to: &str) -> Event {
        let m = Message {
            from: "sender".into(),
            to: to.into(),
            kind: "note".into(),
            subject: "s".into(),
            refs: vec![],
            body: b"b".to_vec(),
        };
        Event {
            seq,
            kind: MESSAGE.into(),
            payload: m.encode(),
        }
    }

    #[test]
    fn message_to_matches_only_messages_addressed_to_the_agent() {
        let p = Predicate::MessageTo("me".into());
        assert!(p.matches(&msg_event(0, "me")), "a message to me matches");
        assert!(
            !p.matches(&msg_event(1, "someone-else")),
            "a message to another agent does not match"
        );
        // A non-message event never matches MessageTo, even if its kind coincidentally decodes.
        let ack = Event {
            seq: 2,
            kind: ACK.into(),
            payload: crate::msg::Ack { message_seq: 0 }.encode(),
        };
        assert!(
            !p.matches(&ack),
            "a non-message event does not match MessageTo"
        );
    }

    #[test]
    fn message_to_does_not_match_a_corrupt_message_payload() {
        // A message event whose payload can't decode must NOT fire the handler (corrupt ≠ match).
        let mut e = msg_event(0, "me");
        e.payload = vec![0xff, 0xff];
        assert!(
            !Predicate::MessageTo("me".into()).matches(&e),
            "a corrupt message payload does not match"
        );
    }

    #[test]
    fn event_kind_matches_by_exact_kind() {
        let p = Predicate::EventKind("model-response".into());
        let mr = Event {
            seq: 0,
            kind: "model-response".into(),
            payload: vec![],
        };
        let other = Event {
            seq: 1,
            kind: "model-request".into(),
            payload: vec![],
        };
        assert!(p.matches(&mr), "matches the exact kind");
        assert!(!p.matches(&other), "a different kind does not match");
    }

    // ── L3b: the active-subscriptions projection (a fold over the log) ────────────────────────────────

    /// Build a `subscribe` [`Event`] at `seq` for a subscription with `id` + `pred`.
    fn sub_event(seq: crate::Seq, id: &str, pred: Predicate) -> Event {
        let s = Subscription {
            id: id.into(),
            predicate: pred,
            program_ref: "p.wasm".into(),
            capability: "model".into(),
        };
        Event {
            seq,
            kind: SUBSCRIBE.into(),
            payload: s.encode(),
        }
    }

    /// Build an `unsubscribe` [`Event`] at `seq` revoking subscription `id`.
    fn unsub_event(seq: crate::Seq, id: &str) -> Event {
        Event {
            seq,
            kind: UNSUBSCRIBE.into(),
            payload: id.as_bytes().to_vec(),
        }
    }

    #[test]
    fn active_subscriptions_folds_subscribe_events_in_seq_order() {
        let log = vec![
            sub_event(0, "a", Predicate::EventKind("k1".into())),
            sub_event(1, "b", Predicate::MessageTo("me".into())),
        ];
        let active = active_subscriptions(&log);
        assert_eq!(
            active.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
            vec![0, 1],
            "both subscriptions active, ordered by defining seq"
        );
        assert_eq!(active[0].1.id, "a");
        assert_eq!(active[1].1.id, "b");
    }

    #[test]
    fn a_later_subscribe_with_the_same_id_supersedes_the_earlier_one() {
        // Re-subscribing under the same id REPLACES the definition (and its seq) — supersession, no dup.
        let log = vec![
            sub_event(0, "a", Predicate::EventKind("old".into())),
            sub_event(3, "a", Predicate::EventKind("new".into())),
        ];
        let active = active_subscriptions(&log);
        assert_eq!(active.len(), 1, "one live subscription for id `a`, not two");
        assert_eq!(active[0].0, 3, "keyed to the LATEST defining seq");
        assert_eq!(
            active[0].1.predicate,
            Predicate::EventKind("new".into()),
            "the superseding definition wins"
        );
    }

    #[test]
    fn an_unsubscribe_revokes_the_subscription_by_id() {
        let log = vec![
            sub_event(0, "a", Predicate::EventKind("k".into())),
            sub_event(1, "b", Predicate::EventKind("k".into())),
            unsub_event(2, "a"),
        ];
        let active = active_subscriptions(&log);
        assert_eq!(
            active.iter().map(|(_, s)| s.id.clone()).collect::<Vec<_>>(),
            vec!["b".to_string()],
            "the unsubscribed `a` is revoked; `b` stays live"
        );
        // A re-subscribe AFTER an unsubscribe brings it back (the log just grows).
        let mut log2 = log.clone();
        log2.push(sub_event(3, "a", Predicate::EventKind("k2".into())));
        let active2 = active_subscriptions(&log2);
        assert_eq!(
            active2.len(),
            2,
            "re-subscribing after unsubscribe re-adds it"
        );
    }

    #[test]
    fn active_subscriptions_skips_a_corrupt_subscribe_event_but_keeps_the_rest() {
        let mut log = vec![
            sub_event(0, "a", Predicate::EventKind("k".into())),
            sub_event(1, "b", Predicate::EventKind("k".into())),
        ];
        log[0].payload = vec![0x00, 0x01]; // corrupt the first subscribe payload
        let active = active_subscriptions(&log);
        assert_eq!(
            active.iter().map(|(_, s)| s.id.clone()).collect::<Vec<_>>(),
            vec!["b".to_string()],
            "the corrupt subscribe is skipped; the well-formed one still projects"
        );
    }

    #[test]
    fn active_subscriptions_is_empty_for_no_subscribes_or_all_revoked() {
        assert!(active_subscriptions(&[]).is_empty(), "empty log → none");
        let log = vec![
            sub_event(0, "a", Predicate::EventKind("k".into())),
            unsub_event(1, "a"),
        ];
        assert!(
            active_subscriptions(&log).is_empty(),
            "the only subscription is revoked → none active"
        );
        // Non-subscription events (a plain message) don't register anything.
        assert!(
            active_subscriptions(&[msg_event(0, "me")]).is_empty(),
            "a message event registers no subscription"
        );
    }

    #[test]
    fn unsubscribe_tag_is_stable() {
        assert_eq!(UNSUBSCRIBE, "unsubscribe");
    }

    // ── L3c: dispatch a landed event against the active subscriptions (the schedulable set) ───────────

    #[test]
    fn dispatch_returns_only_the_active_subscriptions_whose_predicate_matches() {
        // Two active subs: one on messages-to-me, one on kind "tick". A message-to-me event fires only the
        // former; a "tick" event fires only the latter — dispatch = active set filtered by matches().
        let log = [
            sub_event(0, "inbox", Predicate::MessageTo("me".into())),
            sub_event(1, "ticker", Predicate::EventKind("tick".into())),
        ];
        let msg = msg_event(2, "me");
        let fired = dispatch(&[log[0].clone(), log[1].clone(), msg.clone()], &msg);
        assert_eq!(
            fired.iter().map(|(_, s)| s.id.clone()).collect::<Vec<_>>(),
            vec!["inbox".to_string()],
            "a message-to-me fires only the MessageTo(me) subscription"
        );
        let tick = Event {
            seq: 3,
            kind: "tick".into(),
            payload: vec![],
        };
        let fired2 = dispatch(&[log[0].clone(), log[1].clone(), tick.clone()], &tick);
        assert_eq!(
            fired2.iter().map(|(_, s)| s.id.clone()).collect::<Vec<_>>(),
            vec!["ticker".to_string()],
            "a tick event fires only the EventKind(tick) subscription"
        );
    }

    #[test]
    fn dispatch_ignores_revoked_and_superseded_subscriptions() {
        // A revoked subscription never fires; a superseded one fires by its NEW predicate, not the old.
        let msg = msg_event(9, "me");
        // `a` subscribes to MessageTo(me) then is unsubscribed → must NOT fire on a message to me.
        let revoked_log = vec![
            sub_event(0, "a", Predicate::MessageTo("me".into())),
            unsub_event(1, "a"),
            msg.clone(),
        ];
        assert!(
            dispatch(&revoked_log, &msg).is_empty(),
            "a revoked subscription does not fire"
        );
        // `b` first subscribes to a non-matching kind, then re-subscribes (supersede) to MessageTo(me).
        let superseded_log = vec![
            sub_event(0, "b", Predicate::EventKind("other".into())),
            sub_event(1, "b", Predicate::MessageTo("me".into())),
            msg.clone(),
        ];
        let fired = dispatch(&superseded_log, &msg);
        assert_eq!(
            fired.iter().map(|(_, s)| s.id.clone()).collect::<Vec<_>>(),
            vec!["b".to_string()],
            "the superseding predicate (MessageTo) is what dispatch matches on"
        );
    }

    #[test]
    fn dispatch_is_empty_when_no_active_subscription_matches() {
        let log = [sub_event(0, "a", Predicate::EventKind("tick".into()))];
        let msg = msg_event(1, "me");
        assert!(
            dispatch(&[log[0].clone(), msg.clone()], &msg).is_empty(),
            "no active subscription matches this event → nothing scheduled"
        );
        // No subscriptions at all → nothing fires.
        assert!(
            dispatch(std::slice::from_ref(&msg), &msg).is_empty(),
            "no subscriptions → empty dispatch"
        );
    }
}

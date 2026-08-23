//! The timer primitive (`design/cadenza-platform.md` §6).
//!
//! Time is not an input to a fold; it is an event a reducer receives. `fire-after(duration)` is the ONLY
//! timer primitive: a reducer arms it and the runtime later wakes the session by delivering a [`Fired`] event
//! carrying the recorded fire time. A program never reads a wall-clock during a fold — that would make the
//! same event fold differently tomorrow — so the raw primitive knows only a relative `duration` (in
//! nanoseconds), and the reducer only ever sees the recorded fired time folded into its state. Absolute
//! deadlines and crons are built on top of this raw primitive by the system reducer (read `now`, compute the
//! delay, arm `fire-after`); the deadline enforcement in §4 is one such policy.
//!
//! A native, system-owned reducer answers this contract — the timer is entirely dependent on how the runtime
//! models time, which is unlikely to ever change — so, unlike a guest reducer, it is never a wasm program
//! (operator decision 2026-08-22). But its input and output are ordinary content-addressed values of this
//! contract's schema, marshalled through the one canonical codec ([`cadenza_ast::codec`]) exactly like every
//! other contract; there is no bespoke format.
//!
//! Arming is an ordinary [`Request`](crate::Request), so the wake correlates back to the arming reducer
//! through the request's own standard continuation-token (operator review 2026-08-22) — the value carries no
//! bespoke correlation field, only the `UInt64` it needs: the arm carries the `duration`, the fire carries
//! the recorded `fired_time`. Both are non-negative, so they cross as the value model's arbitrary-precision
//! integer leaf ([`crate::contract_value::uint_leaf`] / [`read_uint`](crate::contract_value::read_uint)),
//! which enforces the non-negative range on decode. Decoding is total: [`FireAfter::decode`] /
//! [`Fired::decode`] return `None` on anything that is not a well-formed value, so a bad value is rejected,
//! never a panic.

use crate::{Bytes, Contract, ContractId, Request};
use cadenza_ast::ast::{Builder, StructId};
use cadenza_ast::codec;
use std::sync::OnceLock;

/// The timer contract: a [`Request`](crate::Request) against it arms a timer (§6). It is a real contract
/// whose id is the hash of its declared schema — the compiler-checked [`crate::contracts::timer`] module
/// generated from `contracts/timer.cdz` — with the arm [`Envelope`](crate::contracts::timer) (`FireAfter`) as
/// its input type and the delivered [`Event`](crate::contracts::timer) (`Fired`) as its output. The contract
/// value is built once and cached, so the id is derived only once.
#[must_use]
pub fn timer_contract() -> ContractId {
    static TIMER: OnceLock<Contract> = OnceLock::new();
    TIMER.get_or_init(crate::contracts::timer::contract).id()
}

/// Arm a timer that fires `duration` nanoseconds from now (§6). This is what the payload of a timer
/// [`Request`](crate::Request) carries; the wake correlates back to the arming reducer through that request's
/// standard continuation-token, so nothing else rides in the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FireAfter {
    /// How far in the future to fire, in nanoseconds (non-negative).
    pub duration: u64,
}

/// The event delivered to the arming reducer when its timer fires (§6): the recorded fire time — the
/// runtime's clock at the moment it fired. The reducer never reads the clock itself; it only folds this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fired {
    /// The runtime clock value recorded at the moment the timer fired, in nanoseconds (non-negative).
    pub fired_time: u64,
}

impl FireAfter {
    /// Build the arm value into `b`, returning its root — a value of the schema type `Envelope`, so it
    /// type-ascribes against the contract's schema. The value SHAPE is entirely the generated builder's
    /// (`contracts::timer`, generated from the same source as the schema, so they cannot drift); this only
    /// supplies the `UInt64` leaf.
    fn build(&self, b: &mut Builder) -> StructId {
        use crate::contract_value as v;
        use crate::contracts::timer as c;
        let duration = v::uint_leaf(b, self.duration);
        c::envelope_fire_after(b, duration)
    }

    /// Encode the arm as a Cadenza value in the canonical binary form ([`cadenza_ast::codec`]). The inverse of
    /// [`decode`](Self::decode).
    #[must_use]
    pub fn encode(&self) -> Bytes {
        let mut b = Builder::new();
        let root = self.build(&mut b);
        let arenas = b.finish(root);
        Bytes::from(codec::encode(&arenas))
    }

    /// The [`Request`](crate::Request) a reducer emits to arm this timer: against the [`timer_contract`], with
    /// the arm as its payload and `continuation_token` as the reducer's own correlation — the [`Fired`] wake
    /// comes back as the response to this request, carrying the same token, so the reducer folds it against
    /// whatever it was waiting on. Carries no deadline (a timer is not itself deadline-bounded).
    #[must_use]
    pub fn into_request(self, continuation_token: Bytes) -> Request {
        Request {
            id: timer_contract(),
            payload: self.encode(),
            continuation_token,
            deadline: None,
        }
    }

    /// Decode an arm from a Cadenza value, or `None` if the bytes are not a well-formed `FireAfter` value.
    /// Total, so a malformed value is a rejected arm, never a panic.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        use crate::contract_value as v;
        use crate::contracts::timer as c;
        let arenas = codec::decode(bytes)?;
        let duration = c::as_envelope_fire_after(&arenas, arenas.root)?;
        Some(Self {
            duration: v::read_uint(&arenas, duration)?,
        })
    }
}

impl Fired {
    /// Build the fired-event value into `b`, returning its root — a value of the schema type `Event`.
    fn build(&self, b: &mut Builder) -> StructId {
        use crate::contract_value as v;
        use crate::contracts::timer as c;
        let fired_time = v::uint_leaf(b, self.fired_time);
        c::event_fired(b, fired_time)
    }

    /// Encode the fired event as a Cadenza value in the canonical binary form. The inverse of
    /// [`decode`](Self::decode).
    #[must_use]
    pub fn encode(&self) -> Bytes {
        let mut b = Builder::new();
        let root = self.build(&mut b);
        let arenas = b.finish(root);
        Bytes::from(codec::encode(&arenas))
    }

    /// Decode a fired event from a Cadenza value, or `None` if the bytes are not a well-formed `Fired` value.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        use crate::contract_value as v;
        use crate::contracts::timer as c;
        let arenas = codec::decode(bytes)?;
        let fired_time = c::as_event_fired(&arenas, arenas.root)?;
        Some(Self {
            fired_time: v::read_uint(&arenas, fired_time)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{FireAfter, Fired, timer_contract};
    use crate::Bytes;

    #[test]
    fn a_fire_after_arm_round_trips_through_the_codec() {
        // A duration up to u64::MAX round-trips, including zero and the extremes the unsigned width admits.
        for duration in [0u64, 5000, u64::MAX] {
            let arm = FireAfter { duration };
            assert_eq!(FireAfter::decode(&arm.encode()), Some(arm));
        }
    }

    #[test]
    fn a_fired_event_round_trips_through_the_codec() {
        for fired_time in [0u64, 1_724_371_200, u64::MAX] {
            let ev = Fired { fired_time };
            assert_eq!(Fired::decode(&ev.encode()), Some(ev));
        }
    }

    #[test]
    fn into_request_carries_the_standard_continuation_token_and_the_timer_contract() {
        // The arm is an ordinary request against the timer contract; the reducer's correlation rides in the
        // request's standard continuation-token, not in the value (operator review 2026-08-22).
        let token = Bytes::from_static(b"awaiting-deadline");
        let req = FireAfter { duration: 5000 }.into_request(token.clone());
        assert_eq!(req.id, timer_contract());
        assert_eq!(req.continuation_token, token);
        assert!(req.deadline.is_none());
        // The payload is the encoded arm — it decodes back to the same duration, with no token inside it.
        assert_eq!(
            FireAfter::decode(&req.payload),
            Some(FireAfter { duration: 5000 })
        );
    }

    #[test]
    fn decode_rejects_bytes_that_are_not_a_timer_value() {
        // Not a valid encoding at all.
        assert_eq!(FireAfter::decode(&[0xFF, 0x00, 0x13, 0x37]), None);
        assert_eq!(FireAfter::decode(&[]), None);
        // A well-formed Cadenza value of the wrong shape decodes cleanly but is not a timer arm.
        let mut b = cadenza_ast::ast::Builder::new();
        let root = b.name("not-a-timer");
        let wrong_shape = cadenza_ast::codec::encode(&b.finish(root));
        assert_eq!(FireAfter::decode(&wrong_shape), None);
        // An arm value is not a fired event and vice-versa (the two constructors do not cross).
        assert_eq!(Fired::decode(&FireAfter { duration: 1 }.encode()), None);
        assert_eq!(FireAfter::decode(&Fired { fired_time: 1 }.encode()), None);
    }

    #[test]
    fn the_encoded_arm_has_the_shape_the_schema_declares() {
        // The value must be an `Envelope.FireAfter` whose payload reads back through the generated reader as
        // the UInt64 leaf — a regression to a bespoke shape fails here.
        use crate::contract_value as v;
        use crate::contracts::timer as c;
        let arm = FireAfter { duration: 5000 };
        let arenas = cadenza_ast::codec::decode(&arm.encode()).expect("well-formed value");
        let payload =
            c::as_envelope_fire_after(&arenas, arenas.root).expect("an Envelope.FireAfter value");
        assert_eq!(v::read_uint(&arenas, payload), Some(5000));
    }

    #[test]
    fn the_timer_contract_id_is_a_real_contract_id_and_stable() {
        // Stable across calls (cached), and equal to the id a fresh Contract with the same schema derives —
        // i.e. it is the hash of the declared schema, not of a bare name.
        assert_eq!(timer_contract(), timer_contract());
        let rebuilt = crate::Contract::new(
            crate::Str::from_static("cdz-platform.timer"),
            crate::contracts::timer::schema,
            "Envelope",
            "Event",
        );
        assert_eq!(timer_contract(), rebuilt.id());
    }
}

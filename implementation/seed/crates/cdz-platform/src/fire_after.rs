//! The fire-after timer — the one time primitive (`design/cadenza-platform.md` §6).
//!
//! A reducer arms a timer by performing the [`fire_after_contract`]: it emits a [`FireAfter`] naming a
//! duration and a token it chooses. The runtime waits that long on its clock, then delivers a
//! [`TimerFired`] notification on the [`timer_fired_contract`] carrying the same token back to the reducer,
//! so it wakes and can tell which of its timers fired. Absolute deadlines and crons are the reducer's to
//! build on top; the kernel provides only this raw wake. Enforcing a request deadline is likewise the system
//! reducer's policy (§4), built on this primitive.
//!
//! Both the arm and the wake are Cadenza values in the one canonical codec, their schemas generated from
//! `contracts/fire_after.cdz`. Decoding is total: a malformed value is `None`, never a panic.

use crate::{Bytes, Contract, ContractId, Notification, Request, Str};
use cadenza_ast::ast::{Builder, StructId};
use cadenza_ast::codec;
use std::sync::OnceLock;

/// The contract a reducer performs to arm a timer (§6): its payload is a [`FireAfter`]. A real contract
/// whose id is the hash of its declared schema, built once and cached.
#[must_use]
pub fn fire_after_contract() -> ContractId {
    static FIRE_AFTER: OnceLock<Contract> = OnceLock::new();
    FIRE_AFTER
        .get_or_init(|| {
            Contract::new(
                Str::from_static("cdz-platform.fire-after"),
                crate::contracts::fire_after::schema,
                "Arm",
                "Ack",
            )
        })
        .id()
}

/// The contract of the wake the runtime delivers when a timer fires (§6): a [`Notification`] whose `id` is
/// this contract's id carries a [`TimerFired`] payload back to the reducer that armed the timer.
#[must_use]
pub fn timer_fired_contract() -> ContractId {
    static TIMER_FIRED: OnceLock<Contract> = OnceLock::new();
    TIMER_FIRED
        .get_or_init(|| {
            Contract::new(
                Str::from_static("cdz-platform.timer-fired"),
                crate::contracts::fire_after::schema,
                "Fired",
                "Ack",
            )
        })
        .id()
}

/// Arming a timer: wake me after `millis` milliseconds, echoing `token` so I can correlate the wake (§6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FireAfter {
    /// How long to wait before the wake, in milliseconds.
    pub millis: u64,
    /// A token the reducer chooses; echoed back on the [`TimerFired`] wake so it can tell its timers apart.
    pub token: Bytes,
}

impl FireAfter {
    /// Build the arm value into `b` — a value of the schema type `Arm`. `millis` crosses as its
    /// little-endian 8 bytes.
    fn build(&self, b: &mut Builder) -> StructId {
        use crate::contract_value as v;
        use crate::contracts::fire_after as c;
        let millis = v::bytes_leaf(b, &self.millis.to_le_bytes());
        let token = v::bytes_leaf(b, &self.token);
        c::arm_arm(b, c::ArmArm { millis, token })
    }

    /// Encode the arm as a canonical Cadenza value.
    #[must_use]
    pub fn encode(&self) -> Bytes {
        let mut b = Builder::new();
        let root = self.build(&mut b);
        Bytes::from(codec::encode(&b.finish(root)))
    }

    /// The [`Request`] a reducer emits to arm this timer: against the [`fire_after_contract`], with the arm
    /// as its payload. It correlates nothing of the reducer's own (the wake is a notification), so it carries
    /// an empty token and no deadline.
    #[must_use]
    pub fn into_request(self) -> Request {
        Request {
            id: fire_after_contract(),
            payload: self.encode(),
            continuation_token: Bytes::new(),
            deadline: None,
        }
    }

    /// Decode an arm, or `None` if the bytes are not a well-formed arm (an oversized or short `millis` leaf
    /// is rejected, so the duration is always exactly a 64-bit count).
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        use crate::contract_value as v;
        use crate::contracts::fire_after as c;
        let arenas = codec::decode(bytes)?;
        let arm = c::as_arm_arm(&arenas, arenas.root)?;
        let millis = v::read_bytes(&arenas, arm.millis)?;
        Some(Self {
            millis: u64::from_le_bytes(<[u8; 8]>::try_from(millis.as_ref()).ok()?),
            token: v::read_bytes(&arenas, arm.token)?,
        })
    }
}

/// The wake delivered when a timer fires (§6): the token the reducer armed it with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimerFired {
    /// The token from the [`FireAfter`] that armed this timer.
    pub token: Bytes,
}

impl TimerFired {
    /// Encode the wake as a canonical Cadenza value (schema type `Fired`).
    #[must_use]
    pub fn encode(&self) -> Bytes {
        use crate::contract_value as v;
        use crate::contracts::fire_after as c;
        let mut b = Builder::new();
        let token = v::bytes_leaf(&mut b, &self.token);
        let root = c::fired_fired(&mut b, c::FiredFired { token });
        Bytes::from(codec::encode(&b.finish(root)))
    }

    /// The control-plane [`Notification`] the runtime delivers on fire: on the [`timer_fired_contract`],
    /// carrying this wake as its payload.
    #[must_use]
    pub fn into_notification(self) -> Notification {
        Notification {
            id: timer_fired_contract(),
            payload: self.encode(),
        }
    }

    /// Decode a wake, or `None` if the bytes are not a well-formed fired value.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        use crate::contract_value as v;
        use crate::contracts::fire_after as c;
        let arenas = codec::decode(bytes)?;
        let fired = c::as_fired_fired(&arenas, arenas.root)?;
        Some(Self {
            token: v::read_bytes(&arenas, fired.token)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{FireAfter, TimerFired, fire_after_contract, timer_fired_contract};
    use crate::Bytes;

    #[test]
    fn an_arm_round_trips_through_the_codec() {
        let arm = FireAfter {
            millis: 1_500,
            token: Bytes::from_static(b"deadline-7"),
        };
        assert_eq!(FireAfter::decode(&arm.encode()), Some(arm.clone()));
        // The request an arm becomes is against the fire-after contract.
        assert_eq!(arm.into_request().id, fire_after_contract());
    }

    #[test]
    fn a_fired_wake_round_trips_and_rides_the_timer_fired_contract() {
        let fired = TimerFired {
            token: Bytes::from_static(b"deadline-7"),
        };
        let n = fired.clone().into_notification();
        assert_eq!(n.id, timer_fired_contract());
        assert_eq!(TimerFired::decode(&n.payload), Some(fired));
    }

    #[test]
    fn the_arm_and_fired_contracts_are_distinct_stable_ids() {
        assert_eq!(fire_after_contract(), fire_after_contract());
        assert_ne!(fire_after_contract(), timer_fired_contract());
    }

    #[test]
    fn decode_rejects_malformed_and_a_wrong_length_duration() {
        assert_eq!(FireAfter::decode(&[0xFF, 0x00]), None);
        // A well-formed arm value whose millis leaf is not 8 bytes is rejected.
        let bad = {
            use crate::contract_value as v;
            use crate::contracts::fire_after as c;
            let mut b = cadenza_ast::ast::Builder::new();
            let millis = v::bytes_leaf(&mut b, b"short");
            let token = v::bytes_leaf(&mut b, b"t");
            let root = c::arm_arm(&mut b, c::ArmArm { millis, token });
            Bytes::from(cadenza_ast::codec::encode(&b.finish(root)))
        };
        assert_eq!(FireAfter::decode(&bad), None);
    }
}

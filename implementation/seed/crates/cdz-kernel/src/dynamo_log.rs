//! The DynamoDB log backend (agent-runtime L1d) — the real MULTI-WRITER ordering authority.
//!
//! Vision §2.1: anyone appends concurrently without coordinating, and DynamoDB is the ordering + dedup
//! authority — an atomic CONDITIONAL write assigns each event a monotonic `seq`, so N concurrent appends
//! get a total order and no duplicates without the writers coordinating. This is what the file log
//! ([`crate::FileLog`]) only *stands in* for (single-process); [`DynamoLog`] is the production backend the
//! single fold owner tails while many writers append.
//!
//! Structure mirrors cdz-agent's `bedrock` feature: the async aws-sdk + tokio tree is behind the `aws`
//! feature (kept out of the default build + CI), while the pure **marshalling** (an [`Event`] ↔ a
//! DynamoDB item's logical fields) lives in the DEFAULT build and is UNIT-TESTED with no creds/network.
//! Only the actual DynamoDB calls are feature-gated (network-only, exercised manually).

use crate::{Event, Seq};

/// The DynamoDB attribute NAMES for a log item. A log is one DynamoDB table partition (`LOG_PK` = a fixed
/// partition key per log stream), with `seq` as the range key (the total order), and `kind`/`payload` the
/// event body. Named consts so the marshalling and the (feature-gated) client agree by construction.
pub const ATTR_PK: &str = "log";
pub const ATTR_SEQ: &str = "seq";
pub const ATTR_KIND: &str = "kind";
pub const ATTR_PAYLOAD: &str = "payload";

/// The logical, backend-agnostic form of a log item — what a DynamoDB item encodes, WITHOUT depending on
/// any aws-sdk type (so this and its round-trip are testable in the default build). `pk` is the log-stream
/// partition key; the rest mirror [`Event`]. The (feature-gated) client turns this into/from the sdk's
/// `AttributeValue` map; keeping the shape here means the marshalling logic is exercised without the SDK.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub pk: String,
    pub seq: Seq,
    pub kind: String,
    pub payload: Vec<u8>,
}

/// Marshal an [`Event`] into the log [`Item`] for stream `pk` — pure, no SDK. The inverse of
/// [`item_to_event`]; a round-trip must preserve seq/kind/payload exactly (the log is the source of truth).
pub fn event_to_item(pk: &str, e: &Event) -> Item {
    Item {
        pk: pk.to_string(),
        seq: e.seq,
        kind: e.kind.clone(),
        payload: e.payload.clone(),
    }
}

/// Unmarshal a log [`Item`] back into an [`Event`] (dropping the stream `pk`, which is log-level not
/// event-level) — pure, no SDK.
pub fn item_to_event(i: &Item) -> Event {
    Event {
        seq: i.seq,
        kind: i.kind.clone(),
        payload: i.payload.clone(),
    }
}

// ── The real DynamoDB client — behind the `aws` feature only (network-only; exercised manually). ────────
//
// L1d's client is intentionally thin + not part of CI (no creds/network in CI, exactly as cdz-agent's
// bedrock invoke is): the ordering-authority CONTRACT is a conditional `PutItem` on
// `attribute_not_exists(seq)` for the next seq (retry-on-collision gives the many-writer total order), and
// `tail` is a `Query` on the partition with `seq >= from`. The marshalling above (the part with real logic)
// is what the default-build tests cover; the calls below are wiring over the sdk. Fleshed out when a real
// DynamoDB table is wired for an L1 integration run.
#[cfg(feature = "aws")]
mod client {
    // Placeholder for the aws-sdk-dynamodb PutItem(conditional)/Query wiring — added with a live table.
    // Kept behind the feature so the default build/CI never compiles the aws tree; `cargo build --features
    // aws` confirms the feature + optional deps resolve.
    #[allow(unused_imports)]
    use aws_sdk_dynamodb as _dynamodb;
    #[allow(unused_imports)]
    use tokio as _tokio;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_item_round_trips_exactly() {
        // The log is the source of truth, so marshalling must preserve seq/kind/payload byte-for-byte.
        let e = Event {
            seq: 7,
            kind: "model-response".into(),
            payload: b"HI".to_vec(),
        };
        let item = event_to_item("agent:demo", &e);
        assert_eq!(
            item.pk, "agent:demo",
            "the stream partition key is carried on the item"
        );
        assert_eq!(item.seq, 7);
        assert_eq!(
            item_to_event(&item),
            e,
            "event -> item -> event is identity"
        );
    }

    #[test]
    fn round_trip_preserves_binary_and_empty_payloads() {
        // Payload is opaque bytes (an event body a later rung encodes) — NUL/high bytes/empty must survive,
        // same discipline as the file log's length-framed records.
        for payload in [vec![], vec![0u8, 255, b'"', 200, b'\n']] {
            let e = Event {
                seq: 0,
                kind: "blob".into(),
                payload: payload.clone(),
            };
            assert_eq!(
                item_to_event(&event_to_item("pk", &e)).payload,
                payload,
                "binary/empty payloads round-trip through the item marshalling"
            );
        }
    }

    #[test]
    fn attr_names_are_stable() {
        // The marshalling and the (feature-gated) client agree on these names by construction; pin them so
        // a rename that would desync the two is caught.
        assert_eq!(
            (ATTR_PK, ATTR_SEQ, ATTR_KIND, ATTR_PAYLOAD),
            ("log", "seq", "kind", "payload")
        );
    }
}

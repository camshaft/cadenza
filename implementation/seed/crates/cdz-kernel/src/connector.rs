//! External adapters over the log (minimal-kernel re-charter, rung KA — the connector topology).
//!
//! Operator rulings (client-and-adapters + connector-logs designs): external I/O is handled by SEPARATE
//! deployable adapters (the archetype is a Slack connector), NOT by kernel code. Each connector has TWO logs
//! in a SEPARATE-STREAM topology (fork-1 ruling: NOT tailing the main log):
//!   - INBOUND: a user's external message → the connector APPENDS it into the MAIN log, on-behalf-of the user
//!     (a Cedar PRINCIPAL is carried on the event; fork-2 ruling — the interpret program reads it + Cedar
//!     attenuates what that principal may authorize).
//!   - OUTBOUND: the connector reads WORK-ITEMS from its OWN per-connector log, which the main kernel WRITES
//!     into (a scoped feed — the connector consumes only its items, never folds the whole main log).
//!
//! This module is the connector's LOG topology — pure over the [`crate::Log`] trait, testable with `FileLog`s,
//! no network. The actual Slack HTTP I/O (translate a Slack event ↔ these log ops) is the deployed connector
//! binary on top; here we prove the two-log separate-stream shape + the on-behalf-of principal tagging. The
//! kernel stays event-agnostic: an inbound event's `kind`/`payload` are opaque to it (a Cadenza concern); the
//! principal is a small structured PREFIX on the payload the interpret program reads — the connector does not
//! interpret it either, it just stamps it.

use crate::{Event, Log, Seq};
use anyhow::{anyhow, Result};

/// The event `kind` an inbound connector message carries in the MAIN log — a user's external message posted
/// on-behalf-of a principal. Its payload is [`OnBehalfOf`] (principal + the opaque body); the interpret
/// program decodes it + evaluates the principal's Cedar policy. One kind, like the boot path's `program`.
pub const INBOUND: &str = "inbound";

/// An on-behalf-of envelope: a Cedar `principal` (who the connector is acting for) + the opaque `body` (the
/// external message bytes — Slack text, etc., meaningful only to the Cadenza interpret program). The connector
/// STAMPS the principal; neither the connector nor the kernel interprets the body. Length-prefixed codec, same
/// dependency-free discipline as `msg`/`sub`/`kernel`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnBehalfOf {
    pub principal: String,
    pub body: Vec<u8>,
}

impl OnBehalfOf {
    /// Encode to the [`INBOUND`] event payload: `principal` (length-prefixed) + `body` (length-prefixed).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let p = self.principal.as_bytes();
        out.extend_from_slice(&(p.len() as u32).to_le_bytes());
        out.extend_from_slice(p);
        out.extend_from_slice(&(self.body.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.body);
        out
    }

    /// Decode an [`OnBehalfOf`] from an [`INBOUND`] payload. Errors on a truncated/malformed payload (the log
    /// is the source of truth — a bad decode is loud).
    pub fn decode(bytes: &[u8]) -> Result<OnBehalfOf> {
        let get_len = |b: &[u8], i: usize| -> Result<usize> {
            let s = b.get(i..i + 4).ok_or_else(|| {
                anyhow!("truncated on-behalf-of: expected a 4-byte length at {i}")
            })?;
            Ok(u32::from_le_bytes(s.try_into().expect("4 bytes")) as usize)
        };
        let plen = get_len(bytes, 0)?;
        let pend = 4 + plen;
        let principal = String::from_utf8(
            bytes
                .get(4..pend)
                .ok_or_else(|| anyhow!("truncated on-behalf-of principal"))?
                .to_vec(),
        )
        .map_err(|e| anyhow!("on-behalf-of principal not UTF-8: {e}"))?;
        let blen = get_len(bytes, pend)?;
        let bstart = pend + 4;
        let bend = bstart + blen;
        let body = bytes
            .get(bstart..bend)
            .ok_or_else(|| anyhow!("truncated on-behalf-of body"))?
            .to_vec();
        if bend != bytes.len() {
            return Err(anyhow!(
                "on-behalf-of payload has {} trailing bytes",
                bytes.len() - bend
            ));
        }
        Ok(OnBehalfOf { principal, body })
    }
}

/// INBOUND: post a user's external message into the MAIN log, on-behalf-of `principal`. Appends an [`INBOUND`]
/// event whose payload is the [`OnBehalfOf`] envelope, returning its `seq`. This is a connector's inbound
/// step — a Slack message becomes a log event stamped with the Cedar principal, for the interpret program to
/// handle under that principal's policy. The connector does NOT interpret the body.
pub fn post_on_behalf(main_log: &mut impl Log, principal: &str, body: &[u8]) -> Result<Seq> {
    let env = OnBehalfOf {
        principal: principal.to_string(),
        body: body.to_vec(),
    };
    main_log.append(INBOUND, &env.encode())
}

/// OUTBOUND: the connector's WORK-ITEMS — every event in its OWN per-connector log with `seq >= from`, in
/// order. The main kernel WRITES work-items into this log (via `log_append`); the connector reads its scoped
/// feed here (a plain tail of its own log — NOT a fold of the main log; separate-stream topology). Returns the
/// events verbatim (their `kind`/`payload` are the kernel's projected instructions — opaque to the connector
/// until it translates them to Slack calls). `from` is the connector's cursor (advance past processed items).
pub fn work_items(own_log: &impl Log, from: Seq) -> Result<Vec<Event>> {
    own_log.tail(from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileLog;

    fn temp_log(tag: &str) -> (std::path::PathBuf, FileLog) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!(
            "cdz-kernel-conn-{tag}-{}-{n}.log",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        (p.clone(), FileLog::open(&p).unwrap())
    }

    #[test]
    fn on_behalf_of_round_trips_including_binary_body() {
        let e = OnBehalfOf {
            principal: "user:U123".into(),
            body: vec![0u8, 255, b'h', b'i', b'\n'],
        };
        assert_eq!(OnBehalfOf::decode(&e.encode()).unwrap(), e);
        // Empty principal + empty body survive too.
        let e0 = OnBehalfOf {
            principal: "".into(),
            body: vec![],
        };
        assert_eq!(OnBehalfOf::decode(&e0.encode()).unwrap(), e0);
        assert!(
            OnBehalfOf::decode(&e.encode()[..3]).is_err(),
            "a truncated envelope is a loud error"
        );
    }

    #[test]
    fn inbound_posts_a_principal_stamped_event_into_the_main_log() {
        // A connector posts a user's Slack message into the MAIN log on-behalf-of the user; it lands as an
        // `inbound` event carrying the Cedar principal + the opaque body, for the interpret program to handle.
        let (path, mut main) = temp_log("main");
        let seq = post_on_behalf(&mut main, "user:U123", b"deploy the thing").unwrap();
        let events = main.tail(0).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, INBOUND, "posted as an inbound event");
        let env = OnBehalfOf::decode(&events[0].payload).unwrap();
        assert_eq!(
            env.principal, "user:U123",
            "stamped with the Cedar principal"
        );
        assert_eq!(env.body, b"deploy the thing", "the opaque body survives");
        assert_eq!(seq, events[0].seq);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn outbound_reads_only_the_connectors_own_scoped_log_not_the_main_log() {
        // Separate-stream topology: the connector's outbound feed is ITS OWN log (the kernel writes work-items
        // into it), NOT the main log. A connector reads its scoped items; main-log events never appear here.
        let (mpath, mut main) = temp_log("main2");
        let (cpath, mut conn) = temp_log("conn");
        // Main log has unrelated traffic the connector must NOT see.
        post_on_behalf(&mut main, "user:U1", b"a").unwrap();
        main.append("model-response", b"internal").unwrap();
        // The kernel writes two work-items into THIS connector's own log.
        conn.append("send-slack", b"msg-to-U1").unwrap();
        conn.append("send-slack", b"followup").unwrap();

        let items = work_items(&conn, 0).unwrap();
        assert_eq!(
            items.len(),
            2,
            "the connector sees exactly its own 2 work-items"
        );
        assert!(
            items.iter().all(|e| e.kind == "send-slack"),
            "only the kernel-written work-items, none of the main log's traffic"
        );
        // Cursor: after processing item 0, read only from seq 1.
        let rest = work_items(&conn, 1).unwrap();
        assert_eq!(rest.len(), 1, "the cursor scopes to unprocessed work-items");
        assert_eq!(rest[0].payload, b"followup");
        let _ = std::fs::remove_file(&mpath);
        let _ = std::fs::remove_file(&cpath);
    }
}

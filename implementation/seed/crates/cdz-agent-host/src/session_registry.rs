//! DynamoDB-backed SESSION REGISTRY — the AWS-native INDEX of which sessions exist + which are ACTIVE (I4b).
//! Operator steer on PR #2622: don't enumerate sessions with a full-table Scan of the event LOG (O(all
//! events)); build the REAL index that ALSO answers "current active sessions" — "do it right". This is that
//! index: one small registry ITEM per session, so boot-recovery enumeration + the live active-sessions view
//! are both O(sessions) Queries, not a log scan.
//!
//! **Why a SEPARATE table (not the log table).** The event-log table's sort key is the NUMERIC `seq`
//! (`(session_id HASH, seq N RANGE)`); a registry item keyed by session-id-string can't share that numeric
//! sort key. So the registry is its own table `[session_registry].table`: partition key = `session_id` (one
//! item per session), plus a Global Secondary Index on `status` so "list ACTIVE sessions" is a Query on the
//! GSI partition `status = "active"` rather than a scan-with-filter. Selected by config when the
//! `live-aws-storage` feature is compiled; credentials via the SDK default chain (env/profile/IMDS), same
//! contract as the log/blob/name backends — no broker.
//!
//! **What it serves.**
//! - I4b BOOT-RECOVERY: on daemon boot, `list_all()` (or `list_active()`) yields the sessions to recover; the
//!   registry's `status` lets the boot loop SKIP terminated sessions WITHOUT reading their logs (cheaper than
//!   recover-then-`is_terminated`), then per surviving id → `DynamoLogSink::read_recovered` → `recover_from`.
//! - LIVE OPS: `list_active()` is the "current active sessions" query (an operator/admin view).
//!
//! **Writes (wired in the host loop).** `register(id, reducer_hash)` fires on a successful install
//! (status=active); `mark_terminated(id)` fires on terminate (§lifecycle I7). Both are idempotent (a
//! re-driven install re-puts the same active item; a re-driven terminate re-sets terminated), and both are
//! BEST-EFFORT — a write failure is logged, never fails the install or crashes the loop (the durable log
//! remains the source of truth; the registry is an index over it).

/// A session's lifecycle status in the registry. `active` = installed + schedulable; `terminated` = the
/// session hit its durable terminal marker (§lifecycle terminate / I7) and must NOT be recovered/re-registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Active,
    Terminated,
}

impl SessionStatus {
    /// The wire string stored in `status` (also the GSI partition value). Stable — a rename would split the
    /// index from the writers.
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::Active => "active",
            SessionStatus::Terminated => "terminated",
        }
    }

    /// Parse a stored `status` string back. An unknown value is a corrupt registry item (an alarm) → `None`.
    /// (Named `from_wire`, not `from_str`, to avoid shadowing the `std::str::FromStr::from_str` convention.)
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "active" => Some(SessionStatus::Active),
            "terminated" => Some(SessionStatus::Terminated),
            _ => None,
        }
    }
}

/// One registry record: a session's id + status + the reducer it runs (so boot-recovery can rebuild it) +
/// when the record was last written. The pure decoded shape a `list_*` query returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub session_id: String,
    pub status: SessionStatus,
    /// The reducer's content hash (the genesis reducer — boot-recovery loads it by this to rebuild the
    /// session's reducer). Stored RAW ([`Hash`] = 32 bytes), round-tripped through Dynamo as a BINARY (`B`)
    /// attribute — NO hex (operator directive: zero to_hex conversions except tracing; Dynamo stores binary
    /// natively, no string coercion).
    pub reducer_hash: cdz_kernel::hash::Hash,
    /// Millis since epoch when this record was last written (host-supplied; the kernel stays entropy/clock
    /// free). Informational — orders records for an operator view, not load-bearing for recovery.
    pub updated_ms: u64,
}

#[cfg(feature = "live-aws-storage")]
pub use live::DynamoSessionRegistry;

#[cfg(feature = "live-aws-storage")]
mod live {
    use super::*;
    use aws_sdk_dynamodb::types::AttributeValue;
    use std::io;

    /// Attribute names — one place so writes + reads + the GSI agree.
    const ATTR_SESSION: &str = "session_id";
    const ATTR_STATUS: &str = "status";
    const ATTR_REDUCER: &str = "reducer_hash";
    const ATTR_UPDATED_MS: &str = "updated_ms";
    /// The GSI on `status` (partition = status value) — "list active" is a Query on `status = "active"`.
    const STATUS_INDEX: &str = "status-index";

    /// A DynamoDB-backed session registry over a configured `table` (+ its `status-index` GSI). Holds the
    /// shared dynamodb client; each op is a single keyed write/query. Loads the ambient AWS config ONCE at
    /// construction (the boot path), like the log/blob backends.
    pub struct DynamoSessionRegistry {
        client: aws_sdk_dynamodb::Client,
        table: String,
    }

    impl DynamoSessionRegistry {
        /// Build from an explicit SDK config (a test/integration harness, or a local DynamoDB endpoint).
        pub fn from_conf(config: &aws_config::SdkConfig, table: impl Into<String>) -> Self {
            DynamoSessionRegistry {
                client: aws_sdk_dynamodb::Client::new(config),
                table: table.into(),
            }
        }

        /// Load the ambient AWS config (SDK default chain) ONCE + build. Async because the default chain may
        /// probe the environment (IMDS) — on the daemon boot path, like the other AWS backends.
        pub async fn new(table: impl Into<String>) -> Self {
            let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            DynamoSessionRegistry::from_conf(&config, table)
        }

        /// REGISTER a session as active (install/genesis). Idempotent: a re-driven install re-puts the same
        /// item (status=active), so a crash-redriven register overwrites byte-identically. `reducer_hash` is
        /// how boot-recovery reloads the reducer — passed + stored as raw [`Hash`], no hex. `now_ms` is
        /// host-supplied (the kernel is clock-free).
        pub async fn register(
            &self,
            session_id: &str,
            reducer_hash: cdz_kernel::hash::Hash,
            now_ms: u64,
        ) -> io::Result<()> {
            self.put(session_id, SessionStatus::Active, reducer_hash, now_ms)
                .await
        }

        /// Mark a session TERMINATED (§lifecycle terminate/I7) — boot-recovery then skips it without reading
        /// its log. Idempotent. Preserves `reducer_hash_hex` (still recorded, just not recovered). Reads the
        /// existing reducer hash so a terminate needn't re-supply it; a missing item is a no-op-ish put of a
        /// terminated record with an empty reducer hash (a terminate for an unknown session shouldn't fail the
        /// caller — the session is gone either way).
        pub async fn mark_terminated(&self, session_id: &str, now_ms: u64) -> io::Result<()> {
            // Preserve the recorded reducer hash if present; a terminate for an unknown session (no item)
            // writes a terminated record with a zero hash (a terminate shouldn't fail — the session is gone
            // either way, and a terminated record is never recovered so its reducer hash is unread).
            let reducer = self
                .get_reducer_hash(session_id)
                .await?
                .unwrap_or_else(|| cdz_kernel::hash::Hash::from_bytes([0u8; 32]));
            self.put(session_id, SessionStatus::Terminated, reducer, now_ms)
                .await
        }

        /// LIST ALL registered sessions (both active + terminated) — the boot-recovery enumeration input.
        /// Scans the REGISTRY table (small: one item per session, O(sessions)); paginates. Distinct from the
        /// rejected #2622 approach, which scanned the whole EVENT LOG (O(all events)).
        pub async fn list_all(&self) -> io::Result<Vec<SessionRecord>> {
            let mut records = Vec::new();
            let mut last_key = None;
            loop {
                let resp = self
                    .client
                    .scan()
                    .table_name(&self.table)
                    .set_exclusive_start_key(last_key)
                    .send()
                    .await
                    .map_err(|e| {
                        io::Error::other(format!(
                            "session registry list_all scan failed: {}",
                            aws_sdk_dynamodb::error::DisplayErrorContext(&e)
                        ))
                    })?;
                for item in resp.items() {
                    records.push(decode_record(item)?);
                }
                match resp.last_evaluated_key() {
                    Some(k) if !k.is_empty() => last_key = Some(k.clone()),
                    _ => break,
                }
            }
            Ok(records)
        }

        /// LIST ACTIVE sessions — a Query on the `status-index` GSI partition `status = "active"` (NOT a
        /// scan-with-filter): the "current active sessions" view the operator asked for. Paginates.
        pub async fn list_active(&self) -> io::Result<Vec<SessionRecord>> {
            let mut records = Vec::new();
            let mut last_key = None;
            loop {
                let resp = self
                    .client
                    .query()
                    .table_name(&self.table)
                    .index_name(STATUS_INDEX)
                    .key_condition_expression("#st = :active")
                    .expression_attribute_names("#st", ATTR_STATUS)
                    .expression_attribute_values(
                        ":active",
                        AttributeValue::S(SessionStatus::Active.as_str().to_string()),
                    )
                    .set_exclusive_start_key(last_key)
                    .send()
                    .await
                    .map_err(|e| {
                        io::Error::other(format!(
                            "session registry list_active query failed: {}",
                            aws_sdk_dynamodb::error::DisplayErrorContext(&e)
                        ))
                    })?;
                for item in resp.items() {
                    records.push(decode_record(item)?);
                }
                match resp.last_evaluated_key() {
                    Some(k) if !k.is_empty() => last_key = Some(k.clone()),
                    _ => break,
                }
            }
            Ok(records)
        }

        /// Read just the stored `reducer_hash` (raw, from the binary `B` attribute) for a session — for
        /// `mark_terminated` to preserve it. `None` if the item or attribute is absent / not 32 bytes.
        async fn get_reducer_hash(
            &self,
            session_id: &str,
        ) -> io::Result<Option<cdz_kernel::hash::Hash>> {
            let resp = self
                .client
                .get_item()
                .table_name(&self.table)
                .key(ATTR_SESSION, AttributeValue::S(session_id.to_string()))
                .send()
                .await
                .map_err(|e| {
                    io::Error::other(format!(
                        "session registry get_item failed: {}",
                        aws_sdk_dynamodb::error::DisplayErrorContext(&e)
                    ))
                })?;
            Ok(resp.item().and_then(|item| match item.get(ATTR_REDUCER) {
                Some(AttributeValue::B(b)) => <[u8; 32]>::try_from(b.as_ref())
                    .ok()
                    .map(cdz_kernel::hash::Hash::from_bytes),
                _ => None,
            }))
        }

        /// The shared PutItem for register + mark_terminated.
        async fn put(
            &self,
            session_id: &str,
            status: SessionStatus,
            reducer_hash: cdz_kernel::hash::Hash,
            now_ms: u64,
        ) -> io::Result<()> {
            self.client
                .put_item()
                .table_name(&self.table)
                .item(ATTR_SESSION, AttributeValue::S(session_id.to_string()))
                .item(ATTR_STATUS, AttributeValue::S(status.as_str().to_string()))
                .item(
                    // Raw 32 bytes as a Dynamo BINARY attribute — no hex (operator: Dynamo stores binary).
                    ATTR_REDUCER,
                    AttributeValue::B(aws_sdk_dynamodb::primitives::Blob::new(
                        reducer_hash.as_bytes().to_vec(),
                    )),
                )
                .item(ATTR_UPDATED_MS, AttributeValue::N(now_ms.to_string()))
                .send()
                .await
                .map(|_| ())
                .map_err(|e| {
                    io::Error::other(format!(
                        "session registry put ({session_id}, {}) failed: {}",
                        status.as_str(),
                        aws_sdk_dynamodb::error::DisplayErrorContext(&e)
                    ))
                })
        }
    }

    /// Decode one registry item into a `SessionRecord`. A missing/mistyped required attribute or an unknown
    /// status is a corrupt registry item → `Err` (an alarm the boot loop surfaces, never a silent skip).
    fn decode_record(
        item: &std::collections::HashMap<String, AttributeValue>,
    ) -> io::Result<SessionRecord> {
        let session_id = match item.get(ATTR_SESSION) {
            Some(AttributeValue::S(s)) => s.clone(),
            _ => return Err(corrupt("missing session_id")),
        };
        let status = match item.get(ATTR_STATUS) {
            Some(AttributeValue::S(s)) => {
                SessionStatus::from_wire(s).ok_or_else(|| corrupt("unknown status"))?
            }
            _ => return Err(corrupt("missing status")),
        };
        let reducer_hash = match item.get(ATTR_REDUCER) {
            Some(AttributeValue::B(b)) => <[u8; 32]>::try_from(b.as_ref())
                .map(cdz_kernel::hash::Hash::from_bytes)
                .map_err(|_| corrupt("reducer_hash is not 32 bytes"))?,
            _ => return Err(corrupt("missing reducer_hash")),
        };
        let updated_ms = match item.get(ATTR_UPDATED_MS) {
            Some(AttributeValue::N(n)) => {
                n.parse::<u64>().map_err(|_| corrupt("bad updated_ms"))?
            }
            _ => return Err(corrupt("missing updated_ms")),
        };
        Ok(SessionRecord {
            session_id,
            status,
            reducer_hash,
            updated_ms,
        })
    }

    fn corrupt(what: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("corrupt session registry item: {what}"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_its_wire_string() {
        for s in [SessionStatus::Active, SessionStatus::Terminated] {
            assert_eq!(SessionStatus::from_wire(s.as_str()), Some(s));
        }
        assert_eq!(
            SessionStatus::from_wire("bogus"),
            None,
            "unknown status = corrupt"
        );
    }

    #[test]
    fn status_wire_strings_are_pinned() {
        // Writers + the GSI partition value + readers must agree; pin the literals so a rename can't split them.
        assert_eq!(SessionStatus::Active.as_str(), "active");
        assert_eq!(SessionStatus::Terminated.as_str(), "terminated");
    }
}

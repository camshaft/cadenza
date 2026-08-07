//! DynamoDB-backed [`LogSink`] — the AWS-native durable event log (operator: AWS-native backends for all host
//! traits; `LogConfig::Dynamo { table }` selects it). This is I2 of the AWS-backends arc; it drops in behind
//! the SAME `cdz_kernel::log_store::LogSink` trait the on-disk `LogStore` satisfies, so a session persists
//! events through it identically — the daemon just selects this backend by `[log]` config when the
//! `live-aws-storage` feature is compiled.
//!
//! **Feature-gated (`live-aws-storage`)** alongside the S3 blob store: the DEFAULT build pulls no AWS SDK and
//! sessions use the in-memory (no sink) / on-disk `LogStore` backends, no credentials — the hermetic-gate
//! discipline. Credentials come from the SDK default provider chain (env / profile / IMDS) via `aws-config`,
//! the same contract as the Bedrock transport + the S3 blob store — no broker, no hardcoding.
//!
//! **Item schema (one item per event).** A session's log is a partition; each event is one item keyed by
//! `(session_id, seq)`:
//! - `session_id` (S) — the HASH key: the session this event belongs to (the log partition).
//! - `seq` (N) — the RANGE key: the event's monotonic sequence number (`Event::seq`), so a session's events
//!   sort + scan in order (the recovery read walks the partition by ascending `seq`).
//! - `event` (B) — the event bytes, encoded through the SAME shared `event_ast::encode` canonical codec the
//!   on-disk `LogStore` frames (language-native wire format; a Dynamo read decodes with `event_ast::decode`).
//!
//! The `append` contract (§16c-S1) is preserved: `Ok(())` ONLY once the event is durably stored (DynamoDB
//! `PutItem` is durably committed on a successful response), `Err` = it did NOT reach stable storage → the
//! caller (`Session`) latches it and refuses to route. A conditional-put on the key would reject a duplicate
//! seq, but the kernel's seq is monotonic-by-construction per session, so an unconditional put is the v0
//! contract (idempotent on identical bytes — a crash-redriven append of the same (session, seq) overwrites
//! byte-identical content), mirroring the on-disk append's non-conditional write.
//!
//! **Recovery note (read surface).** `LogSink` is APPEND-ONLY (no read on the trait); on-disk recovery is a
//! concrete `LogStore` function today, and the daemon does no boot-recovery yet. A Dynamo READ surface
//! (`read_all` — Query the partition by ascending `seq`, decode each item) is provided here as an inherent
//! method for a future boot-recovery wiring, so the Dynamo log is recoverable WITHOUT touching the kernel
//! `LogSink` trait. Wiring boot-recovery-from-Dynamo into the daemon is a follow-on slice (the daemon does no
//! recovery regardless today).

use cdz_kernel::event::Event;
use cdz_kernel::event_ast;
use cdz_kernel::log_store::LogSink;
use std::io;

/// Dynamo attribute names — one place so `append` + `read_all` agree.
const ATTR_SESSION: &str = "session_id";
const ATTR_SEQ: &str = "seq";
const ATTR_EVENT: &str = "event";

/// A DynamoDB-backed durable event log for ONE session (the `LogSinkBuilder` builds one per installed session,
/// so the session id — the log partition key — is fixed at construction, matching the `LogSink` trait whose
/// `append` gets only `&Event`). Holds the shared dynamodb client + the table + this session's partition key.
pub struct DynamoLogSink {
    client: aws_sdk_dynamodb::Client,
    table: String,
    session_id: String,
}

impl DynamoLogSink {
    /// Build a sink for `session_id` over `table`, using an explicit SDK config (the builder loads the ambient
    /// default chain once + hands it here so each per-session sink is a cheap client clone, not a fresh config
    /// load — same build-once/clone-per-session shape as the live executor set).
    pub fn from_conf(
        config: &aws_config::SdkConfig,
        table: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        DynamoLogSink {
            client: aws_sdk_dynamodb::Client::new(config),
            table: table.into(),
            session_id: session_id.into(),
        }
    }

    /// Read this session's whole log back in `seq` order — the recovery READ surface (`LogSink` is
    /// append-only; this is an inherent method, not a trait method, so it adds no kernel-trait surface). Query
    /// the partition (`session_id = self.session_id`) with ascending sort key, decode each item's `event`
    /// bytes via `event_ast::decode`. Returns the events in order. A malformed/missing attribute or a decode
    /// failure is an `Err` (a corrupt stored event is an alarm, like the on-disk `Corrupt` recovery kind).
    /// Paginates so a session longer than one Query page is fully read.
    pub async fn read_all(&self) -> io::Result<Vec<Event>> {
        let mut events = Vec::new();
        let mut last_key = None;
        loop {
            let resp = self
                .client
                .query()
                .table_name(&self.table)
                .key_condition_expression("#s = :sid")
                .expression_attribute_names("#s", ATTR_SESSION)
                .expression_attribute_values(
                    ":sid",
                    aws_sdk_dynamodb::types::AttributeValue::S(self.session_id.clone()),
                )
                .scan_index_forward(true) // ascending seq
                .set_exclusive_start_key(last_key)
                .send()
                .await
                .map_err(|e| {
                    io::Error::other(format!(
                        "DynamoLogSink read_all query failed: {}",
                        aws_sdk_dynamodb::error::DisplayErrorContext(&e)
                    ))
                })?;
            for item in resp.items() {
                let blob = match item.get(ATTR_EVENT) {
                    Some(aws_sdk_dynamodb::types::AttributeValue::B(b)) => b.as_ref(),
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "DynamoLogSink read_all: an item is missing the `event` binary attribute",
                        ));
                    }
                };
                let event = event_ast::decode(blob).map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("DynamoLogSink read_all: an event failed to decode: {e:?}"),
                    )
                })?;
                events.push(event);
            }
            // Paginate: a non-None LastEvaluatedKey means more items remain.
            match resp.last_evaluated_key() {
                Some(k) if !k.is_empty() => last_key = Some(k.clone()),
                _ => break,
            }
        }
        Ok(events)
    }
}

#[async_trait::async_trait(?Send)]
impl LogSink for DynamoLogSink {
    async fn append(&mut self, event: &Event) -> io::Result<()> {
        // Encode through the SHARED cadenza-ast canonical codec — the SAME wire format the on-disk LogStore
        // frames (language-native, not bespoke). Stored as a binary attribute.
        let body = event_ast::encode(event);
        self.client
            .put_item()
            .table_name(&self.table)
            .item(
                ATTR_SESSION,
                aws_sdk_dynamodb::types::AttributeValue::S(self.session_id.clone()),
            )
            .item(
                ATTR_SEQ,
                aws_sdk_dynamodb::types::AttributeValue::N(event.seq.to_string()),
            )
            .item(
                ATTR_EVENT,
                aws_sdk_dynamodb::types::AttributeValue::B(
                    aws_sdk_dynamodb::primitives::Blob::new(body),
                ),
            )
            .send()
            .await
            .map(|_| ())
            // §16c-S1: an append failure means the event did NOT reach stable storage → Err, and the caller
            // latches it + refuses to route. The whole point of the durable-log trait.
            .map_err(|e| {
                io::Error::other(format!(
                    "DynamoLogSink append (session {}, seq {}) failed: {}",
                    self.session_id,
                    event.seq,
                    aws_sdk_dynamodb::error::DisplayErrorContext(&e)
                ))
            })
    }
}

/// A [`LogSinkBuilder`](crate::factory::LogSinkBuilder) that builds a [`DynamoLogSink`] per installed session
/// over a configured `table` — the deployed daemon installs this when `[log].backend = "dynamo"`. Loads the
/// ambient AWS config ONCE at construction; each `build` is a cheap client clone for that session's partition
/// (build-once/clone-per-session, so a per-install sink construction never re-probes IMDS).
pub struct DynamoLogSinkBuilder {
    config: aws_config::SdkConfig,
    table: String,
}

impl DynamoLogSinkBuilder {
    /// Build the builder, loading the ambient AWS config (SDK default provider chain + region from env) ONCE.
    /// Async because the default chain may probe the environment (e.g. IMDS) — on the daemon boot path, like
    /// the S3 blob store + Bedrock transport.
    pub async fn new(table: impl Into<String>) -> Self {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        DynamoLogSinkBuilder {
            config,
            table: table.into(),
        }
    }

    /// Build from an explicit SDK config (a test/integration harness pointing at a specific region / a local
    /// DynamoDB endpoint) instead of the ambient default chain.
    pub fn from_conf(config: aws_config::SdkConfig, table: impl Into<String>) -> Self {
        DynamoLogSinkBuilder {
            config,
            table: table.into(),
        }
    }
}

#[async_trait::async_trait(?Send)]
impl crate::factory::LogSinkBuilder for DynamoLogSinkBuilder {
    async fn build(&self, id: &crate::host::SessionId) -> Result<Option<Box<dyn LogSink>>, String> {
        // Cheap per-session sink: clone the shared config into a fresh client keyed to this session's
        // partition. No network here (client construction is local; the AWS load happened once in `new`).
        Ok(Some(Box::new(DynamoLogSink::from_conf(
            &self.config,
            self.table.clone(),
            id.as_str(),
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::LogSinkBuilder; // the trait, so `.build(...)` resolves on the builder

    fn test_cfg() -> aws_config::SdkConfig {
        // A minimal config never makes a call until an operation runs — lets us construct the sink/builder
        // and exercise the pure key/attr shape without touching AWS.
        aws_config::SdkConfig::builder()
            .behavior_version(aws_config::BehaviorVersion::latest())
            .build()
    }

    #[test]
    fn sink_holds_the_table_and_session_partition() {
        let sink = DynamoLogSink::from_conf(&test_cfg(), "cdz-log", "worker-1");
        assert_eq!(sink.table, "cdz-log");
        assert_eq!(sink.session_id, "worker-1");
    }

    #[tokio::test]
    async fn builder_builds_a_sink_per_session_id() {
        let builder = DynamoLogSinkBuilder::from_conf(test_cfg(), "cdz-log");
        // The builder yields a sink (Some) for any session id — the daemon attaches it as the durable log.
        let sink = builder
            .build(&crate::host::SessionId::new("worker-2"))
            .await
            .expect("builder succeeds");
        assert!(
            sink.is_some(),
            "dynamo builder always yields a durable sink"
        );
    }

    #[test]
    fn attr_names_are_stable() {
        // append + read_all must agree on the schema; pin the names so a rename can't silently split them.
        assert_eq!(
            (ATTR_SESSION, ATTR_SEQ, ATTR_EVENT),
            ("session_id", "seq", "event")
        );
    }
}

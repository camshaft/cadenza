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
//! - `session_id` (B) — the HASH key: the session this event belongs to (the log partition), carried as the
//!   RAW 32 genesis-hash bytes (matching the SessionRegistry Binary key — no hex string; hashes-everywhere).
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

use cdz_kernel::blob::BlobStore;
use cdz_kernel::event::Event;
use cdz_kernel::event_ast;
use cdz_kernel::log_store::{maybe_offload, rehydrate, LogSink, Recovered, RecoveryKind};
use std::io;

/// Dynamo attribute names — one place so `append` + `read_all` agree.
const ATTR_SESSION: &str = "session_id";
const ATTR_SEQ: &str = "seq";
const ATTR_EVENT: &str = "event";

/// Decode a session's stored event blobs (already read in ascending `seq` order) into a kernel
/// [`Recovered`] — the pure, backend-agnostic core of the Dynamo recovery-read (I4b slice 1). Kept
/// separate from the AWS Query so it is hermetically testable without a client, and so the corruption
/// discrimination is auditable in one place.
///
/// **Corruption discrimination (I4b greenlit default #3: alarm-and-skip, never silent-drop).** Decode
/// each blob in `seq` order. The FIRST blob that fails to decode ends the good prefix and marks the read
/// [`RecoveryKind::Corrupt`] — an alarm the boot-recovery loop must not miss: it halts recovery of THAT
/// session loud and skips it, while recovering the healthy sessions. Every blob decoding cleanly is a
/// [`RecoveryKind::Clean`] read. There is no `TornTail` from Dynamo: a `PutItem` is atomic per item, so a
/// stored item is never a half-written frame (unlike the on-disk log's byte stream) — a present-but-
/// undecodable item is genuine corruption, not a torn write.
///
/// `good_prefix_len` is the summed encoded byte length of the good-prefix events. Unlike the on-disk log
/// (where it is the truncation offset), Dynamo truncation would delete items by their `seq` range, so for
/// this backend the field is informational (the count/extent of the good prefix), not a byte offset to
/// seek to. The recovery core (`Session::recover_from`) uses only `events` + `kind`, not this field.
fn recovered_from_event_blobs<'a>(blobs: impl IntoIterator<Item = &'a [u8]>) -> Recovered {
    let mut events = Vec::new();
    let mut good_prefix_len: u64 = 0;
    let mut kind = RecoveryKind::Clean;
    for blob in blobs {
        match event_ast::decode(blob) {
            Ok(event) => {
                good_prefix_len += event_ast::encode(&event).len() as u64;
                events.push(event);
            }
            // First undecodable item: genuine corruption (atomic PutItem rules out a torn frame). End the
            // good prefix here and flag Corrupt so the boot loop alarms + skips this session, not the rest.
            Err(_) => {
                kind = RecoveryKind::Corrupt;
                break;
            }
        }
    }
    Recovered {
        events,
        kind,
        good_prefix_len,
    }
}

/// A DynamoDB-backed durable event log for ONE session (the `LogSinkBuilder` builds one per installed session,
/// so the session id — the log partition key — is fixed at construction, matching the `LogSink` trait whose
/// `append` gets only `&Event`). Holds the shared dynamodb client + the table + this session's partition key.
pub struct DynamoLogSink {
    client: aws_sdk_dynamodb::Client,
    table: String,
    /// The log partition key = this session's genesis hash as RAW 32 bytes (stored as a Dynamo Binary `B`
    /// attribute, matching the SessionRegistry key — no hex string; hashes ride raw everywhere).
    session_id: cdz_kernel::hash::Hash,
    /// GAP-4 D1 log-body OFFLOAD (opt-in), the SAME shared seam the disk `LogStore` uses: when
    /// `Some((blob, threshold))`, an encoded event body larger than `threshold` bytes is offloaded to the
    /// content-addressed [`BlobStore`] (a tiny `(blob-ptr)` frame is stored as the Dynamo item's `event`
    /// attribute instead of the body) via [`maybe_offload`], keeping items small + under Dynamo's item-size
    /// ceiling; a read [`rehydrate`]s it back byte-identical. `None` = bodies stay inline (pre-D1). Each sink
    /// owns its handle (`append` + `maybe_offload` are `&mut self`/`&mut blob`); the builder opens a fresh one
    /// per session over the SAME `[blob]` root — one physical CAS, matching the File-backend D1 wiring.
    offload: Option<(Box<dyn BlobStore>, usize)>,
}

impl DynamoLogSink {
    /// Build a sink for `session_id` over `table`, using an explicit SDK config (the builder loads the ambient
    /// default chain once + hands it here so each per-session sink is a cheap client clone, not a fresh config
    /// load — same build-once/clone-per-session shape as the live executor set).
    pub fn from_conf(
        config: &aws_config::SdkConfig,
        table: impl Into<String>,
        session_id: cdz_kernel::hash::Hash,
    ) -> Self {
        DynamoLogSink {
            client: aws_sdk_dynamodb::Client::new(config),
            table: table.into(),
            session_id,
            offload: None,
        }
    }

    /// Enable GAP-4 D1 log-body offload on this sink: an encoded event body over `threshold` bytes offloads to
    /// `blob` (the content-addressed store) as a `(blob-ptr)` frame; recovery rehydrates it. The sink OWNS the
    /// handle (`append`/`maybe_offload` need `&mut`), so the builder hands each per-session sink its own.
    pub fn with_offload(mut self, blob: Box<dyn BlobStore>, threshold: usize) -> Self {
        self.offload = Some((blob, threshold));
        self
    }

    /// The REAL event body for the bytes `stored` in a Dynamo item — [`rehydrate`]d from the blob CAS when
    /// offload is configured (a `(blob-ptr)` frame derefs; an inline body passes through), or `stored`
    /// unchanged when there is no offload. Get-only, so `&self` suffices (unlike append's `&mut`).
    async fn rehydrate_body(&self, stored: Vec<u8>) -> io::Result<Vec<u8>> {
        match &self.offload {
            Some((blob, _threshold)) => rehydrate(&stored, blob.as_ref()).await,
            None => Ok(stored),
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
                    aws_sdk_dynamodb::types::AttributeValue::B(
                        aws_sdk_dynamodb::primitives::Blob::new(
                            self.session_id.as_bytes().to_vec(),
                        ),
                    ),
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
                let stored = match item.get(ATTR_EVENT) {
                    Some(aws_sdk_dynamodb::types::AttributeValue::B(b)) => b.as_ref().to_vec(),
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "DynamoLogSink read_all: an item is missing the `event` binary attribute",
                        ));
                    }
                };
                // GAP-4 D1: rehydrate an offloaded body (a (blob-ptr) frame derefs from the CAS) before decode;
                // an inline body passes through unchanged. A pointer whose blob is absent is a hard io error.
                let blob = self.rehydrate_body(stored).await?;
                let event = event_ast::decode(&blob).map_err(|e| {
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

    /// Read this session's log as a kernel [`Recovered`] for boot-recovery (I4b slice 1) — the input to
    /// [`cdz_kernel::kernel::Session::recover_from`]. Query the partition in ascending `seq`, then decode
    /// through [`recovered_from_event_blobs`], which yields the good prefix + a [`RecoveryKind`].
    ///
    /// This differs from [`read_all`](Self::read_all) in its CORRUPTION contract (I4b greenlit default #3):
    /// where `read_all` returns `Err` on the first undecodable event (it is a general read that treats any
    /// bad event as a hard failure), `read_recovered` instead ends the good prefix at the first bad item and
    /// reports [`RecoveryKind::Corrupt`] — so the boot loop keeps the recovered good prefix, alarms on the
    /// corrupt session, skips re-registering IT, and recovers the healthy sessions (never a silent drop, and
    /// never one poisoned session aborting the whole daemon's recovery). A transport/query error is still a
    /// hard `Err` (the read itself failed — distinct from a stored-event decode failure).
    pub async fn read_recovered(&self) -> io::Result<Recovered> {
        let mut blobs: Vec<Vec<u8>> = Vec::new();
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
                    aws_sdk_dynamodb::types::AttributeValue::B(
                        aws_sdk_dynamodb::primitives::Blob::new(
                            self.session_id.as_bytes().to_vec(),
                        ),
                    ),
                )
                .scan_index_forward(true) // ascending seq
                .set_exclusive_start_key(last_key)
                .send()
                .await
                .map_err(|e| {
                    io::Error::other(format!(
                        "DynamoLogSink read_recovered query failed: {}",
                        aws_sdk_dynamodb::error::DisplayErrorContext(&e)
                    ))
                })?;
            for item in resp.items() {
                match item.get(ATTR_EVENT) {
                    Some(aws_sdk_dynamodb::types::AttributeValue::B(b)) => {
                        // GAP-4 D1: rehydrate an offloaded body from the CAS before the recovery adapter sees
                        // it (so the good-prefix/corrupt discrimination runs on the REAL event bytes). A
                        // pointer whose blob is absent is a hard io error (data loss), NOT a decode-corruption.
                        let real = self.rehydrate_body(b.as_ref().to_vec()).await?;
                        blobs.push(real);
                    }
                    // A missing/mis-typed event attribute is a schema break, not a benign decode failure —
                    // the item exists but has no readable body. Treat it as the read failing (hard Err),
                    // like read_all, rather than silently marking corruption.
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "DynamoLogSink read_recovered: an item is missing the `event` binary attribute",
                        ));
                    }
                }
            }
            match resp.last_evaluated_key() {
                Some(k) if !k.is_empty() => last_key = Some(k.clone()),
                _ => break,
            }
        }
        Ok(recovered_from_event_blobs(
            blobs.iter().map(|b| b.as_slice()),
        ))
    }
}

#[async_trait::async_trait(?Send)]
impl LogSink for DynamoLogSink {
    async fn append(&mut self, event: &Event) -> io::Result<()> {
        // Encode through the SHARED cadenza-ast canonical codec — the SAME wire format the on-disk LogStore
        // frames (language-native, not bespoke). Stored as a binary attribute.
        let body = event_ast::encode(event);
        // GAP-4 D1: when offload is configured, an over-threshold body is written to the blob CAS FIRST and
        // replaced by a tiny (blob-ptr) frame here (`maybe_offload` does the content-addressed put + returns
        // the pointer bytes); the item then carries the pointer, not the body. The CAS write-through happening
        // BEFORE the put_item is what keeps the pointer from ever out-living its body (no dangling ptr on a
        // crash between the two). No offload = the body is stored inline verbatim (pre-D1 behavior).
        let stored = match &mut self.offload {
            Some((blob, threshold)) => maybe_offload(&body, blob.as_mut(), *threshold).await?,
            None => body,
        };
        self.client
            .put_item()
            .table_name(&self.table)
            .item(
                ATTR_SESSION,
                aws_sdk_dynamodb::types::AttributeValue::B(
                    aws_sdk_dynamodb::primitives::Blob::new(self.session_id.as_bytes().to_vec()),
                ),
            )
            .item(
                ATTR_SEQ,
                aws_sdk_dynamodb::types::AttributeValue::N(event.seq.to_string()),
            )
            .item(
                ATTR_EVENT,
                aws_sdk_dynamodb::types::AttributeValue::B(
                    aws_sdk_dynamodb::primitives::Blob::new(stored),
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
                    self.session_id.to_base64url(),
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
    /// GAP-4 D1 log-body offload (opt-in): when `Some((blob_dir, threshold))`, each per-session sink this
    /// builder makes offloads over-threshold event bodies to a content-addressed [`DiskBlobStore`] rooted at
    /// `blob_dir` (the SAME `[blob]` dir the effect-blob path uses — one physical CAS). A FRESH handle is
    /// opened per `build`/`recover` (the sink owns it; `append`/`maybe_offload` are `&mut`), matching the
    /// File-backend D1 wiring. `None` = bodies stay inline. (Dir-backed today, like the File path; an S3
    /// offload backend for the prod Dynamo+S3 pairing is a follow-on slice.)
    offload: Option<(std::path::PathBuf, usize)>,
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
            offload: None,
        }
    }

    /// Build from an explicit SDK config (a test/integration harness pointing at a specific region / a local
    /// DynamoDB endpoint) instead of the ambient default chain.
    pub fn from_conf(config: aws_config::SdkConfig, table: impl Into<String>) -> Self {
        DynamoLogSinkBuilder {
            config,
            table: table.into(),
            offload: None,
        }
    }

    /// Enable GAP-4 D1 log-body offload for every sink this builder makes: bodies over `threshold` bytes
    /// offload to a [`DiskBlobStore`](cdz_kernel::blob::DiskBlobStore) rooted at `blob_dir` (the SAME `[blob]`
    /// dir the effect-blob path uses — one physical CAS). Recovery rehydrates. The daemon calls this when
    /// `[log].backend = dynamo` AND `[blob].backend = dir` AND `[log].offload_threshold` is set.
    pub fn with_offload(
        mut self,
        blob_dir: impl Into<std::path::PathBuf>,
        threshold: usize,
    ) -> Self {
        self.offload = Some((blob_dir.into(), threshold));
        self
    }

    /// Open a FRESH raw [`DiskBlobStore`](cdz_kernel::blob::DiskBlobStore) over the configured offload dir — a
    /// separate handle over the same physical CAS (`put` is `&mut self`, so each `build`/`recover` owns one).
    fn open_offload_blob(
        blob_dir: &std::path::Path,
    ) -> Result<cdz_kernel::blob::DiskBlobStore, String> {
        cdz_kernel::blob::DiskBlobStore::open(blob_dir).map_err(|e| {
            format!(
                "could not open Dynamo-log-offload blob store {}: {e}",
                blob_dir.display()
            )
        })
    }
}

#[async_trait::async_trait(?Send)]
impl crate::factory::LogSinkBuilder for DynamoLogSinkBuilder {
    async fn build(&self, id: &crate::host::SessionId) -> Result<Option<Box<dyn LogSink>>, String> {
        // Cheap per-session sink: clone the shared config into a fresh client keyed to this session's
        // partition. No network here (client construction is local; the AWS load happened once in `new`).
        let sink = DynamoLogSink::from_conf(&self.config, self.table.clone(), id.hash());
        // GAP-4 D1: attach body-offload when configured — a fresh raw DiskBlobStore handle over the shared
        // `[blob]` CAS root (own handle per build, since `append`/`maybe_offload` are `&mut`).
        let sink = match &self.offload {
            Some((blob_dir, threshold)) => {
                sink.with_offload(Box::new(Self::open_offload_blob(blob_dir)?), *threshold)
            }
            None => sink,
        };
        Ok(Some(Box::new(sink)))
    }

    /// Read back this session's Dynamo-partition log for boot-recovery (§lifecycle I4b). Builds the same
    /// per-session sink `build` does, then `read_recovered` Queries the `session_id` partition ascending +
    /// decodes to a `Recovered` — ending the good prefix at the first undecodable item + flagging Corrupt
    /// (the I4b default-#3 contract), NOT hard-erroring like `read_all`. An empty partition recovers as a
    /// clean empty log (the caller treats a genesis-less recovery as nothing to resurrect). A Query/transport
    /// failure is `Err`.
    async fn recover(
        &self,
        id: &crate::host::SessionId,
    ) -> Result<Option<cdz_kernel::log_store::Recovered>, String> {
        let sink = DynamoLogSink::from_conf(&self.config, self.table.clone(), id.hash());
        // GAP-4 D1: rehydrate offloaded bodies on recovery — attach the same offload handle (get-only on the
        // read path, but the sink field holds a Box either way) over the shared `[blob]` CAS root.
        let sink = match &self.offload {
            Some((blob_dir, threshold)) => {
                sink.with_offload(Box::new(Self::open_offload_blob(blob_dir)?), *threshold)
            }
            None => sink,
        };
        sink.read_recovered().await.map(Some).map_err(|e| {
            format!(
                "could not recover Dynamo log for session {}: {e}",
                id.to_base64url()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::LogSinkBuilder; // the trait, so `.build(...)` resolves on the builder
    use cdz_kernel::event::EventBody;
    use cdz_kernel::hash::Hash;

    /// A minimal well-formed event that round-trips through `event_ast::encode/decode` — a Genesis at
    /// `seq` (or a later Inbound), so the pure recovery-adapter test uses real canonical bytes.
    fn genesis_event(seq: u64) -> Event {
        Event {
            seq,
            cause: None,
            body: EventBody::Genesis {
                reducer: Hash::of(b"reducer"),
                spawn_nonce: Hash::of(b"nonce"),
                parent: None,
            },
        }
    }

    fn test_cfg() -> aws_config::SdkConfig {
        // A minimal config never makes a call until an operation runs — lets us construct the sink/builder
        // and exercise the pure key/attr shape without touching AWS.
        aws_config::SdkConfig::builder()
            .behavior_version(aws_config::BehaviorVersion::latest())
            .build()
    }

    #[test]
    fn sink_holds_the_table_and_session_partition() {
        let sink = DynamoLogSink::from_conf(&test_cfg(), "cdz-log", Hash::of(b"worker-1"));
        assert_eq!(sink.table, "cdz-log");
        assert_eq!(sink.session_id, Hash::of(b"worker-1"));
    }

    #[tokio::test]
    async fn builder_builds_a_sink_per_session_id() {
        let builder = DynamoLogSinkBuilder::from_conf(test_cfg(), "cdz-log");
        // The builder yields a sink (Some) for any session id — the daemon attaches it as the durable log.
        let sink = builder
            .build(&crate::host::SessionId::new(Hash::of(b"worker-2")))
            .await
            .expect("builder succeeds");
        assert!(
            sink.is_some(),
            "dynamo builder always yields a durable sink"
        );
    }

    #[test]
    fn recovered_all_clean_yields_clean_prefix() {
        // Every stored blob decodes → a Clean read of the whole good prefix, in order.
        let e0 = event_ast::encode(&genesis_event(0));
        let e1 = event_ast::encode(&genesis_event(1));
        let expected_len = (e0.len() + e1.len()) as u64;
        let rec = recovered_from_event_blobs([e0.as_slice(), e1.as_slice()]);
        assert_eq!(rec.kind, RecoveryKind::Clean);
        assert_eq!(rec.events.len(), 2);
        assert_eq!(rec.events[0].seq, 0);
        assert_eq!(rec.events[1].seq, 1);
        assert_eq!(rec.good_prefix_len, expected_len);
        assert!(!rec.is_corrupt());
    }

    #[test]
    fn recovered_stops_at_first_corrupt_and_flags_corrupt() {
        // A bad blob after a good prefix (I4b default #3): keep the good prefix, mark Corrupt, DON'T include
        // the bad one — the boot loop then alarms + skips this session while recovering the healthy rest.
        let good = event_ast::encode(&genesis_event(0));
        let good_len = good.len() as u64;
        let garbage: &[u8] = b"\xff\xff not a valid canonical event frame";
        let after = event_ast::encode(&genesis_event(2)); // must NOT be reached
        let rec = recovered_from_event_blobs([good.as_slice(), garbage, after.as_slice()]);
        assert_eq!(rec.kind, RecoveryKind::Corrupt);
        assert!(rec.is_corrupt());
        assert_eq!(
            rec.events.len(),
            1,
            "only the pre-corruption good prefix is kept"
        );
        assert_eq!(rec.events[0].seq, 0);
        assert_eq!(rec.good_prefix_len, good_len);
    }

    #[test]
    fn recovered_empty_is_clean_empty() {
        // No stored events → an empty Clean Recovered. recover_from turns this into EmptyLog (caller
        // genesis()es fresh) — the pure adapter just reports "nothing, cleanly".
        let rec = recovered_from_event_blobs(std::iter::empty::<&[u8]>());
        assert_eq!(rec.kind, RecoveryKind::Clean);
        assert!(rec.events.is_empty());
        assert_eq!(rec.good_prefix_len, 0);
    }

    #[test]
    fn recovered_corrupt_first_item_yields_empty_corrupt() {
        // Corruption at the very head: no good prefix, Corrupt kind — the whole session's log is unusable,
        // the boot loop alarms and skips it (never silently drops it).
        let garbage: &[u8] = b"not an event";
        let rec = recovered_from_event_blobs([garbage]);
        assert_eq!(rec.kind, RecoveryKind::Corrupt);
        assert!(rec.events.is_empty());
        assert_eq!(rec.good_prefix_len, 0);
    }

    #[test]
    fn attr_names_are_stable() {
        // append + read_all must agree on the schema; pin the names so a rename can't silently split them.
        assert_eq!(
            (ATTR_SESSION, ATTR_SEQ, ATTR_EVENT),
            ("session_id", "seq", "event")
        );
    }

    #[tokio::test]
    async fn offload_rehydrate_body_round_trips_inline_and_blob_ptr() {
        // GAP-4 D1 Dynamo offload seam (hermetic — no live DynamoDB): `maybe_offload` writes an over-threshold
        // body to the content-addressed CAS + returns a (blob-ptr) frame; a sink configured with the SAME
        // store rehydrates it BYTE-IDENTICAL on the read path, and passes a sub-threshold body through inline.
        use cdz_kernel::blob::MemBlobStore;
        let mut blob = MemBlobStore::new();
        let threshold = 8usize;

        let big = vec![7u8; 64];
        let stored_big = maybe_offload(&big, &mut blob, threshold).await.unwrap();
        assert_ne!(
            stored_big, big,
            "an over-threshold body offloads to a (blob-ptr) frame, not stored inline"
        );
        let small = b"tiny".to_vec();
        let stored_small = maybe_offload(&small, &mut blob, threshold).await.unwrap();
        assert_eq!(stored_small, small, "a sub-threshold body stays inline");

        // The same store, moved into a sink, rehydrates both on the read path.
        let sink = DynamoLogSink::from_conf(&test_cfg(), "cdz-log", Hash::of(b"w"))
            .with_offload(Box::new(blob), threshold);
        assert_eq!(
            sink.rehydrate_body(stored_big).await.unwrap(),
            big,
            "the offloaded body derefs byte-identical from the CAS"
        );
        assert_eq!(
            sink.rehydrate_body(stored_small).await.unwrap(),
            small,
            "an inline body passes through rehydrate unchanged"
        );
    }

    #[tokio::test]
    async fn rehydrate_body_without_offload_is_identity() {
        // A sink with NO offload returns the stored bytes verbatim (pre-D1: nothing to deref).
        let sink = DynamoLogSink::from_conf(&test_cfg(), "cdz-log", Hash::of(b"w"));
        let bytes = b"an inline event body".to_vec();
        assert_eq!(sink.rehydrate_body(bytes.clone()).await.unwrap(), bytes);
    }

    #[tokio::test]
    async fn rehydrate_body_with_a_missing_offloaded_blob_is_a_hard_not_found() {
        // A (blob-ptr) frame whose body is ABSENT from the CAS is a hard NotFound (data loss — never a silent
        // empty body), pinning the rehydrate contract on the Dynamo read/recovery path.
        use cdz_kernel::blob::MemBlobStore;
        let mut src = MemBlobStore::new();
        let ptr = maybe_offload(&[9u8; 64], &mut src, 8).await.unwrap();
        // A DIFFERENT, empty store has no such blob → rehydrate errors NotFound.
        let sink = DynamoLogSink::from_conf(&test_cfg(), "cdz-log", Hash::of(b"w"))
            .with_offload(Box::new(MemBlobStore::new()), 8);
        let err = sink.rehydrate_body(ptr).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}

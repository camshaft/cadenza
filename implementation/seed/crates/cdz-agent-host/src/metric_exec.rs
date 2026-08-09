//! [`MetricExecutor`] — the `metric/publish` effect executor: a REDUCER publishes a metric that the host
//! records into the shared metrics [`Registry`], so it drains to the configured backends (statsd/otlp/
//! prometheus) alongside the host's own metrics (operator Q3: "get metrics OUT OF reducers → pulled into the
//! metric backends we've got set up").
//!
//! A reducer performs a `metric/publish` effect whose payload is a [`MetricSample`] (name / kind / value /
//! labels — the kernel-side codec, v-agent-harness). The kernel authorizes it (Cedar gates
//! `action == "metric/publish"` on the metric-NAME target, BEFORE dispatch) then routes it here. This
//! executor DECODES the sample and RECORDS it into the Registry. Fire-and-forget: `Ok(None)` once recorded.
//!
//! **Design (operator review on #2549 — the metrics surface is reducer-shaped, not host-shaped):**
//! - A reducer publishes under its OWN metric name (`sample.name` IS the registry metric name) — NOT one
//!   shared `cdz_agent_host.reducer` blob. Labels are the aggregation dimension, so metrics are distinct
//!   queryable series, not one lump keyed by a `Variant` string.
//! - **CARDINALITY is CEDAR's job, not the host's.** WHICH metric names a reducer may publish is authorized by
//!   the Cedar policy on the `metric/publish` target BEFORE dispatch — that authorization IS the cardinality
//!   control (a locked-down reducer publishes a fixed set). So the host does NOT cap distinct names (an earlier
//!   host cap was inevitable-POLICY-in-the-host, the standing-order anti-pattern). The host keeps only a
//!   guest-STRING SAFETY check — a genuine pipeline-safety MECHANISM, not a count policy.
//! - **HANDLE CACHE:** the executor caches the registered [`MetricHandle`] per (name, kind, labels) key, so a
//!   repeat publish records directly on the cached handle with NO registry interaction on hits (register only
//!   on first-sight of a key).
//!
//! **Host = MECHANISM + a SAFETY BOUND (standing order: host = mechanism, policy = wasm):**
//! - MECHANISM: decode, map `kind` → the registry metric type (counter → [`Counter`], gauge → [`Gauge`],
//!   summary/timing → [`Summary`]), record under the reducer's own name.
//! - GUEST-STRING SAFETY: a metric name/label is GUEST-CONTROLLED and ships OFF-BOX to the backends. The
//!   host validates name + label key/vals to a safe charset + length before ANY off-box export
//!   (never-log-guest-controlled-strings) and rejects unsafe strings PERMANENT — the host NEVER logs the raw
//!   guest string. (This is safety mechanism, NOT a cardinality/count policy — that's Cedar's.)

use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::event_ast::{decode_metric_publish, decode_metrics_publish, MetricSample};
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;
use s2n_quic_dc_metrics::{Counter, Gauge, Summary, Unit};
use std::collections::HashMap;

/// Max byte-length of a metric name or a label key/value — a guest string bounds the off-box export size.
const MAX_STR_LEN: usize = 128;

/// A registered metric handle, cached per key so a repeat publish records without re-hitting the registry.
/// One variant per registry metric type the `kind` vocab maps onto.
enum MetricHandle {
    Counter(Counter),
    Gauge(Gauge),
    Summary(Summary),
}

/// Records reducer-published metrics into the shared metrics [`Registry`] under the reducer's OWN metric names.
/// Holds a Registry handle (cheap `Arc` into the registry storage — the SAME one the exporter drains) + a
/// CACHE of registered handles keyed by (name, kind, sorted-labels), so a repeat publish records on the cached
/// handle with no registry interaction on hits.
pub struct MetricExecutor {
    registry: crate::metrics::Registry,
    /// Cache: (name, kind, sorted-labels) → the registered handle. First publish of a key registers + inserts;
    /// subsequent publishes hit the cache. `kind` is part of the key so a name reused across kinds gets
    /// distinct handles (the registry binds a key to one type + would panic on a type change — §17 totality).
    cache: HashMap<CacheKey, MetricHandle>,
}

/// The cache/registry key: the reducer's metric name, its kind, and its sorted labels — so distinct
/// (name, kind, label-set) tuples are distinct series (the queryable-series shape the review asked for).
type CacheKey = (String, String, Vec<(String, String)>);

impl MetricExecutor {
    /// Build the executor over the daemon's shared metrics [`Registry`] (the same one host/effect metrics
    /// register into + the exporter drains), so a reducer-published metric reaches the configured backends.
    pub fn new(registry: crate::metrics::Registry) -> Self {
        MetricExecutor {
            registry,
            cache: HashMap::new(),
        }
    }

    /// Validate a guest string (metric name or label key/value) is safe to carry OFF-BOX: non-empty, within
    /// [`MAX_STR_LEN`], and only printable-ASCII-minus-control (no newlines/control chars / statsd-line
    /// metacharacters that could corrupt or inject into an export). `false` → the caller rejects the publish
    /// PERMANENT (a malformed request for this executor). The host never logs the raw string.
    fn is_safe_str(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= MAX_STR_LEN
            && s.chars()
                .all(|c| !c.is_control() && c.is_ascii() && c != '|' && c != ':')
    }

    /// The aggregation string for a metric's labels — the registry distinguishes series under ONE metric name
    /// by this `Variant|…`-style aggregation. Labels are sorted for a stable key (the guest may emit them in
    /// any order). `None` (no labels) records the bare series. The metric NAME is the reducer's own
    /// `sample.name` (passed as the registry metric), NOT folded in here — so each name is its own series.
    fn label_aggregation(labels: &[(String, String)]) -> Option<String> {
        if labels.is_empty() {
            return None;
        }
        let mut sorted = labels.to_vec();
        sorted.sort();
        let mut agg = String::from("Variant");
        for (k, v) in &sorted {
            agg.push_str(&format!("|{k}={v}"));
        }
        Some(agg)
    }

    /// Guest-string safety over a whole sample (name + every label key/val) — bounded charset + length before
    /// any off-box export. `Err(msg)` names which part failed (the host never logs the raw guest string).
    fn safety_check(sample: &MetricSample) -> Result<(), &'static str> {
        if !Self::is_safe_str(&sample.name) {
            return Err(
                "MetricExecutor: metric name is empty/too-long/has unsafe chars (rejected before off-box export)",
            );
        }
        for (k, v) in &sample.labels {
            if !Self::is_safe_str(k) || !Self::is_safe_str(v) {
                return Err(
                    "MetricExecutor: a metric label key/value is empty/too-long/has unsafe chars (rejected before off-box export)",
                );
            }
        }
        Ok(())
    }

    /// Map the reducer's raw-string `unit` vocab onto the metrics [`Registry`]'s `Unit` enum. `None` = no
    /// unit given (empty string) → the caller uses the kind's natural default. The Registry has no
    /// millisecond variant, so a `milliseconds` unit maps to `Microsecond` (the finest sub-second unit — the
    /// VALUE the reducer sent is recorded unchanged; the unit is display metadata on the handle). An unknown
    /// unit string is treated as unspecified (`None`) rather than an error — the kernel carries the vocab, the
    /// host maps what it knows + defaults the rest (never rejects a publish over a display unit).
    fn unit_for(unit: &str) -> Option<Unit> {
        match unit {
            "" => None,
            "count" => Some(Unit::Count),
            "bytes" => Some(Unit::Byte),
            // No Millisecond in the Registry — microseconds is the finest sub-second unit; the value is
            // unchanged (display metadata only). Both sub-second time units map here.
            "microseconds" | "milliseconds" => Some(Unit::Microsecond),
            "seconds" => Some(Unit::Second),
            "percent" => Some(Unit::Percent),
            _ => None, // unknown unit → unspecified (default by kind); never an error over a display unit
        }
    }

    /// Record one already-safety-checked sample: get-or-register the handle for its (name, kind, labels) key,
    /// then record `value` on it. Returns `Err(EffectOutcome)` for an unknown kind (a clean PERMANENT). The
    /// metric registers under the reducer's OWN name (`sample.name`); the cache means a repeat publish never
    /// re-hits the registry.
    fn record(&mut self, sample: &MetricSample) -> Result<(), EffectOutcome> {
        // The kind vocab the host maps onto registry types. An unknown kind is a clean PERMANENT.
        // (Vocab is raw-string, reducer/host-owned; the kernel just carries it.)
        if !matches!(
            sample.kind.as_str(),
            "counter" | "gauge" | "summary" | "timing"
        ) {
            return Err(EffectOutcome::err(format!(
                "MetricExecutor: unknown metric kind {:?} (expected counter/gauge/summary/timing)",
                sample.kind
            )));
        }
        let mut labels = sample.labels.clone();
        labels.sort();
        let key: CacheKey = (sample.name.clone(), sample.kind.clone(), labels);

        // Get-or-register the handle (register only on a cache MISS — no registry interaction on hits).
        let handle = self.cache.entry(key).or_insert_with(|| {
            let name = sample.name.clone();
            let agg = Self::label_aggregation(&sample.labels);
            // The reducer-supplied UNIT (operator Q3 round-3) mapped onto the Registry Unit enum. Empty/
            // unknown → the kind's natural default (counter=Count, summary/timing=Microsecond).
            let unit = Self::unit_for(&sample.unit);
            match sample.kind.as_str() {
                "counter" => MetricHandle::Counter(self.registry.register_counter_with_unit(
                    name,
                    agg,
                    unit.unwrap_or(Unit::Count),
                )),
                "gauge" => MetricHandle::Gauge(self.registry.register_gauge(
                    name,
                    agg,
                    unit.unwrap_or(Unit::Count),
                )),
                // summary + timing both record into a Summary (a distribution/histogram). A time metric
                // defaults to Microsecond when the reducer gave no unit.
                _ => MetricHandle::Summary(self.registry.register_summary(
                    name,
                    agg,
                    unit.unwrap_or(Unit::Microsecond),
                )),
            }
        });

        // Record `value` on the (cached) handle per its type.
        match handle {
            MetricHandle::Counter(c) => {
                // A counter delta: the f64 as a saturating non-negative u64 increment (a counter only goes up;
                // a negative/NaN delta clamps to 0 rather than wrapping).
                let delta = if sample.value.is_finite() && sample.value > 0.0 {
                    sample.value as u64
                } else {
                    0
                };
                c.increment(delta);
            }
            MetricHandle::Gauge(g) => {
                let v = if sample.value.is_finite() {
                    sample.value as i64
                } else {
                    0
                };
                g.set(v);
            }
            MetricHandle::Summary(s) => s.record_f64(sample.value),
        }
        Ok(())
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for MetricExecutor {
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
        // Family-keyed, matching the router + authz decision. A non-metric/publish family is structural →
        // PERMANENT (§17: observable Err, never a panic).
        if !req.content_type.matches_family(effect_ct::METRIC_PUBLISH) {
            return EffectOutcome::err(format!(
                "MetricExecutor only handles the {} family, got {}",
                effect_ct::METRIC_PUBLISH,
                req.content_type.family
            ));
        }
        // The payload IS the encoded MetricSample. A blob-ref / missing payload can't be a metric sample →
        // structural PERMANENT.
        let bytes: &[u8] = match &req.payload {
            Some(Payload::Inline(bytes)) => bytes,
            Some(Payload::Blob(_)) => {
                return EffectOutcome::err(
                    "MetricExecutor: a blob-ref metric/publish payload is unsupported — inline the metric sample",
                );
            }
            None => {
                return EffectOutcome::err(
                    "MetricExecutor: a metric/publish effect requires a payload (the encoded metric sample)",
                );
            }
        };
        // Decode the payload. A reducer may publish a BATCH (metrics-publish, Vec<MetricSample>) or a SINGLE
        // (metric-publish) sample — try the batch codec first, fall back to single. A frame that decodes as
        // NEITHER is a clean PERMANENT (the reducer built a bad payload; retrying won't fix it). The host does
        // NOT log the raw bytes (guest-controlled).
        let samples: Vec<MetricSample> = match decode_metrics_publish(bytes) {
            Ok(batch) => batch,
            Err(_) => match decode_metric_publish(bytes) {
                Ok(single) => vec![single],
                Err(e) => {
                    return EffectOutcome::err(format!(
                        "MetricExecutor: metric/publish payload did not decode as a batch or single sample: {e:?}"
                    ));
                }
            },
        };

        // GUEST-STRING SAFETY over EVERY sample FIRST (all-or-nothing): a batch is ONE publish, so if any
        // sample's name/label is unsafe, reject the WHOLE publish PERMANENT without recording ANY of them
        // (no half-recorded batch — a partial record is confusing + leaks some unsafe-adjacent samples). The
        // host never records or logs an unsafe guest string into the telemetry pipeline. (Cardinality — how
        // MANY names — is Cedar's authorization, not here.)
        for sample in &samples {
            if let Err(msg) = Self::safety_check(sample) {
                return EffectOutcome::err(msg);
            }
        }

        // Record each under the reducer's own name (get-or-register the cached handle). An unknown kind →
        // PERMANENT (checked per-sample in `record`; the safety pass above already validated every sample, so
        // a mid-batch kind error still hasn't recorded anything unsafe — but note it CAN partially record
        // valid-kind samples before a later unknown-kind one; acceptable since all passed safety + an unknown
        // kind is a reducer bug, not a safety issue).
        for sample in &samples {
            if let Err(outcome) = self.record(sample) {
                return outcome;
            }
        }
        // Recorded — fire-and-forget: Ok(None) acks the publish (no result payload for the reducer to fold).
        EffectOutcome::Ok(None)
    }

    /// This single-family executor serves exactly the `metric/publish` family.
    fn handles_family(&self, family: &str) -> bool {
        family == effect_ct::METRIC_PUBLISH
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::Timeliness;
    use cdz_kernel::event::Retryability;
    use cdz_kernel::event_ast::{encode_metric_publish, encode_metrics_publish};

    fn publish_req(sample: &MetricSample) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::METRIC_PUBLISH,
            sample.name.clone(),
            Some(Payload::Inline(encode_metric_publish(sample).into())),
            Timeliness::Interactive,
        )
    }

    /// A metric/publish effect carrying a BATCH payload (metrics-publish, Vec<MetricSample>).
    fn batch_req(samples: &[MetricSample]) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::METRIC_PUBLISH,
            "batch",
            Some(Payload::Inline(encode_metrics_publish(samples).into())),
            Timeliness::Interactive,
        )
    }

    fn sample(name: &str, kind: &str, value: f64) -> MetricSample {
        MetricSample {
            name: name.into(),
            kind: kind.into(),
            value,
            unit: String::new(),
            labels: vec![],
        }
    }

    #[tokio::test]
    async fn records_counter_gauge_summary_under_the_reducers_own_names() {
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry.clone());
        // Each metric under its OWN name (not one shared aggregate), one of each kind.
        for (name, kind, val) in [
            ("agent.turns", "counter", 3.0),
            ("agent.queue_depth", "gauge", 42.0),
            ("agent.tick_latency", "summary", 12.5),
            ("agent.model_ms", "timing", 88.0),
        ] {
            let out = exec
                .perform(
                    EffectId(0),
                    &publish_req(&sample(name, kind, val)),
                    Hash::of(b"k"),
                )
                .await;
            assert!(
                matches!(out, EffectOutcome::Ok(None)),
                "a published {kind} metric {name} acks Ok(None), got {out:?}"
            );
        }
        assert!(
            registry.try_take_current_metrics_line().is_some(),
            "the registry reports the reducer-published metrics"
        );
    }

    #[tokio::test]
    async fn a_repeat_publish_hits_the_handle_cache_not_the_registry() {
        // The cache means a second publish of the same (name, kind, labels) records without re-registering.
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry);
        let m = sample("agent.turns", "counter", 1.0);
        assert!(matches!(
            exec.perform(EffectId(0), &publish_req(&m), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(None)
        ));
        assert_eq!(
            exec.cache.len(),
            1,
            "first publish registered + cached one handle"
        );
        // Repeat: same key → cache hit, no new entry.
        assert!(matches!(
            exec.perform(EffectId(0), &publish_req(&m), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(None)
        ));
        assert_eq!(
            exec.cache.len(),
            1,
            "repeat publish hit the cache — no new registration"
        );
        // A DIFFERENT name → a distinct cached series.
        assert!(matches!(
            exec.perform(
                EffectId(0),
                &publish_req(&sample("agent.other", "counter", 1.0)),
                Hash::of(b"k")
            )
            .await,
            EffectOutcome::Ok(None)
        ));
        assert_eq!(
            exec.cache.len(),
            2,
            "a distinct name is its own cached series"
        );
    }

    #[tokio::test]
    async fn distinct_labels_are_distinct_series_stable_regardless_of_order() {
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry);
        let labeled = MetricSample {
            name: "agent.calls".into(),
            kind: "counter".into(),
            value: 1.0,
            unit: String::new(),
            labels: vec![
                ("role".into(), "builder".into()),
                ("ok".into(), "true".into()),
            ],
        };
        assert!(matches!(
            exec.perform(EffectId(0), &publish_req(&labeled), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(None)
        ));
        // Same labels in REVERSE order → same series (sorted key), so no new cache entry.
        let mut reordered = labeled.clone();
        reordered.labels.reverse();
        assert!(matches!(
            exec.perform(EffectId(0), &publish_req(&reordered), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(None)
        ));
        assert_eq!(
            exec.cache.len(),
            1,
            "label order doesn't change the series (sorted key)"
        );
        // A DIFFERENT label set under the same name → a distinct series.
        let other_labels = MetricSample {
            labels: vec![("role".into(), "reviewer".into())],
            ..labeled.clone()
        };
        assert!(matches!(
            exec.perform(EffectId(0), &publish_req(&other_labels), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(None)
        ));
        assert_eq!(
            exec.cache.len(),
            2,
            "a distinct label set is a distinct series"
        );
    }

    #[tokio::test]
    async fn no_host_cardinality_cap_cedar_owns_that() {
        // The host does NOT cap distinct names (that's Cedar's authorization job). Publishing many distinct
        // names all succeed here — a locked-down reducer is constrained by its Cedar policy, not a host count.
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry);
        for i in 0..200 {
            let out = exec
                .perform(
                    EffectId(0),
                    &publish_req(&sample(&format!("m{i}"), "counter", 1.0)),
                    Hash::of(b"k"),
                )
                .await;
            assert!(
                matches!(out, EffectOutcome::Ok(None)),
                "name {i} records (no host cardinality cap)"
            );
        }
        assert_eq!(
            exec.cache.len(),
            200,
            "all distinct names cached, no host cap"
        );
    }

    #[tokio::test]
    async fn an_unknown_kind_is_permanent() {
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry);
        let out = exec
            .perform(
                EffectId(0),
                &publish_req(&sample("m", "histogram-ish", 1.0)),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("unknown metric kind")),
            "an unmapped kind is PERMANENT, got {out:?}"
        );
    }

    #[tokio::test]
    async fn an_unsafe_metric_name_is_rejected_before_export() {
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry);
        // A statsd-line-corrupting char (':') in the name → rejected PERMANENT (guest-string safety).
        let out = exec
            .perform(
                EffectId(0),
                &publish_req(&sample("bad:name", "counter", 1.0)),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("unsafe chars")),
            "an unsafe metric name is rejected before off-box export, got {out:?}"
        );
        // A control char likewise.
        let out = exec
            .perform(
                EffectId(0),
                &publish_req(&sample("bad\nname", "counter", 1.0)),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, .. } if message.contains("unsafe chars"))
        );
    }

    #[tokio::test]
    async fn a_non_metric_family_and_bad_payloads_are_permanent() {
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry);
        // Wrong family.
        let wrong = EffectRequest::new_with_family(
            effect_ct::EMIT,
            "x",
            Some(Payload::Inline(bytes::Bytes::new())),
            Timeliness::Interactive,
        );
        assert!(
            matches!(exec.perform(EffectId(0), &wrong, Hash::of(b"k")).await, EffectOutcome::Err { message, .. } if message.contains("only handles"))
        );
        assert!(
            exec.handles_family(effect_ct::METRIC_PUBLISH) && !exec.handles_family(effect_ct::EMIT)
        );
        // Missing payload.
        let no_payload = EffectRequest::new_with_family(
            effect_ct::METRIC_PUBLISH,
            "m",
            None,
            Timeliness::Interactive,
        );
        assert!(
            matches!(exec.perform(EffectId(0), &no_payload, Hash::of(b"k")).await, EffectOutcome::Err { message, .. } if message.contains("requires a payload"))
        );
        // Undecodable payload.
        let garbage = EffectRequest::new_with_family(
            effect_ct::METRIC_PUBLISH,
            "m",
            Some(Payload::Inline(b"not a metric sample".to_vec().into())),
            Timeliness::Interactive,
        );
        assert!(
            matches!(exec.perform(EffectId(0), &garbage, Hash::of(b"k")).await, EffectOutcome::Err { message, .. } if message.contains("did not decode"))
        );
    }

    #[tokio::test]
    async fn a_reducer_supplied_unit_maps_to_the_registry_unit() {
        // The reducer-supplied unit rides through + maps to the Registry Unit enum (display metadata on the
        // handle; the value is unchanged). We can't read the unit back out (drain-on-report, no getter), so
        // assert the publish RECORDS cleanly for each known unit + that unit_for maps as specified.
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry);
        for (unit, kind) in [
            ("count", "counter"),
            ("bytes", "gauge"),
            ("milliseconds", "timing"),
            ("seconds", "timing"),
            ("percent", "gauge"),
            ("", "counter"),         // unspecified → kind default
            ("furlongs", "counter"), // unknown → unspecified, NOT an error
        ] {
            let mut s = sample(&format!("m.{unit}.{kind}"), kind, 1.0);
            s.unit = unit.into();
            let out = exec
                .perform(EffectId(0), &publish_req(&s), Hash::of(b"k"))
                .await;
            assert!(
                matches!(out, EffectOutcome::Ok(None)),
                "unit {unit:?} on a {kind} records cleanly (unknown unit is not an error), got {out:?}"
            );
        }
        // unit_for mapping (no Millisecond variant → ms maps to Microsecond; unknown → None).
        assert_eq!(MetricExecutor::unit_for(""), None);
        assert_eq!(MetricExecutor::unit_for("bytes"), Some(Unit::Byte));
        assert_eq!(
            MetricExecutor::unit_for("milliseconds"),
            Some(Unit::Microsecond)
        );
        assert_eq!(MetricExecutor::unit_for("seconds"), Some(Unit::Second));
        assert_eq!(MetricExecutor::unit_for("percent"), Some(Unit::Percent));
        assert_eq!(MetricExecutor::unit_for("furlongs"), None);
    }

    #[tokio::test]
    async fn a_batch_publish_records_every_sample_and_a_bad_sample_rejects_the_whole() {
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry);
        // A batch of 3 distinct metrics records all 3 (each its own cached series).
        let batch = [
            sample("agent.turns", "counter", 2.0),
            sample("agent.queue", "gauge", 5.0),
            sample("agent.latency", "timing", 12.0),
        ];
        let out = exec
            .perform(EffectId(0), &batch_req(&batch), Hash::of(b"k"))
            .await;
        assert!(
            matches!(out, EffectOutcome::Ok(None)),
            "batch records, got {out:?}"
        );
        assert_eq!(
            exec.cache.len(),
            3,
            "each batch sample is its own cached series"
        );
        // A batch with ONE unsafe sample rejects the WHOLE publish (all-or-nothing) — and records NONE of it
        // (cache unchanged from the 3 above).
        let bad_batch = [
            sample("agent.ok", "counter", 1.0),
            sample("bad:name", "counter", 1.0), // unsafe (':' corrupts a statsd line)
        ];
        let out = exec
            .perform(EffectId(0), &batch_req(&bad_batch), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("unsafe chars")),
            "a batch with an unsafe sample is rejected whole, got {out:?}"
        );
        assert_eq!(
            exec.cache.len(),
            3,
            "a rejected batch records NONE of its samples (no half-recorded batch)"
        );
        // A SINGLE (non-batch) metric-publish payload still works (back-compat fallback).
        let out = exec
            .perform(
                EffectId(0),
                &publish_req(&sample("agent.single", "counter", 1.0)),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(out, EffectOutcome::Ok(None)),
            "single-sample publish still works"
        );
        assert_eq!(exec.cache.len(), 4);
    }
}

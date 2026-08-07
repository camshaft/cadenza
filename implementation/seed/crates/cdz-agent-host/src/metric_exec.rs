//! [`MetricExecutor`] — the `metric/publish` effect executor: a REDUCER publishes a metric that the host
//! records into the shared metrics [`Registry`], so it drains to the configured backends (statsd/otlp/
//! prometheus) alongside the host's own metrics (operator Q3: "get metrics OUT OF reducers → pulled into the
//! metric backends we've got set up").
//!
//! A reducer performs a `metric/publish` effect whose payload is a [`MetricSample`] (name / kind / value /
//! labels — the kernel-side codec, v-agent-harness). The kernel authorizes it (Cedar gates
//! `action == "metric/publish"` on the metric-NAME target, BEFORE dispatch — WHICH metrics a session may
//! publish is evolvable policy on the log) then routes it here. This executor DECODES the sample and RECORDS
//! it into the Registry. Fire-and-forget: it returns `Ok(None)` once recorded (a metric publish has no
//! meaningful result payload for the reducer to fold).
//!
//! **The host's job is MECHANISM + a SAFETY BOUND (operator standing-order: host = mechanism, policy = wasm).**
//! - MECHANISM: decode the sample, map `kind` → the registry's metric type (counter → [`Counter::increment`],
//!   gauge → [`Gauge::set`], timing → [`Summary::record_value`]), record it.
//! - 🔴 SAFETY BOUND (why this executor holds state): a metric name/label is GUEST-CONTROLLED and ships
//!   OFF-BOX to the metric backends. Two hazards the host must bound (a bound is mechanism, not policy) —
//!   CARDINALITY: an unbounded set of distinct guest names/labels is a cardinality bomb (registry bloat +
//!   backend cost), so the executor caps the number of DISTINCT metric names it admits per session; and
//!   GUEST-STRING SAFETY: a guest string must not carry control chars / unbounded length into an off-box
//!   telemetry export (never-log-guest-controlled-strings), so names/labels are validated to a safe charset +
//!   length and rejected (PERMANENT) otherwise, and the host NEVER logs the raw guest string. Which NAMES are
//!   allowed is Cedar policy (the metric-name target authz); the host only enforces the structural cardinality
//!   + charset bound that keeps the telemetry pipeline safe regardless of policy.

use cdz_kernel::effect::{effect_ct, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::event_ast::{decode_metric_publish, MetricSample};
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;
use std::collections::HashSet;

/// Max distinct metric NAMES a single session may publish (the cardinality cap — a bound on the guest's
/// contribution to registry/backend cardinality). A session publishing more distinct names than this has its
/// further-new names rejected (PERMANENT); already-admitted names keep recording. Generous for legitimate use
/// (a handful of well-known metrics per role), tight enough that a runaway/hostile guest can't explode
/// cardinality. Not config-tunable in v1 (a fixed safety bound; a policy-driven cap would be a later slice).
const MAX_DISTINCT_NAMES: usize = 64;

/// Max byte-length of a metric name or a label key/value — a guest string bounds the off-box export size.
const MAX_STR_LEN: usize = 128;

/// Records reducer-published metrics into the shared metrics [`Registry`], applying the host cardinality +
/// guest-string-safety bound. Holds a Registry handle (cheap `Arc` into the registry storage — the SAME one
/// the exporter drains) + the set of metric names it has admitted for this session (the cardinality guard).
pub struct MetricExecutor {
    registry: crate::metrics::Registry,
    /// The distinct metric names admitted so far (cardinality guard). A new name beyond [`MAX_DISTINCT_NAMES`]
    /// is rejected; an already-admitted name records without growing the set.
    admitted: HashSet<String>,
}

impl MetricExecutor {
    /// Build the executor over the daemon's shared metrics [`Registry`] (the same one host/effect metrics
    /// register into + the exporter drains), so a reducer-published metric reaches the configured backends.
    pub fn new(registry: crate::metrics::Registry) -> Self {
        MetricExecutor {
            registry,
            admitted: HashSet::new(),
        }
    }

    /// Validate a guest string (metric name or label key/value) is safe to carry OFF-BOX: non-empty, within
    /// [`MAX_STR_LEN`], and only printable-ASCII-minus-control (no newlines/control chars that could corrupt a
    /// statsd line / injection into an export). Returns `false` for anything unsafe — the caller rejects the
    /// whole publish PERMANENT (a malformed request for this executor). The host never logs the raw string.
    fn is_safe_str(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= MAX_STR_LEN
            && s.chars()
                .all(|c| !c.is_control() && c.is_ascii() && c != '|' && c != ':')
    }

    /// The `Variant|…`-style aggregation string a published metric records under, so it groups distinctly in
    /// the registry (the same convention the host's own metrics use). Labels ride as a stable-ordered suffix.
    ///
    /// The `kind` is folded into the key: the registry binds each `(metric, aggregation)` key to ONE metric
    /// TYPE and PANICS on a type change for the same key. A guest could reuse a name across kinds (a `counter`
    /// then a `gauge` "agent.turns"); keying by kind gives them distinct registry entries so that can never
    /// crash the host (§17 totality — a guest must not be able to panic the daemon). Distinct kinds of the
    /// same name simply become distinct series.
    fn aggregation_for(sample: &MetricSample) -> String {
        // Labels are sorted for a stable aggregation key (the guest may emit them in any order); the safety
        // check already bounded their charset + count-per-name via the name cardinality cap.
        let mut labels = sample.labels.clone();
        labels.sort();
        let mut agg = format!("Variant|{}|{}", sample.kind, sample.name);
        for (k, v) in &labels {
            agg.push_str(&format!(",{k}={v}"));
        }
        agg
    }
}

/// The metric name every reducer-published metric aggregates under (distinct from the host's own
/// `cdz_agent_host.session`/`.effect` metrics — reducer metrics are a separate namespace an operator can
/// filter on). The individual metric is distinguished by the `Variant|<name>,<labels>` aggregation.
const REDUCER_METRIC: &str = "cdz_agent_host.reducer";

#[async_trait::async_trait(?Send)]
impl Executor for MetricExecutor {
    async fn perform(&mut self, req: &EffectRequest, _idempotency_key: Hash) -> EffectOutcome {
        // Family-keyed, matching the router + authz decision. A non-metric/publish family is structural →
        // PERMANENT (§17: observable Err, never a panic).
        if !req.content_type.matches_family(effect_ct::METRIC_PUBLISH) {
            return EffectOutcome::Err(crate::retry::permanent(format!(
                "MetricExecutor only handles the {} family, got {}",
                effect_ct::METRIC_PUBLISH,
                req.content_type.family
            )));
        }
        // The payload IS the encoded MetricSample. A blob-ref / missing payload can't be a metric sample →
        // structural PERMANENT.
        let bytes: &[u8] = match &req.payload {
            Some(Payload::Inline(bytes)) => bytes,
            Some(Payload::Blob(_)) => {
                return EffectOutcome::Err(crate::retry::permanent(
                    "MetricExecutor: a blob-ref metric/publish payload is unsupported — inline the metric sample",
                ));
            }
            None => {
                return EffectOutcome::Err(crate::retry::permanent(
                    "MetricExecutor: a metric/publish effect requires a payload (the encoded metric sample)",
                ));
            }
        };
        // Decode the kernel-side MetricSample. A malformed frame is a clean PERMANENT (the reducer built a bad
        // sample; retrying won't fix it). The host does NOT log the raw bytes (guest-controlled).
        let sample = match decode_metric_publish(bytes) {
            Ok(s) => s,
            Err(e) => {
                return EffectOutcome::Err(crate::retry::permanent(format!(
                    "MetricExecutor: metric/publish payload did not decode: {e:?}"
                )));
            }
        };

        // 🔴 GUEST-STRING SAFETY: the name + every label key/val must be safe to carry off-box (bounded
        // charset + length). Reject the whole publish PERMANENT otherwise — never record or log an unsafe
        // guest string into the telemetry pipeline.
        if !Self::is_safe_str(&sample.name) {
            return EffectOutcome::Err(crate::retry::permanent(
                "MetricExecutor: metric name is empty/too-long/has unsafe chars (rejected before off-box export)",
            ));
        }
        for (k, v) in &sample.labels {
            if !Self::is_safe_str(k) || !Self::is_safe_str(v) {
                return EffectOutcome::Err(crate::retry::permanent(
                    "MetricExecutor: a metric label key/value is empty/too-long/has unsafe chars (rejected before off-box export)",
                ));
            }
        }

        // 🔴 CARDINALITY BOUND: admit a new distinct name only under the cap; an already-admitted name always
        // records. This bounds the guest's contribution to registry/backend cardinality.
        if !self.admitted.contains(&sample.name) {
            if self.admitted.len() >= MAX_DISTINCT_NAMES {
                return EffectOutcome::Err(crate::retry::permanent(format!(
                    "MetricExecutor: session exceeded its distinct-metric-name cap ({MAX_DISTINCT_NAMES}) — new metric names rejected (cardinality bound)"
                )));
            }
            self.admitted.insert(sample.name.clone());
        }

        // MECHANISM: map the kind → the registry's metric type + record. `kind` is a raw string vocab
        // (host/reducer own it); an unknown kind is a clean PERMANENT (the reducer named a kind the host
        // doesn't map). value is the exact f64 the reducer sent.
        let aggregation = Some(Self::aggregation_for(&sample));
        match sample.kind.as_str() {
            "counter" => {
                // A counter delta: the f64 value as a saturating non-negative u64 increment (a counter only
                // goes up; a negative/NaN delta is clamped to 0 — recording nothing rather than wrapping).
                let delta = if sample.value.is_finite() && sample.value > 0.0 {
                    sample.value as u64
                } else {
                    0
                };
                self.registry
                    .register_counter(REDUCER_METRIC.into(), aggregation)
                    .increment(delta);
            }
            "gauge" => {
                // A gauge set: the f64 as an i64 (saturating; the registry gauge is integral).
                let v = if sample.value.is_finite() {
                    sample.value as i64
                } else {
                    0
                };
                self.registry
                    .register_gauge(
                        REDUCER_METRIC.into(),
                        aggregation,
                        s2n_quic_dc_metrics::Unit::Count,
                    )
                    .set(v);
            }
            "timing" => {
                // A timing/distribution sample recorded into a Summary (histogram). record_f64 keeps the
                // exact value; the unit is microseconds (the same latency unit the host's own Summaries use).
                self.registry
                    .register_summary(
                        REDUCER_METRIC.into(),
                        aggregation,
                        s2n_quic_dc_metrics::Unit::Microsecond,
                    )
                    .record_f64(sample.value);
            }
            other => {
                return EffectOutcome::Err(crate::retry::permanent(format!(
                    "MetricExecutor: unknown metric kind {other:?} (expected counter/gauge/timing)"
                )));
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
    use cdz_kernel::event_ast::encode_metric_publish;

    fn publish_req(sample: &MetricSample) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::METRIC_PUBLISH,
            sample.name.clone(),
            Some(Payload::Inline(encode_metric_publish(sample).into())),
            Timeliness::Interactive,
        )
    }

    fn sample(name: &str, kind: &str, value: f64) -> MetricSample {
        MetricSample {
            name: name.into(),
            kind: kind.into(),
            value,
            labels: vec![],
        }
    }

    #[tokio::test]
    async fn records_counter_gauge_timing_into_the_registry() {
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry.clone());
        for (kind, val) in [("counter", 3.0), ("gauge", 42.0), ("timing", 12.5)] {
            let out = exec
                .perform(
                    &publish_req(&sample("agent.turns", kind, val)),
                    Hash::of(b"k"),
                )
                .await;
            assert!(
                matches!(out, EffectOutcome::Ok(None)),
                "a published {kind} metric acks Ok(None), got {out:?}"
            );
        }
        // The registry reports a line over the recorded reducer metrics (drain-on-report model).
        assert!(
            registry.try_take_current_metrics_line().is_some(),
            "the registry reports the reducer-published metrics"
        );
    }

    #[tokio::test]
    async fn labels_ride_into_a_stable_aggregation() {
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry.clone());
        let m = MetricSample {
            name: "agent.calls".into(),
            kind: "counter".into(),
            value: 1.0,
            labels: vec![
                ("role".into(), "builder".into()),
                ("ok".into(), "true".into()),
            ],
        };
        let out = exec.perform(&publish_req(&m), Hash::of(b"k")).await;
        assert!(matches!(out, EffectOutcome::Ok(None)));
        // Aggregation is label-sorted + stable regardless of input order.
        let mut reordered = m.clone();
        reordered.labels.reverse();
        assert_eq!(
            MetricExecutor::aggregation_for(&m),
            MetricExecutor::aggregation_for(&reordered),
            "label order doesn't change the aggregation key"
        );
    }

    #[tokio::test]
    async fn an_unknown_kind_is_permanent() {
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry);
        let out = exec
            .perform(
                &publish_req(&sample("m", "histogram-ish", 1.0)),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("unknown metric kind")),
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
                &publish_req(&sample("bad:name", "counter", 1.0)),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("unsafe chars")),
            "an unsafe metric name is rejected before off-box export, got {out:?}"
        );
        // A control char likewise.
        let out = exec
            .perform(
                &publish_req(&sample("bad\nname", "counter", 1.0)),
                Hash::of(b"k"),
            )
            .await;
        assert!(matches!(&out, EffectOutcome::Err(r) if r.contains("unsafe chars")));
    }

    #[tokio::test]
    async fn the_cardinality_cap_rejects_new_names_but_keeps_admitted_ones() {
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry);
        // Admit up to the cap.
        for i in 0..MAX_DISTINCT_NAMES {
            let out = exec
                .perform(
                    &publish_req(&sample(&format!("m{i}"), "counter", 1.0)),
                    Hash::of(b"k"),
                )
                .await;
            assert!(matches!(out, EffectOutcome::Ok(None)), "name {i} admitted");
        }
        // One more DISTINCT name → rejected (cardinality bound).
        let out = exec
            .perform(
                &publish_req(&sample("one-too-many", "counter", 1.0)),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("cardinality")),
            "a new name beyond the cap is rejected, got {out:?}"
        );
        // But an ALREADY-ADMITTED name still records fine (the cap bounds distinct names, not volume).
        let out = exec
            .perform(&publish_req(&sample("m0", "counter", 5.0)), Hash::of(b"k"))
            .await;
        assert!(
            matches!(out, EffectOutcome::Ok(None)),
            "an already-admitted name keeps recording past the cap"
        );
    }

    #[tokio::test]
    async fn a_non_metric_family_and_bad_payloads_are_permanent() {
        let registry = crate::metrics::Registry::new();
        let mut exec = MetricExecutor::new(registry);
        // Wrong family.
        let wrong = EffectRequest::new_with_family(
            effect_ct::EMIT,
            "x".to_string(),
            Some(Payload::Inline(bytes::Bytes::new())),
            Timeliness::Interactive,
        );
        assert!(
            matches!(exec.perform(&wrong, Hash::of(b"k")).await, EffectOutcome::Err(r) if r.contains("only handles"))
        );
        assert!(
            exec.handles_family(effect_ct::METRIC_PUBLISH) && !exec.handles_family(effect_ct::EMIT)
        );
        // Missing payload.
        let no_payload = EffectRequest::new_with_family(
            effect_ct::METRIC_PUBLISH,
            "m".to_string(),
            None,
            Timeliness::Interactive,
        );
        assert!(
            matches!(exec.perform(&no_payload, Hash::of(b"k")).await, EffectOutcome::Err(r) if r.contains("requires a payload"))
        );
        // Undecodable payload.
        let garbage = EffectRequest::new_with_family(
            effect_ct::METRIC_PUBLISH,
            "m".to_string(),
            Some(Payload::Inline(b"not a metric sample".to_vec().into())),
            Timeliness::Interactive,
        );
        assert!(
            matches!(exec.perform(&garbage, Hash::of(b"k")).await, EffectOutcome::Err(r) if r.contains("did not decode"))
        );
    }
}

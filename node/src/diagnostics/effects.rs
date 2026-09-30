//! In-memory metric registry (port of the `metrics[F]` half of `effects/package.scala`).
//!
//! Implements `rchain_shared::metrics::Metrics` and turns instrument calls into a
//! `PeriodSnapshot` the reporters can consume. Replaces kamon's `TrieMap[String, Metric[_]]`
//! backend with a `Mutex`-guarded `BTreeMap` accumulator.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rchain_shared::metrics::{Metrics, Source};

use super::model::{
    Bucket, Distribution, MeasurementUnit, MetricDistribution, MetricSnapshot, MetricValue,
    PeriodSnapshot, Tags,
};

#[derive(Clone, Debug, Default)]
struct HistogramAcc {
    count: i64,
    sum: i64,
    min: i64,
    max: i64,
    /// Observed value → how many times it was seen. **Without this the registry cannot produce a
    /// distribution at all**, and the first version did not have it: `snapshot()` emitted a single
    /// `Bucket { value: h.max, frequency: h.count }`, so every `_bucket{le=…}` line the endpoint
    /// rendered was a function of the **running maximum** and the scrape schedule rather than of the
    /// data. C182's two earlier "bucket" defects were both faces of this one — the boundaries were
    /// changed twice while the thing being bucketed was a single fabricated value (2026-09-30, Unit 2
    /// of the programme; the devnet's histogram read `_count 7617` against a census of 101).
    ///
    /// Bounded by the number of **distinct** values, not by the number of observations: the two
    /// publishers here send one sample per bucket edge (`casper/src/dag.rs:272,283`), so this holds at
    /// most five entries each. It is a `BTreeMap` so the rendered order is stable.
    buckets: BTreeMap<i64, i64>,
}

#[derive(Debug, Default)]
struct Inner {
    counters: BTreeMap<String, i64>,
    gauges: BTreeMap<String, i64>,
    range_samplers: BTreeMap<String, i64>,
    histograms: BTreeMap<String, HistogramAcc>,
}

/// A thread-safe in-memory metric registry (port of `effects.metrics`).
#[derive(Debug, Default)]
pub struct MetricsRegistry {
    inner: Mutex<Inner>,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        MetricsRegistry::default()
    }

    /// Observe a queue without retaining its sender or receiver. All quantities
    /// are gauges: this registry's reporters accumulate counters across snapshots.
    /// The peak is sampled, not an exact enqueue-time high-water mark.
    pub fn queue_observer(
        self: &Arc<Self>,
        source: Source,
    ) -> Arc<dyn Fn(usize, bool) + Send + Sync> {
        let registry = self.clone();
        let mut_stats = Mutex::new((0_i64, 0_i64));
        Arc::new(move |depth, active| {
            let depth = i64::try_from(depth).unwrap_or(i64::MAX);
            let mut stats = mut_stats.lock().unwrap_or_else(|p| p.into_inner());
            stats.0 = stats.0.max(depth);
            stats.1 = stats.1.saturating_add(1);
            // Update together so one scrape cannot mix values from two samples.
            let mut inner = registry.lock();
            for (name, value) in [
                ("depth", depth),
                ("sampled_peak", stats.0),
                ("observations", stats.1),
                ("consumer_active", i64::from(active)),
            ] {
                inner.gauges.insert(Self::key(&source, name), value);
            }
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn key(source: &Source, name: &str) -> String {
        source.sub(name).0
    }

    /// Produce a snapshot of the currently-accumulated metrics (port of the reporters' input).
    pub fn snapshot(&self) -> PeriodSnapshot {
        let inner = self.lock();

        let mut counters = Vec::new();
        for (name, value) in &inner.counters {
            counters.push(MetricValue {
                name: name.clone(),
                tags: Tags::new(),
                value: *value,
                unit: MeasurementUnit::NONE,
            });
        }

        let mut gauges = Vec::new();
        for (name, value) in &inner.gauges {
            gauges.push(MetricValue {
                name: name.clone(),
                tags: Tags::new(),
                value: *value,
                unit: MeasurementUnit::NONE,
            });
        }

        let mut histograms = Vec::new();
        for (name, h) in &inner.histograms {
            histograms.push(MetricDistribution {
                name: name.clone(),
                tags: Tags::new(),
                unit: MeasurementUnit::NONE,
                distribution: Distribution {
                    count: h.count,
                    sum: h.sum,
                    min: h.min,
                    max: h.max,
                    // The observations, as recorded. This one line is the difference between an
                    // instrument and a shape: the renderer buckets these against its configured edges,
                    // so `_bucket{le=…}` is now a fact about the data.
                    buckets: h
                        .buckets
                        .iter()
                        .map(|(value, frequency)| Bucket {
                            value: *value,
                            frequency: *frequency,
                        })
                        .collect(),
                },
            });
        }

        let mut range_samplers = Vec::new();
        for (name, value) in &inner.range_samplers {
            range_samplers.push(MetricDistribution {
                name: name.clone(),
                tags: Tags::new(),
                unit: MeasurementUnit::NONE,
                distribution: Distribution {
                    count: 1,
                    sum: *value,
                    min: *value,
                    max: *value,
                    buckets: vec![Bucket {
                        value: *value,
                        frequency: 1,
                    }],
                },
            });
        }

        PeriodSnapshot {
            from: 0,
            to: now_millis(),
            metrics: MetricSnapshot {
                counters,
                gauges,
                histograms,
                range_samplers,
            },
        }
    }
}

impl Metrics for MetricsRegistry {
    fn increment_counter(&self, source: &Source, name: &str, delta: i64) {
        let mut inner = self.lock();
        *inner.counters.entry(Self::key(source, name)).or_default() += delta;
    }

    fn increment_sampler(&self, source: &Source, name: &str, delta: i64) {
        let mut inner = self.lock();
        *inner
            .range_samplers
            .entry(Self::key(source, name))
            .or_default() += delta;
    }

    fn sample(&self, source: &Source, name: &str) {
        let mut inner = self.lock();
        inner
            .range_samplers
            .entry(Self::key(source, name))
            .or_default();
    }

    fn set_gauge(&self, source: &Source, name: &str, value: i64) {
        let mut inner = self.lock();
        inner.gauges.insert(Self::key(source, name), value);
    }

    fn increment_gauge(&self, source: &Source, name: &str, delta: i64) {
        let mut inner = self.lock();
        *inner.gauges.entry(Self::key(source, name)).or_default() += delta;
    }

    fn decrement_gauge(&self, source: &Source, name: &str, delta: i64) {
        let mut inner = self.lock();
        *inner.gauges.entry(Self::key(source, name)).or_default() -= delta;
    }

    fn record(&self, source: &Source, name: &str, value: i64, count: i64) {
        let mut inner = self.lock();
        let h = inner.histograms.entry(Self::key(source, name)).or_default();
        if h.count == 0 {
            h.min = value;
            h.max = value;
        } else {
            h.min = h.min.min(value);
            h.max = h.max.max(value);
        }
        h.count += count;
        h.sum += value * count;
        // The observation itself, which is what `snapshot` renders a distribution *from*. A `count` of
        // zero carries no observation, and recording one would invent a bucket out of a call that
        // observed nothing.
        if count > 0 {
            h.buckets
                .entry(value)
                .and_modify(|f| *f += count)
                .or_insert(count);
        }
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_gauges_and_histograms_accumulate() {
        let registry = MetricsRegistry::new();
        let src = Source::base();

        registry.increment_counter(&src, "req", 3);
        registry.increment_counter(&src, "req", 2);
        registry.set_gauge(&src, "conn", 10);
        registry.increment_gauge(&src, "conn", 5);
        registry.record(&src, "latency", 100, 2);
        registry.record(&src, "latency", 300, 1);

        let snap = registry.snapshot();

        let counter = snap
            .metrics
            .counters
            .iter()
            .find(|m| m.name == "rchain.req")
            .unwrap();
        assert_eq!(counter.value, 5);

        let gauge = snap
            .metrics
            .gauges
            .iter()
            .find(|m| m.name == "rchain.conn")
            .unwrap();
        assert_eq!(gauge.value, 15);

        let hist = snap
            .metrics
            .histograms
            .iter()
            .find(|m| m.name == "rchain.latency")
            .unwrap();
        assert_eq!(hist.distribution.count, 3);
        assert_eq!(hist.distribution.sum, 500);
        assert_eq!(hist.distribution.min, 100);
        assert_eq!(hist.distribution.max, 300);
    }

    #[test]
    fn queue_samples_keep_the_peak_and_separate_stages() {
        let registry = Arc::new(MetricsRegistry::new());
        let a = registry.queue_observer(Source::base().sub("queue_a"));
        let b = registry.queue_observer(Source::base().sub("queue_b"));
        a(7, true);
        a(2, true);
        b(3, true);
        a(0, false);
        let snap = registry.snapshot();
        let gauge = |name: &str| {
            snap.metrics
                .gauges
                .iter()
                .find(|g| g.name == name)
                .unwrap()
                .value
        };
        assert_eq!(gauge("rchain.queue_a.depth"), 0);
        assert_eq!(gauge("rchain.queue_a.sampled_peak"), 7);
        assert_eq!(gauge("rchain.queue_a.consumer_active"), 0);
        assert_eq!(gauge("rchain.queue_a.observations"), 3);
        assert_eq!(gauge("rchain.queue_b.depth"), 3);
        assert_eq!(gauge("rchain.queue_b.sampled_peak"), 3);
        assert_eq!(gauge("rchain.queue_b.consumer_active"), 1);
    }

    #[test]
    fn empty_registry_produces_empty_snapshot() {
        let registry = MetricsRegistry::new();
        let snap = registry.snapshot();
        assert!(snap.metrics.counters.is_empty());
        assert!(snap.metrics.gauges.is_empty());
        assert!(snap.metrics.histograms.is_empty());
        assert!(snap.metrics.range_samplers.is_empty());
    }
}

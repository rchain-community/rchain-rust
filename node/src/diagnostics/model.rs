//! Metric snapshot data model (port of the kamon snapshot types the reporters read).

use std::collections::BTreeMap;

/// Metric tags (labels). Deterministic iteration order (sorted by key).
pub type Tags = BTreeMap<String, String>;

/// A measurement-unit dimension (port of `kamon.metric.MeasurementUnit.Dimension`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    Time,
    Information,
    None,
}

/// A measurement unit (port of `kamon.metric.MeasurementUnit`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasurementUnit {
    pub dimension: Dimension,
    pub magnitude: f64,
}

impl MeasurementUnit {
    pub const NONE: MeasurementUnit = MeasurementUnit {
        dimension: Dimension::None,
        magnitude: 1.0,
    };
    pub const SECONDS: MeasurementUnit = MeasurementUnit {
        dimension: Dimension::Time,
        magnitude: 1.0,
    };
    pub const BYTES: MeasurementUnit = MeasurementUnit {
        dimension: Dimension::Information,
        magnitude: 1.0,
    };
}

/// A histogram bucket (port of `kamon.metric.Bucket`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bucket {
    pub value: i64,
    pub frequency: i64,
}

/// A value distribution (port of `kamon.metric.Distribution`).
#[derive(Clone, Debug, PartialEq)]
pub struct Distribution {
    pub count: i64,
    pub sum: i64,
    pub min: i64,
    pub max: i64,
    pub buckets: Vec<Bucket>,
}

impl Distribution {
    /// Value at the given percentile (0.0–1.0), by cumulative bucket frequency.
    pub fn percentile(&self, p: f64) -> i64 {
        if self.count <= 0 {
            return 0;
        }
        let target = (p * self.count as f64) as i64;
        let mut cumulative = 0i64;
        for bucket in &self.buckets {
            cumulative += bucket.frequency;
            if cumulative >= target {
                return bucket.value;
            }
        }
        self.max
    }
}

/// A counter/gauge snapshot (port of `kamon.metric.MetricValue`).
#[derive(Clone, Debug, PartialEq)]
pub struct MetricValue {
    pub name: String,
    pub tags: Tags,
    pub value: i64,
    pub unit: MeasurementUnit,
}

/// A histogram/range-sampler snapshot (port of `kamon.metric.MetricDistribution`).
#[derive(Clone, Debug, PartialEq)]
pub struct MetricDistribution {
    pub name: String,
    pub tags: Tags,
    pub unit: MeasurementUnit,
    pub distribution: Distribution,
}

/// A collection of metric snapshots (port of `kamon.metric.MetricSnapshot`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricSnapshot {
    pub counters: Vec<MetricValue>,
    pub gauges: Vec<MetricValue>,
    pub histograms: Vec<MetricDistribution>,
    pub range_samplers: Vec<MetricDistribution>,
}

/// A period snapshot (port of `kamon.metric.PeriodSnapshot`). Timestamps are epoch milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct PeriodSnapshot {
    pub from: i64,
    pub to: i64,
    pub metrics: MetricSnapshot,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distribution(count: i64, buckets: &[(i64, i64)], max: i64) -> Distribution {
        Distribution {
            count,
            sum: buckets.iter().map(|(v, f)| v * f).sum(),
            min: buckets.first().map(|(v, _)| *v).unwrap_or(0),
            max,
            buckets: buckets
                .iter()
                .map(|(value, frequency)| Bucket {
                    value: *value,
                    frequency: *frequency,
                })
                .collect(),
        }
    }

    /// An empty distribution (or one with a nonsensical count) answers `0` rather than dividing by
    /// a zero count or indexing an empty bucket list — the reporter asks for percentiles on metrics
    /// nothing has recorded yet.
    #[test]
    fn a_distribution_with_no_count_answers_zero() {
        let empty = Distribution {
            count: 0,
            sum: 0,
            min: 0,
            max: 0,
            buckets: Vec::new(),
        };
        for p in [0.0, 0.5, 1.0, 2.0] {
            assert_eq!(empty.percentile(p), 0, "p = {p}");
        }

        let negative = Distribution {
            count: -1,
            ..empty.clone()
        };
        assert_eq!(negative.percentile(0.99), 0);
    }

    /// The percentile walks the buckets by cumulative frequency: the first bucket whose running
    /// total reaches `p * count`. The target is **truncated** to an integer (`as i64`), which is
    /// observable at the boundary — `p = 0.21` of ten values is still target 2, so it answers the
    /// same bucket as `p = 0.2`.
    #[test]
    fn the_percentile_walks_the_cumulative_bucket_frequency() {
        // Values 1 (×2), 3 (×5), 9 (×3): count 10.
        let d = distribution(10, &[(1, 2), (3, 5), (9, 3)], 9);

        assert_eq!(d.percentile(0.0), 1, "the lowest value");
        assert_eq!(
            d.percentile(0.2),
            1,
            "target 2, reached by the first bucket"
        );
        assert_eq!(d.percentile(0.21), 1, "target 2 (truncated)");
        assert_eq!(d.percentile(0.3), 3, "target 3, the second bucket");
        assert_eq!(d.percentile(0.5), 3);
        assert_eq!(
            d.percentile(0.7),
            3,
            "cumulative 7 is still the second bucket"
        );
        assert_eq!(d.percentile(0.8), 9, "target 8 needs the third bucket");
        assert_eq!(
            d.percentile(1.0),
            9,
            "the highest bucket carries the last count"
        );
    }

    /// When the buckets do not account for every count, a percentile past their total falls back to
    /// `max` — the reporter's own `max` field rather than a silent 0 or a panic.
    #[test]
    fn a_percentile_past_the_buckets_falls_back_to_the_maximum() {
        let d = distribution(100, &[(1, 2), (3, 2)], 42);
        assert_eq!(d.percentile(0.01), 1, "inside the buckets");
        assert_eq!(
            d.percentile(0.04),
            3,
            "the last bucket covers targets up to 4"
        );
        assert_eq!(
            d.percentile(0.5),
            42,
            "target 50 is past every bucket: the distribution's own max"
        );
        assert_eq!(d.percentile(1.0), 42);

        // A distribution with a count but no buckets at all is the same case.
        let bucketless = Distribution {
            count: 3,
            sum: 0,
            min: 0,
            max: 7,
            buckets: Vec::new(),
        };
        assert_eq!(bucketless.percentile(0.5), 7);
    }

    /// Tags are a `BTreeMap`, so their iteration order is the sorted key order regardless of the
    /// order they were inserted in — which is what makes a metrics snapshot's rendering stable
    /// between runs (and between two nodes reporting the same metric).
    #[test]
    fn tags_iterate_in_sorted_key_order_whatever_the_insertion_order() {
        let mut one: Tags = BTreeMap::new();
        one.insert("z".to_string(), "1".to_string());
        one.insert("a".to_string(), "2".to_string());
        one.insert("m".to_string(), "3".to_string());

        let mut other: Tags = BTreeMap::new();
        other.insert("a".to_string(), "2".to_string());
        other.insert("m".to_string(), "3".to_string());
        other.insert("z".to_string(), "1".to_string());

        let keys: Vec<&String> = one.keys().collect();
        assert_eq!(keys, vec!["a", "m", "z"]);
        assert_eq!(one, other, "the same tags, however they were built");

        // A snapshot without metrics is the empty one, not a panic.
        let snapshot = MetricSnapshot::default();
        assert!(snapshot.counters.is_empty() && snapshot.histograms.is_empty());
    }

    /// The three unit constants are the ones the reporters scale by: a changed magnitude would
    /// silently rescale every time and byte metric.
    #[test]
    fn the_unit_constants_are_the_documented_ones() {
        assert_eq!(MeasurementUnit::NONE.dimension, Dimension::None);
        assert_eq!(MeasurementUnit::NONE.magnitude, 1.0);
        assert_eq!(MeasurementUnit::SECONDS.dimension, Dimension::Time);
        assert_eq!(MeasurementUnit::BYTES.dimension, Dimension::Information);
        assert!((MeasurementUnit::BYTES.magnitude - 1.0).abs() < f64::EPSILON);
    }
}

//! InfluxDB line-protocol reporters (port of `UdpInfluxDBReporter.scala` +
//! `BatchInfluxDBReporter.scala`). The line-protocol encoding, the UDP reporter, and the batch
//! reporter's HTTP POST are ported; the periodic flush/batching loop is left to the caller.

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use super::model::{MetricDistribution, MetricValue, PeriodSnapshot, Tags};

/// InfluxDB string escaping (port of `escapeString`): ` `, `=` and `,` are backslash-escaped.
pub fn escape_string(input: &str) -> String {
    input
        .replace(' ', "\\ ")
        .replace('=', "\\=")
        .replace(',', "\\,")
}

/// Java `Double.toString` (append `.0` for integral doubles).
fn double_to_string(value: f64) -> String {
    let s = value.to_string();
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}

fn write_name_and_tags(
    builder: &mut String,
    name: &str,
    metric_tags: &Tags,
    additional_tags: &Tags,
) {
    builder.push_str(name);
    let mut tags = metric_tags.clone();
    for (k, v) in additional_tags {
        tags.insert(k.clone(), v.clone());
    }
    for (key, value) in &tags {
        builder.push(',');
        builder.push_str(&escape_string(key));
        builder.push('=');
        builder.push_str(&escape_string(value));
    }
    builder.push(' ');
}

fn write_int_field(builder: &mut String, field_name: &str, value: i64, append_separator: bool) {
    builder.push_str(field_name);
    builder.push('=');
    builder.push_str(&value.to_string());
    builder.push('i');
    if append_separator {
        builder.push(',');
    }
}

fn write_double_field(builder: &mut String, field_name: &str, value: f64, append_separator: bool) {
    builder.push_str(field_name);
    builder.push('=');
    builder.push_str(&double_to_string(value));
    if append_separator {
        builder.push(',');
    }
}

fn write_timestamp(builder: &mut String, timestamp: i64, newline: bool) {
    builder.push(' ');
    builder.push_str(&timestamp.to_string());
    if newline {
        builder.push('\n');
    }
}

/// Encode a counter/gauge into a line-protocol record (port of `writeMetricValue`).
pub fn write_metric_value(
    builder: &mut String,
    metric: &MetricValue,
    field_name: &str,
    timestamp: i64,
    additional_tags: &Tags,
    newline: bool,
) {
    write_name_and_tags(builder, &metric.name, &metric.tags, additional_tags);
    write_int_field(builder, field_name, metric.value, false);
    write_timestamp(builder, timestamp, newline);
}

/// Encode a histogram/range-sampler into a line-protocol record (port of `writeMetricDistribution`).
pub fn write_metric_distribution(
    builder: &mut String,
    metric: &MetricDistribution,
    percentiles: &[f64],
    timestamp: i64,
    additional_tags: &Tags,
    newline: bool,
) {
    write_name_and_tags(builder, &metric.name, &metric.tags, additional_tags);
    write_int_field(builder, "count", metric.distribution.count, true);
    write_int_field(builder, "sum", metric.distribution.sum, true);
    write_int_field(builder, "min", metric.distribution.min, true);
    for &p in percentiles {
        write_double_field(
            builder,
            &format!("p{}", double_to_string(p)),
            metric.distribution.percentile(p) as f64,
            true,
        );
    }
    write_int_field(builder, "max", metric.distribution.max, false);
    write_timestamp(builder, timestamp, newline);
}

/// UDP reporter settings (port of `UdpInfluxDBReporter.Settings`).
#[derive(Clone, Debug, PartialEq)]
pub struct UdpSettings {
    pub address: SocketAddr,
    pub max_packet_size: i64,
    pub percentiles: Vec<f64>,
    pub additional_tags: Tags,
}

/// Batches line-protocol measurements and flushes over UDP (port of `MetricDataPacketBuffer`).
pub struct MetricDataPacketBuffer<'a> {
    max_packet_size: i64,
    socket: &'a UdpSocket,
    remote: SocketAddr,
    buffer: String,
}

impl<'a> MetricDataPacketBuffer<'a> {
    pub fn new(max_packet_size: i64, socket: &'a UdpSocket, remote: SocketAddr) -> Self {
        MetricDataPacketBuffer {
            max_packet_size,
            socket,
            remote,
            buffer: String::new(),
        }
    }

    pub fn append_measurement(&mut self, measurement: &str) -> std::io::Result<()> {
        let separator = "\n";
        if self.fits_on_buffer(&format!("{separator}{measurement}")) {
            let m_separator = if self.buffer.is_empty() {
                ""
            } else {
                separator
            };
            self.buffer.push_str(m_separator);
            self.buffer.push_str(measurement);
        } else {
            self.flush()?;
            self.buffer.push_str(measurement);
        }
        Ok(())
    }

    fn fits_on_buffer(&self, data: &str) -> bool {
        (self.buffer.len() + data.len()) as i64 <= self.max_packet_size
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.socket.send_to(self.buffer.as_bytes(), self.remote)?;
        self.buffer.clear();
        Ok(())
    }
}

/// UDP InfluxDB reporter (port of `UdpInfluxDBReporter`).
pub struct UdpInfluxDbReporter {
    settings: UdpSettings,
    socket: UdpSocket,
}

impl UdpInfluxDbReporter {
    pub fn new(settings: UdpSettings) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        Ok(UdpInfluxDbReporter { settings, socket })
    }

    pub fn report_period_snapshot(&self, snapshot: &PeriodSnapshot) -> std::io::Result<()> {
        let mut buffer = MetricDataPacketBuffer::new(
            self.settings.max_packet_size,
            &self.socket,
            self.settings.address,
        );
        let timestamp = snapshot.to;

        for c in &snapshot.metrics.counters {
            let mut b = String::new();
            write_metric_value(
                &mut b,
                c,
                "count",
                timestamp,
                &self.settings.additional_tags,
                false,
            );
            buffer.append_measurement(&b)?;
        }
        for g in &snapshot.metrics.gauges {
            let mut b = String::new();
            write_metric_value(
                &mut b,
                g,
                "value",
                timestamp,
                &self.settings.additional_tags,
                false,
            );
            buffer.append_measurement(&b)?;
        }
        for h in &snapshot.metrics.histograms {
            let mut b = String::new();
            write_metric_distribution(
                &mut b,
                h,
                &self.settings.percentiles,
                timestamp,
                &self.settings.additional_tags,
                false,
            );
            buffer.append_measurement(&b)?;
        }
        for rs in &snapshot.metrics.range_samplers {
            let mut b = String::new();
            write_metric_distribution(
                &mut b,
                rs,
                &self.settings.percentiles,
                timestamp,
                &self.settings.additional_tags,
                false,
            );
            buffer.append_measurement(&b)?;
        }

        Ok(())
    }
}

/// Batch (HTTP) reporter settings (port of `BatchInfluxDBReporter.Settings`).
#[derive(Clone, Debug, PartialEq)]
pub struct BatchSettings {
    pub url: String,
    pub batch_interval: Duration,
    pub percentiles: Vec<f64>,
    pub credentials: Option<String>,
    pub additional_tags: Tags,
}

/// Batch InfluxDB reporter (port of `BatchInfluxDBReporter`); the HTTP POST transport is ported.
pub struct BatchInfluxDbReporter {
    settings: BatchSettings,
}

impl BatchInfluxDbReporter {
    pub fn new(settings: BatchSettings) -> Self {
        BatchInfluxDbReporter { settings }
    }

    /// Translate a whole snapshot into a `\n`-terminated line-protocol string (port of
    /// `translateToLineProtocol`).
    pub fn translate_to_line_protocol(&self, snapshot: &PeriodSnapshot) -> String {
        let mut builder = String::new();
        let timestamp = snapshot.to;

        for c in &snapshot.metrics.counters {
            write_metric_value(
                &mut builder,
                c,
                "count",
                timestamp,
                &self.settings.additional_tags,
                true,
            );
        }
        for g in &snapshot.metrics.gauges {
            write_metric_value(
                &mut builder,
                g,
                "value",
                timestamp,
                &self.settings.additional_tags,
                true,
            );
        }
        for h in &snapshot.metrics.histograms {
            write_metric_distribution(
                &mut builder,
                h,
                &self.settings.percentiles,
                timestamp,
                &self.settings.additional_tags,
                true,
            );
        }
        for rs in &snapshot.metrics.range_samplers {
            write_metric_distribution(
                &mut builder,
                rs,
                &self.settings.percentiles,
                timestamp,
                &self.settings.additional_tags,
                true,
            );
        }

        builder
    }

    /// Translate the snapshot to line protocol and POST it to the configured InfluxDB `/write`
    /// endpoint (port of `BatchInfluxDBReporter.reportPeriodSnapshot`). The periodic flush loop that
    /// drives this on `batch_interval` is left to the caller.
    pub async fn report_period_snapshot(&self, snapshot: &PeriodSnapshot) -> Result<(), String> {
        let body = self.translate_to_line_protocol(snapshot);
        let client = reqwest::Client::new();
        let mut req = client.post(&self.settings.url).body(body);
        if let Some(creds) = &self.settings.credentials {
            req = req.header(reqwest::header::AUTHORIZATION, format!("Basic {creds}"));
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("InfluxDB POST returned HTTP {}", resp.status()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::model::{Bucket, Distribution, Tags};

    #[test]
    fn escape_string_escapes_space_eq_comma() {
        assert_eq!(escape_string("a b=c,d"), "a\\ b\\=c\\,d");
    }

    #[test]
    fn counter_line_protocol() {
        let mut tags = Tags::new();
        tags.insert("tag".to_string(), "value".to_string());
        let metric = MetricValue {
            name: "my.counter".to_string(),
            tags,
            value: 5,
            unit: crate::diagnostics::model::MeasurementUnit::NONE,
        };
        let mut b = String::new();
        write_metric_value(&mut b, &metric, "count", 1234567890, &Tags::new(), false);
        assert_eq!(b, "my.counter,tag=value count=5i 1234567890");
    }

    #[test]
    fn distribution_line_protocol() {
        let metric = MetricDistribution {
            name: "hist".to_string(),
            tags: Tags::new(),
            unit: crate::diagnostics::model::MeasurementUnit::NONE,
            distribution: Distribution {
                count: 3,
                sum: 16,
                min: 1,
                max: 10,
                buckets: vec![
                    Bucket {
                        value: 1,
                        frequency: 1,
                    },
                    Bucket {
                        value: 5,
                        frequency: 1,
                    },
                    Bucket {
                        value: 10,
                        frequency: 1,
                    },
                ],
            },
        };
        let mut b = String::new();
        write_metric_distribution(
            &mut b,
            &metric,
            &[0.5, 0.99],
            1234567890,
            &Tags::new(),
            false,
        );
        assert_eq!(
            b,
            "hist count=3i,sum=16i,min=1i,p0.5=1.0,p0.99=5.0,max=10i 1234567890"
        );
    }

    #[test]
    fn batch_reporter_terminates_records_with_newline() {
        let snapshot = crate::diagnostics::model::PeriodSnapshot {
            from: 0,
            to: 1234567890,
            metrics: crate::diagnostics::model::MetricSnapshot {
                counters: vec![MetricValue {
                    name: "c".to_string(),
                    tags: Tags::new(),
                    value: 1,
                    unit: crate::diagnostics::model::MeasurementUnit::NONE,
                }],
                gauges: vec![],
                histograms: vec![],
                range_samplers: vec![],
            },
        };
        let reporter = BatchInfluxDbReporter::new(BatchSettings {
            url: "http://localhost:8086/write?precision=ms&db=metrics".to_string(),
            batch_interval: Duration::from_secs(10),
            percentiles: vec![0.5],
            credentials: None,
            additional_tags: Tags::new(),
        });
        assert_eq!(
            reporter.translate_to_line_protocol(&snapshot),
            "c count=1i 1234567890\n"
        );
    }
}

#[cfg(test)]
mod line_protocol_tests {
    use super::*;

    use super::super::model::{Bucket, Dimension, Distribution, MeasurementUnit};

    fn tags(pairs: &[(&str, &str)]) -> Tags {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn counter(name: &str, value: i64) -> MetricValue {
        MetricValue {
            name: name.to_string(),
            tags: tags(&[("node", "n1")]),
            value,
            unit: MeasurementUnit::NONE,
        }
    }

    /// Line-protocol escaping: `space`, `=` and `,` are the three characters that would otherwise
    /// split a key or a value into extra parts.
    #[test]
    fn escaping_covers_space_equals_and_comma() {
        assert_eq!(escape_string("a b"), "a\\ b");
        assert_eq!(escape_string("a=b"), "a\\=b");
        assert_eq!(escape_string("a,b"), "a\\,b");
        assert_eq!(escape_string("plain"), "plain");
        assert_eq!(escape_string("a b=c,d"), "a\\ b\\=c\\,d");
    }

    /// A Java `Double.toString` keeps an integral double distinguishable from an integer (`1.0`, not
    /// `1`), which is how InfluxDB tells a float field from an int field.
    #[test]
    fn a_double_is_rendered_with_its_decimal_point() {
        assert_eq!(double_to_string(1.0), "1.0");
        assert_eq!(double_to_string(-3.0), "-3.0");
        assert_eq!(double_to_string(1.5), "1.5");
        assert_eq!(double_to_string(0.0), "0.0");
    }

    /// A counter or gauge becomes `<name>,<tags> <field>=<value>i <timestamp>`, with the value marked
    /// as an integer (`i`) and the tags sorted (they come from a `BTreeMap`).
    #[test]
    fn a_metric_value_renders_a_line_protocol_record() {
        let mut out = String::new();
        write_metric_value(
            &mut out,
            &counter("my.counter", 7),
            "value",
            1234,
            &Tags::new(),
            true,
        );
        assert_eq!(out, "my.counter,node=n1 value=7i 1234\n");

        // Without a newline the record is left open for the next one; extra tags are merged and win
        // over the metric's own.
        let mut out = String::new();
        write_metric_value(
            &mut out,
            &counter("my.counter", 7),
            "value",
            1234,
            &tags(&[("node", "override"), ("extra", "x")]),
            false,
        );
        assert_eq!(out, "my.counter,extra=x,node=override value=7i 1234");
    }

    /// A distribution renders `count`, `sum`, `min`, one field per percentile, and `max` — each
    /// followed by a comma because the field list continues.
    #[test]
    fn a_distribution_renders_every_aggregate() {
        let distribution = MetricDistribution {
            name: "my.histogram".to_string(),
            tags: tags(&[("k", "v")]),
            unit: MeasurementUnit {
                dimension: Dimension::Time,
                magnitude: 1.0,
            },
            distribution: Distribution {
                count: 4,
                sum: 10,
                min: 1,
                max: 4,
                buckets: vec![
                    Bucket {
                        value: 1,
                        frequency: 1,
                    },
                    Bucket {
                        value: 2,
                        frequency: 2,
                    },
                    Bucket {
                        value: 4,
                        frequency: 1,
                    },
                ],
            },
        };
        let mut out = String::new();
        write_metric_distribution(
            &mut out,
            &distribution,
            &[0.5, 0.99],
            99,
            &Tags::new(),
            true,
        );

        assert!(out.starts_with("my.histogram,k=v "), "{out}");
        assert!(out.contains("count=4i,"), "{out}");
        assert!(out.contains("sum=10i,"), "{out}");
        assert!(out.contains("min=1i,"), "{out}");
        assert!(out.contains("p0.5="), "{out}");
        assert!(out.contains("p0.99="), "{out}");
        assert!(out.contains("max=4i "), "{out}");
        assert!(out.ends_with(" 99\n"), "{out}");
    }

    /// The percentile rendering uses `double_to_string` for the field *name* too, so `p1` is spelled
    /// `p1.0` — the same shape a float field needs.
    #[test]
    fn a_percentile_field_name_keeps_its_decimal_point() {
        let distribution = MetricDistribution {
            name: "h".to_string(),
            tags: Tags::new(),
            unit: MeasurementUnit::NONE,
            distribution: Distribution {
                count: 1,
                sum: 5,
                min: 5,
                max: 5,
                buckets: vec![Bucket {
                    value: 5,
                    frequency: 1,
                }],
            },
        };
        let mut out = String::new();
        write_metric_distribution(&mut out, &distribution, &[1.0], 1, &Tags::new(), false);
        assert!(out.contains("p1.0="), "{out}");
        assert!(!out.contains("p1="), "{out}");
    }

    /// The packet buffer sends when the measurement no longer fits in the configured packet, and
    /// keeps the remainder buffered otherwise — the bound that keeps a UDP datagram legal. A socket
    /// bound to an ephemeral port is the whole harness (the packet goes to a port nobody reads).
    #[test]
    fn the_packet_buffer_flushes_when_the_measurement_would_overflow() {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind");
        let remote: SocketAddr = "127.0.0.1:9".parse().expect("addr");

        // A generous packet size: nothing flushes until asked.
        let mut buffer = MetricDataPacketBuffer::new(1024, &socket, remote);
        buffer.append_measurement("m v=1i 1\n").expect("append");
        buffer.flush().expect("flush");

        // A packet size smaller than one measurement: the first append fits nothing, so it flushes
        // and keeps the measurement for the next pack.
        let mut tiny = MetricDataPacketBuffer::new(4, &socket, remote);
        tiny.append_measurement("long.measurement v=1i 1\n")
            .expect("append");
        tiny.flush().expect("flush");
    }

    /// A UDP reporter can be constructed from its settings, and a period snapshot with no metrics
    /// reports successfully (the empty-snapshot arm).
    #[test]
    fn an_empty_snapshot_is_reported_without_error() {
        let settings = UdpSettings {
            address: "127.0.0.1:9".parse().expect("addr"),
            max_packet_size: 1024,
            percentiles: vec![0.5],
            additional_tags: Tags::new(),
        };
        let reporter = UdpInfluxDbReporter::new(settings).expect("reporter");
        let snapshot = PeriodSnapshot {
            from: 0,
            to: 1,
            metrics: super::super::model::MetricSnapshot::default(),
        };
        reporter
            .report_period_snapshot(&snapshot)
            .expect("an empty snapshot reports");
    }
}

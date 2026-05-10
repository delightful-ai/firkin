//! Trace and benchmark sample primitives for Firkin.

use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};
use smallvec::SmallVec;
use thiserror::Error as ThisError;
use tokio::time::MissedTickBehavior;

pub mod events;
pub use events::*;

/// Maximum shared tags on a trace envelope.
pub const MAX_SHARED_TAGS: usize = 32;

/// Maximum per-sample tags on one benchmark sample.
///
/// Product-path promotion samples need source trust, event endpoints, probe
/// boundary, and per-leg CLI/browser/database boundary tags. Keep the cap
/// finite so tags remain inspectable, but high enough that promotion evidence
/// cannot be silently truncated.
pub const MAX_SAMPLE_TAGS: usize = 16;

/// Maximum tag key length in bytes.
pub const MAX_TAG_KEY_BYTES: usize = 64;

/// Maximum tag value length in bytes.
pub const MAX_TAG_VALUE_BYTES: usize = 256;

/// Default sample cap for one recorder drain window.
pub const DEFAULT_SAMPLE_CAP: usize = 4096;

/// Benchmark metric category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum BenchmarkMetricKind {
    /// End-to-end lifecycle latency, such as restore or command start.
    LifecycleLatency,
    /// Firkin host-side overhead, measured separately from guest workload cost.
    FirkinOverhead,
    /// VM/container workload resource usage.
    WorkloadResource,
}

/// Benchmark measurement unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum BenchmarkUnit {
    /// Milliseconds.
    Milliseconds,
    /// Microseconds.
    Microseconds,
    /// Percentage.
    Percent,
    /// Mebibytes.
    Mebibytes,
    /// Hertz.
    Hertz,
    /// Bytes.
    Bytes,
    /// Unitless count.
    Count,
    /// Count per second.
    CountPerSecond,
    /// File or filesystem operations per second.
    OperationsPerSecond,
    /// Bytes per second.
    BytesPerSecond,
    /// Mebibytes per second.
    MebibytesPerSecond,
    /// I/O operations per second.
    Iops,
    /// Unitless ratio.
    Ratio,
}

/// Single benchmark sample.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BenchmarkSample {
    metric: Cow<'static, str>,
    kind: BenchmarkMetricKind,
    unit: BenchmarkUnit,
    value: f64,
    #[serde(default, rename = "tags", skip_serializing_if = "SampleTags::is_empty")]
    sample_tags: SampleTags,
}

impl BenchmarkSample {
    /// Construct a benchmark sample.
    #[must_use]
    pub fn new(
        metric: impl Into<String>,
        kind: BenchmarkMetricKind,
        unit: BenchmarkUnit,
        value: f64,
    ) -> Self {
        Self {
            metric: Cow::Owned(metric.into()),
            kind,
            unit,
            value,
            sample_tags: SampleTags::default(),
        }
    }

    /// Construct a benchmark sample from a static metric name.
    #[must_use]
    pub fn from_static(
        metric: &'static str,
        kind: BenchmarkMetricKind,
        unit: BenchmarkUnit,
        value: f64,
    ) -> Self {
        Self {
            metric: Cow::Borrowed(metric),
            kind,
            unit,
            value,
            sample_tags: SampleTags::default(),
        }
    }

    /// Attach a static tag to the sample.
    #[must_use]
    pub fn with_static_tag(mut self, key: &'static str, value: &'static str) -> Self {
        let _ = self.sample_tags.push_static(key, value);
        self
    }

    /// Attach a dynamic tag value to the sample.
    #[must_use]
    pub fn with_dynamic_tag(mut self, key: &'static str, value: impl Into<String>) -> Self {
        let _ = self.sample_tags.push_dynamic(key, value.into());
        self
    }

    /// Return the metric name.
    #[must_use]
    pub fn metric(&self) -> &str {
        &self.metric
    }

    /// Return the metric category.
    #[must_use]
    pub const fn kind(&self) -> BenchmarkMetricKind {
        self.kind
    }

    /// Return the measurement unit.
    #[must_use]
    pub const fn unit(&self) -> BenchmarkUnit {
        self.unit
    }

    /// Return the observed value.
    #[must_use]
    pub const fn value(&self) -> f64 {
        self.value
    }

    /// Return per-sample tags.
    #[must_use]
    pub fn tags(&self) -> &[SampleTag] {
        self.sample_tags.as_slice()
    }

    /// Return a tag value by key.
    #[must_use]
    pub fn tag_value(&self, key: &str) -> Option<&str> {
        self.sample_tags.value(key)
    }

    fn with_tags(mut self, tags: SampleTags) -> Self {
        self.sample_tags = tags;
        self
    }
}

impl<'de> Deserialize<'de> for BenchmarkSample {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawBenchmarkSample {
            metric: String,
            kind: BenchmarkMetricKind,
            unit: BenchmarkUnit,
            value: f64,
            #[serde(default, rename = "tags")]
            tags: BTreeMap<String, String>,
        }

        let raw = RawBenchmarkSample::deserialize(deserializer)?;
        let sample_tags = SampleTags::from_owned_map(raw.tags).map_err(serde::de::Error::custom)?;
        Ok(Self {
            metric: Cow::Owned(raw.metric),
            kind: raw.kind,
            unit: raw.unit,
            value: raw.value,
            sample_tags,
        })
    }
}

/// One benchmark sample tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleTag {
    key: Cow<'static, str>,
    value: Cow<'static, str>,
}

impl SampleTag {
    /// Return the tag key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Return the tag value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Small ordered tag set carried on one sample.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleTags {
    tags: SmallVec<[SampleTag; 2]>,
}

impl SampleTags {
    /// Return whether no tags are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// Return the tag count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// Return tags as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[SampleTag] {
        &self.tags
    }

    /// Return a tag value by key.
    #[must_use]
    pub fn value(&self, key: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|tag| tag.key() == key)
            .map(SampleTag::value)
    }

    fn push_static(&mut self, key: &'static str, value: &'static str) -> Result<(), RecorderError> {
        self.push(
            Cow::Borrowed(key),
            Cow::Borrowed(value),
            RecorderError::TagLimitExceeded { key },
        )
    }

    fn push_dynamic(&mut self, key: &'static str, value: String) -> Result<(), RecorderError> {
        self.push(
            Cow::Borrowed(key),
            Cow::Owned(value),
            RecorderError::TagLimitExceeded { key },
        )
    }

    fn push(
        &mut self,
        key: Cow<'static, str>,
        value: Cow<'static, str>,
        error: RecorderError,
    ) -> Result<(), RecorderError> {
        validate_tag(key.as_ref(), value.as_ref(), &error)?;
        if let Some(existing) = self.tags.iter_mut().find(|tag| tag.key == key) {
            existing.value = value;
            return Ok(());
        }
        if self.tags.len() >= MAX_SAMPLE_TAGS {
            return Err(error);
        }
        self.tags.push(SampleTag { key, value });
        Ok(())
    }

    fn from_owned_map(tags: BTreeMap<String, String>) -> Result<Self, String> {
        if tags.len() > MAX_SAMPLE_TAGS {
            return Err(format!(
                "too many sample tags: maximum {MAX_SAMPLE_TAGS}, actual {}",
                tags.len()
            ));
        }
        let mut sample_tags = Self::default();
        for (key, value) in tags {
            validate_tag_str(&key, &value)?;
            sample_tags.tags.push(SampleTag {
                key: Cow::Owned(key),
                value: Cow::Owned(value),
            });
        }
        Ok(sample_tags)
    }
}

impl Serialize for SampleTags {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.tags.len()))?;
        for tag in &self.tags {
            map.serialize_entry(tag.key(), tag.value())?;
        }
        map.end()
    }
}

/// Shared trace-envelope tags.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Tags {
    tags: BTreeMap<String, String>,
}

impl Tags {
    /// Construct an empty shared tag set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the tag count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// Return whether no shared tags are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// Add a static shared tag.
    ///
    /// # Errors
    ///
    /// Returns [`RecorderError::TagLimitExceeded`] if the tag key, value, or
    /// cardinality exceeds the configured hard limits.
    pub fn with_static(
        mut self,
        key: &'static str,
        value: &'static str,
    ) -> Result<Self, RecorderError> {
        self.insert(key, value.to_owned())?;
        Ok(self)
    }

    /// Add a shared tag with a dynamic value.
    ///
    /// # Errors
    ///
    /// Returns [`RecorderError::TagLimitExceeded`] if the tag key, value, or
    /// cardinality exceeds the configured hard limits.
    pub fn with_dynamic(
        mut self,
        key: &'static str,
        value: impl Into<String>,
    ) -> Result<Self, RecorderError> {
        self.insert(key, value.into())?;
        Ok(self)
    }

    /// Return a tag value by key.
    #[must_use]
    pub fn value(&self, key: &str) -> Option<&str> {
        self.tags.get(key).map(String::as_str)
    }

    /// Iterate over shared tags.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.tags
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    fn insert(&mut self, key: &'static str, value: String) -> Result<(), RecorderError> {
        validate_tag(key, &value, &RecorderError::TagLimitExceeded { key })?;
        if !self.tags.contains_key(key) && self.tags.len() >= MAX_SHARED_TAGS {
            return Err(RecorderError::TagLimitExceeded { key });
        }
        self.tags.insert(key.to_owned(), value);
        Ok(())
    }
}

/// Recorder operating profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BenchProfile {
    /// Instrumentation is off.
    Off,
    /// Default low-overhead lifecycle profile.
    #[default]
    Default,
    /// Detailed profile for samplers and low-rate gauges.
    Detailed,
}

/// Recorder construction parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecorderConfig {
    /// Maximum number of retained samples before overflow policy applies.
    pub sample_cap: usize,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            sample_cap: DEFAULT_SAMPLE_CAP,
        }
    }
}

/// Trace recorder.
#[derive(Clone)]
pub enum Recorder {
    /// No-op recorder.
    Disabled,
    /// Enabled recorder.
    Enabled(Arc<EnabledRecorder>),
}

impl Recorder {
    /// Construct a no-op recorder.
    #[must_use]
    pub const fn disabled() -> Self {
        Self::Disabled
    }

    /// Construct an enabled recorder.
    #[must_use]
    pub fn enabled(profile: BenchProfile, tags: Tags) -> Self {
        Self::enabled_with_config(profile, tags, RecorderConfig::default())
    }

    /// Construct an enabled recorder with explicit configuration.
    #[must_use]
    pub fn enabled_with_config(profile: BenchProfile, tags: Tags, config: RecorderConfig) -> Self {
        if profile == BenchProfile::Off {
            return Self::Disabled;
        }
        Self::Enabled(Arc::new(EnabledRecorder::new(profile, tags, config)))
    }

    /// Start a lifecycle latency span.
    #[must_use]
    pub fn span(&self, metric: &'static str) -> Span<'_> {
        self.span_kind(
            metric,
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
        )
    }

    /// Start a span with an explicit kind and unit.
    #[must_use]
    pub fn span_kind(
        &self,
        metric: &'static str,
        kind: BenchmarkMetricKind,
        unit: BenchmarkUnit,
    ) -> Span<'_> {
        Span {
            recorder: self,
            metric,
            kind,
            unit,
            started: self.is_enabled().then(Instant::now),
            sample_tags: SampleTags::default(),
            record_on_drop: self.is_enabled(),
        }
    }

    /// Push one sample to the bus.
    pub fn sample(&self, sample: BenchmarkSample) {
        if let Self::Enabled(enabled) = self {
            enabled.push(sample);
        }
    }

    /// Start a host/guest checkpoint.
    #[must_use]
    pub fn checkpoint(&self, name: &'static str) -> Checkpoint<'_> {
        Checkpoint {
            recorder: self,
            name,
            started: self.is_enabled().then(Instant::now),
        }
    }

    /// Construct a raw sandbox event trace recorder.
    #[must_use]
    pub fn event_trace(
        &self,
        lifecycle: LifecycleClass,
        workload: WorkloadClass,
        profile: RuntimeProfile,
    ) -> EventTraceRecorder {
        EventTraceRecorder::new(lifecycle, workload, profile)
    }

    /// Store one completed raw sandbox event trace.
    pub fn record_event_trace(&self, trace: SandboxEventTrace) {
        if let Self::Enabled(enabled) = self {
            enabled.push_event_trace(trace);
        }
    }

    /// Attach a periodic sampler to this recorder.
    ///
    /// # Errors
    ///
    /// Returns [`RecorderError::NoRuntime`] if called outside a tokio runtime,
    /// or [`RecorderError::Closed`] if the recorder was already closed.
    pub fn attach_sampler<S>(
        &self,
        sampler: S,
        interval: Duration,
    ) -> Result<SamplerId, RecorderError>
    where
        S: Sampler,
    {
        let Self::Enabled(enabled) = self else {
            return Ok(SamplerId::disabled());
        };
        if enabled.closed.load(Ordering::Relaxed) {
            return Err(RecorderError::Closed);
        }

        let handle = tokio::runtime::Handle::try_current().map_err(|_| RecorderError::NoRuntime)?;
        let id = SamplerId(enabled.next_sampler_id.fetch_add(1, Ordering::Relaxed));
        let recorder = self.clone();
        let sampler_name = sampler.name();
        let interval = interval.max(Duration::from_millis(1));
        let join = handle.spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if recorder.is_closed() {
                    break;
                }
                let snapshot = sampler.snapshot().await;
                if recorder.is_closed() {
                    recorder.count_closed_drops(snapshot.len() as u64);
                    break;
                }
                for sample in snapshot {
                    recorder.sample(sample.with_static_tag("sampler", sampler_name));
                }
            }
        });
        enabled
            .samplers
            .lock()
            .push(SamplerHandle { _id: id, join });
        Ok(id)
    }

    /// Drain currently buffered samples without closing sampler tasks.
    #[must_use]
    pub fn drain(&self) -> RecordedTrace {
        match self {
            Self::Disabled => RecordedTrace::empty(),
            Self::Enabled(enabled) => enabled.drain(),
        }
    }

    /// Close this recorder, abort sampler tasks, and drain buffered samples.
    pub async fn close_and_drain(&self) -> RecordedTrace {
        let Self::Enabled(enabled) = self else {
            return RecordedTrace::empty();
        };
        enabled.closed.store(true, Ordering::Release);
        let samplers = std::mem::take(&mut *enabled.samplers.lock());
        for sampler in samplers {
            sampler.join.abort();
            let _ = sampler.join.await;
        }
        enabled.drain()
    }

    /// Return recorder counters.
    #[must_use]
    pub fn stats(&self) -> RecorderStats {
        match self {
            Self::Disabled => RecorderStats::default(),
            Self::Enabled(enabled) => enabled.stats(),
        }
    }

    fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled(_))
    }

    fn is_closed(&self) -> bool {
        match self {
            Self::Disabled => true,
            Self::Enabled(enabled) => enabled.closed.load(Ordering::Acquire),
        }
    }

    fn count_closed_drops(&self, count: u64) {
        if let Self::Enabled(enabled) = self {
            enabled.closed_drops.fetch_add(count, Ordering::Relaxed);
        }
    }
}

/// Enabled recorder state.
pub struct EnabledRecorder {
    samples: Mutex<Vec<BenchmarkSample>>,
    event_traces: Mutex<Vec<SandboxEventTrace>>,
    shared_tags: Arc<Tags>,
    profile: BenchProfile,
    samplers: Mutex<Vec<SamplerHandle>>,
    sample_cap: usize,
    closed: AtomicBool,
    overflow: AtomicU64,
    closed_drops: AtomicU64,
    next_sampler_id: AtomicU64,
}

impl EnabledRecorder {
    fn new(profile: BenchProfile, tags: Tags, config: RecorderConfig) -> Self {
        Self {
            samples: Mutex::new(Vec::with_capacity(config.sample_cap.min(64))),
            event_traces: Mutex::new(Vec::new()),
            shared_tags: Arc::new(tags),
            profile,
            samplers: Mutex::new(Vec::new()),
            sample_cap: config.sample_cap,
            closed: AtomicBool::new(false),
            overflow: AtomicU64::new(0),
            closed_drops: AtomicU64::new(0),
            next_sampler_id: AtomicU64::new(1),
        }
    }

    fn push(&self, sample: BenchmarkSample) {
        if self.closed.load(Ordering::Acquire) {
            self.closed_drops.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if self.sample_cap == 0 {
            self.overflow.fetch_add(1, Ordering::Relaxed);
            return;
        }

        let mut samples = self.samples.lock();
        if samples.len() < self.sample_cap {
            samples.push(sample);
            return;
        }

        if SampleClass::for_sample(&sample) == SampleClass::Lifecycle
            && let Some(index) = samples
                .iter()
                .position(|existing| SampleClass::for_sample(existing) != SampleClass::Lifecycle)
        {
            samples.remove(index);
            samples.push(sample);
            self.overflow.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.overflow.fetch_add(1, Ordering::Relaxed);
    }

    fn push_event_trace(&self, trace: SandboxEventTrace) {
        if self.closed.load(Ordering::Acquire) {
            self.closed_drops.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.event_traces.lock().push(trace);
    }

    fn drain(&self) -> RecordedTrace {
        RecordedTrace {
            shared_tags: Arc::clone(&self.shared_tags),
            samples: std::mem::take(&mut *self.samples.lock()),
            event_traces: std::mem::take(&mut *self.event_traces.lock()),
            overflowed: self.overflow.load(Ordering::Relaxed),
            stats: self.stats(),
        }
    }

    fn stats(&self) -> RecorderStats {
        RecorderStats {
            profile: self.profile,
            overflowed: self.overflow.load(Ordering::Relaxed),
            closed_drops: self.closed_drops.load(Ordering::Relaxed),
        }
    }
}

struct SamplerHandle {
    _id: SamplerId,
    join: tokio::task::JoinHandle<()>,
}

/// Recorder counters that are not ordinary workload samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecorderStats {
    profile: BenchProfile,
    overflowed: u64,
    closed_drops: u64,
}

impl RecorderStats {
    /// Return the recorder profile.
    #[must_use]
    pub const fn profile(&self) -> BenchProfile {
        self.profile
    }

    /// Return how many samples overflowed the cap.
    #[must_use]
    pub const fn overflowed(&self) -> u64 {
        self.overflowed
    }

    /// Return how many samples were submitted after close.
    #[must_use]
    pub const fn closed_drops(&self) -> u64 {
        self.closed_drops
    }
}

/// Drained trace envelope.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedTrace {
    /// Tags shared by the whole trace.
    pub shared_tags: Arc<Tags>,
    /// Samples recorded during the drain window.
    pub samples: Vec<BenchmarkSample>,
    /// Raw sandbox event traces recorded during the drain window.
    pub event_traces: Vec<SandboxEventTrace>,
    /// Count of samples dropped or displaced by overflow policy.
    pub overflowed: u64,
    /// Recorder counters captured at drain time.
    pub stats: RecorderStats,
}

impl RecordedTrace {
    /// Return an empty trace.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            shared_tags: Arc::new(Tags::new()),
            samples: Vec::new(),
            event_traces: Vec::new(),
            overflowed: 0,
            stats: RecorderStats::default(),
        }
    }

    /// Return samples without flattening shared tags.
    #[must_use]
    pub fn into_samples(self) -> Vec<BenchmarkSample> {
        self.samples
    }

    /// Return samples with shared tags copied into per-sample tags.
    #[must_use]
    pub fn into_flat_samples(self) -> Vec<BenchmarkSample> {
        self.samples
            .into_iter()
            .map(|mut sample| {
                for (key, value) in self.shared_tags.iter() {
                    if sample.tag_value(key).is_none() {
                        let _ = sample.sample_tags.push(
                            Cow::Owned(key.to_owned()),
                            Cow::Owned(value.to_owned()),
                            RecorderError::TagLimitExceeded { key: "shared" },
                        );
                    }
                }
                sample
            })
            .collect()
    }
}

/// Span outcome tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanOutcome {
    /// The span completed successfully.
    Ok,
    /// The span completed with an error.
    Error,
    /// The span was dropped without explicit completion.
    Cancelled,
    /// The span was dropped during panic unwinding.
    Panic,
}

impl SpanOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
            Self::Panic => "panic",
        }
    }
}

/// Stable failure class for a failed span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureClass {
    code: Cow<'static, str>,
}

impl FailureClass {
    /// Construct from a static failure code.
    #[must_use]
    pub const fn static_code(code: &'static str) -> Self {
        Self {
            code: Cow::Borrowed(code),
        }
    }

    /// Construct from a dynamic failure code.
    #[must_use]
    pub fn dynamic_code(code: impl Into<String>) -> Self {
        Self {
            code: Cow::Owned(code.into()),
        }
    }

    /// Return the failure code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.code
    }
}

/// RAII span that records on explicit finish or drop.
pub struct Span<'r> {
    recorder: &'r Recorder,
    metric: &'static str,
    kind: BenchmarkMetricKind,
    unit: BenchmarkUnit,
    started: Option<Instant>,
    sample_tags: SampleTags,
    record_on_drop: bool,
}

impl Span<'_> {
    /// Attach a static tag to this span.
    #[must_use]
    pub fn tag_static(mut self, key: &'static str, value: &'static str) -> Self {
        let _ = self.sample_tags.push_static(key, value);
        self
    }

    /// Attach a dynamic tag value to this span.
    #[must_use]
    pub fn tag_dynamic(mut self, key: &'static str, value: impl Into<String>) -> Self {
        let _ = self.sample_tags.push_dynamic(key, value.into());
        self
    }

    /// Tag the span as a cold path.
    #[must_use]
    pub fn cold(self) -> Self {
        self.tag_static("phase_variant", "cold")
    }

    /// Tag the span as a warm path.
    #[must_use]
    pub fn warm(self) -> Self {
        self.tag_static("phase_variant", "warm")
    }

    /// Tag the span as a hot path.
    #[must_use]
    pub fn hot(self) -> Self {
        self.tag_static("phase_variant", "hot")
    }

    /// Finish the span successfully.
    pub fn finish_ok(mut self) {
        self.record(SpanOutcome::Ok, None);
        self.record_on_drop = false;
    }

    /// Finish the span with an error class.
    pub fn finish_error(mut self, class: FailureClass) {
        self.record(SpanOutcome::Error, Some(class));
        self.record_on_drop = false;
    }

    /// Discard this span without recording.
    pub fn discard(mut self) {
        self.record_on_drop = false;
    }

    fn record(&mut self, outcome: SpanOutcome, failure_class: Option<FailureClass>) {
        let Some(started) = self.started else {
            return;
        };
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut tags = self.sample_tags.clone();
        let _ = tags.push_static("outcome", outcome.as_str());
        if let Some(class) = failure_class {
            let _ = tags.push_dynamic("failure_class", class.as_str().to_owned());
        }
        let sample = BenchmarkSample::from_static(self.metric, self.kind, self.unit, elapsed_ms)
            .with_tags(tags);
        self.recorder.sample(sample);
    }
}

impl Drop for Span<'_> {
    fn drop(&mut self) {
        if !self.record_on_drop {
            return;
        }
        let outcome = if std::thread::panicking() {
            SpanOutcome::Panic
        } else {
            SpanOutcome::Cancelled
        };
        self.record(outcome, None);
        self.record_on_drop = false;
    }
}

/// Periodic sample source.
#[async_trait]
pub trait Sampler: Send + Sync + 'static {
    /// Return a stable sampler name.
    fn name(&self) -> &'static str;

    /// Capture one snapshot.
    async fn snapshot(&self) -> Vec<BenchmarkSample>;
}

/// Sampler registration id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SamplerId(u64);

impl SamplerId {
    /// Return the disabled-recorder sentinel id.
    #[must_use]
    pub const fn disabled() -> Self {
        Self(0)
    }

    /// Return whether this is the disabled-recorder sentinel.
    #[must_use]
    pub const fn is_disabled(self) -> bool {
        self.0 == 0
    }
}

/// Paired host/guest checkpoint.
pub struct Checkpoint<'r> {
    recorder: &'r Recorder,
    name: &'static str,
    started: Option<Instant>,
}

impl Checkpoint<'_> {
    /// Record host and guest values plus elapsed checkpoint duration.
    pub fn record_pair(self, host_value: f64, guest_value: f64, unit: BenchmarkUnit) {
        let Some(started) = self.started else {
            return;
        };
        let metric = format!("checkpoint.{}", self.name);
        self.recorder.sample(
            BenchmarkSample::new(
                metric.clone(),
                BenchmarkMetricKind::WorkloadResource,
                unit,
                host_value,
            )
            .with_static_tag("checkpoint", self.name)
            .with_static_tag("side", "host")
            .with_static_tag("clock_domain", "host"),
        );
        self.recorder.sample(
            BenchmarkSample::new(
                metric,
                BenchmarkMetricKind::WorkloadResource,
                unit,
                guest_value,
            )
            .with_static_tag("checkpoint", self.name)
            .with_static_tag("side", "guest")
            .with_static_tag("clock_domain", "guest"),
        );
        self.recorder.sample(
            BenchmarkSample::new(
                format!("checkpoint.{}.elapsed_ms", self.name),
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                started.elapsed().as_secs_f64() * 1000.0,
            )
            .with_static_tag("checkpoint", self.name)
            .with_static_tag("clock_domain", "host"),
        );
    }
}

/// Sample overflow class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleClass {
    /// Lifecycle latency sample.
    Lifecycle,
    /// Gauge/resource sample.
    Gauge,
}

impl SampleClass {
    fn for_sample(sample: &BenchmarkSample) -> Self {
        match sample.kind() {
            BenchmarkMetricKind::LifecycleLatency => Self::Lifecycle,
            BenchmarkMetricKind::FirkinOverhead | BenchmarkMetricKind::WorkloadResource => {
                Self::Gauge
            }
        }
    }
}

/// Recorder API error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum RecorderError {
    /// Sample cap was exceeded.
    #[error("recorder sample cap exceeded: dropped {dropped} sample(s) in class {class:?}")]
    SampleCapExceeded {
        /// Dropped sample count.
        dropped: u64,
        /// Dropped sample class.
        class: SampleClass,
    },
    /// Sampler attach requires a current tokio runtime.
    #[error("sampler attach requires a current tokio runtime")]
    NoRuntime,
    /// Tag limit was exceeded.
    #[error("tag limit exceeded for key `{key}`")]
    TagLimitExceeded {
        /// Tag key that exceeded a hard limit.
        key: &'static str,
    },
    /// Recorder is closed.
    #[error("recorder is closed")]
    Closed,
}

fn validate_tag(key: &str, value: &str, error: &RecorderError) -> Result<(), RecorderError> {
    validate_tag_str(key, value).map_err(|_| error.clone())
}

fn validate_tag_str(key: &str, value: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > MAX_TAG_KEY_BYTES {
        return Err(format!(
            "invalid tag key length: maximum {MAX_TAG_KEY_BYTES}, actual {}",
            key.len()
        ));
    }
    if value.len() > MAX_TAG_VALUE_BYTES {
        return Err(format!(
            "invalid tag value length: maximum {MAX_TAG_VALUE_BYTES}, actual {}",
            value.len()
        ));
    }
    Ok(())
}

/// Canonical lifecycle phase names.
pub mod phase {
    /// Public request accepted.
    pub const REQUEST_RECEIVED: &str = "phase.request_received";
    /// OCI image resolution.
    pub const IMAGE_RESOLVE: &str = "phase.image_resolve";
    /// OCI image pull.
    pub const IMAGE_PULL: &str = "phase.image_pull";
    /// OCI layer unpack.
    pub const IMAGE_UNPACK: &str = "phase.image_unpack";
    /// Rootfs clone.
    pub const ROOTFS_CLONE: &str = "phase.rootfs_clone";
    /// Rootfs preparation.
    pub const ROOTFS_PREPARE: &str = "phase.rootfs_prepare";
    /// Overlay creation.
    pub const OVERLAY_CREATE: &str = "phase.overlay_create";
    /// Workspace creation.
    pub const WORKSPACE_CREATE: &str = "phase.workspace_create";
    /// Workspace ready.
    pub const WORKSPACE_READY: &str = "phase.workspace_ready";
    /// VM config build.
    pub const VM_CONFIG_BUILD: &str = "phase.vm_config_build";
    /// Virtualization config validation.
    pub const VZ_VALIDATE: &str = "phase.vz_validate";
    /// VM object creation.
    pub const VM_CREATE: &str = "phase.vm_create";
    /// Disk attachment.
    pub const DISK_ATTACH: &str = "phase.disk_attach";
    /// VM start call.
    pub const VM_START: &str = "phase.vm_start";
    /// Kernel boot window.
    pub const VM_KERNEL_BOOT: &str = "phase.vm_kernel_boot";
    /// Guest init.
    pub const GUEST_INIT: &str = "phase.guest_init";
    /// Guest agent listening.
    pub const GUEST_AGENT_LISTENING: &str = "phase.guest_agent_listening";
    /// Vsock handshake.
    pub const VSOCK_HANDSHAKE: &str = "phase.vsock_handshake";
    /// Agent handshake.
    pub const AGENT_HANDSHAKE: &str = "phase.agent_handshake";
    /// Network device ready.
    pub const NETWORK_DEVICE_READY: &str = "phase.network_device_ready";
    /// IP assigned.
    pub const IP_ASSIGNED: &str = "phase.ip_assigned";
    /// DNS ready.
    pub const DNS_READY: &str = "phase.dns_ready";
    /// Mounts ready.
    pub const MOUNTS_READY: &str = "phase.mounts_ready";
    /// Cgroups ready.
    pub const CGROUPS_READY: &str = "phase.cgroups_ready";
    /// First exec accepted.
    pub const FIRST_EXEC: &str = "phase.first_exec";
    /// First stdout byte observed.
    pub const FIRST_STDOUT: &str = "phase.first_stdout";
    /// Agent task ready.
    pub const AGENT_TASK_READY: &str = "phase.agent_task_ready";
    /// Graceful process stop.
    pub const PROCESS_GRACEFUL_STOP: &str = "phase.process_graceful_stop";
    /// Forced process kill.
    pub const PROCESS_FORCED_KILL: &str = "phase.process_forced_kill";
    /// VM stop.
    pub const VM_STOP: &str = "phase.vm_stop";
    /// Disk detach.
    pub const DISK_DETACH: &str = "phase.disk_detach";
    /// Filesystem trim.
    pub const FSTRIM: &str = "phase.fstrim";
    /// Total teardown.
    pub const TEARDOWN_TOTAL: &str = "phase.teardown_total";
}

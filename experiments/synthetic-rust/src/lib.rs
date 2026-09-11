//! A deliberately small Rust backend for falsifying the Caliber boundary.
//!
//! This is an experiment, not a reusable application framework. The sample,
//! command, and state schemas below belong to this experiment and must not be
//! moved into `caliber-core`. The core contributes only bounded queues,
//! revisioned publication, immutable resources, and latest-value telemetry.
//!
//! The backend owns a tiny sample library and exposes three independent
//! communication planes:
//!
//! * commands are ordered application operations;
//! * state is an atomically replaced, revisioned publication;
//! * waveform bytes are published once as an immutable resource and meters
//!   use a latest-value telemetry slot.

use caliber_core::{
    Error as CoreError, LatestTelemetry, ResourceHandle, ResourceRegistry, ResourceView, Revision,
    StatePublisher,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

const STATE_SCHEMA: u32 = 1;
const WAVEFORM_KEY: &str = "synthetic-waveform";
const WAVEFORM_KEY_BYTES: [u8; 18] = *b"synthetic-waveform";
const MAX_QUERY_BYTES: usize = 256;
const MAX_STATE_BYTES: usize = 1024;
const METER_WIDTH: usize = 2;

/// The application-owned sample identifiers in the fixture library.
pub const SAMPLE_KICK: u32 = 1;
pub const SAMPLE_SNARE: u32 = 2;
pub const SAMPLE_HAT: u32 = 3;

/// One sample exposed by the experiment's local application schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleSummary {
    pub id: u32,
    pub name: &'static str,
    pub duration_ms: u32,
    pub favorite: bool,
}

/// Application commands. `based_on_revision` is deliberately application
/// policy data: Caliber transports it but does not decide what it means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    SetSearch {
        query: String,
        based_on_revision: Revision,
    },
    SelectSample {
        sample_id: u32,
        based_on_revision: Revision,
    },
    StartPreview {
        sample_id: u32,
        based_on_revision: Revision,
    },
    StopPreview {
        based_on_revision: Revision,
    },
    ToggleFavorite {
        sample_id: u32,
        based_on_revision: Revision,
    },
}

impl Command {
    fn based_on_revision(&self) -> Revision {
        match self {
            Self::SetSearch {
                based_on_revision, ..
            }
            | Self::SelectSample {
                based_on_revision, ..
            }
            | Self::StartPreview {
                based_on_revision, ..
            }
            | Self::StopPreview { based_on_revision }
            | Self::ToggleFavorite {
                based_on_revision, ..
            } => *based_on_revision,
        }
    }
}

/// A local reference carried by state publications instead of waveform bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaveformRef {
    pub resource_key: &'static str,
    pub byte_len: usize,
}

/// The decoded application-visible state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateSnapshot {
    pub revision: Revision,
    pub query: String,
    pub visible_samples: Vec<SampleSummary>,
    pub selected_sample: Option<u32>,
    pub preview_sample: Option<u32>,
    pub favorite_samples: BTreeSet<u32>,
    pub indexing_percent: u8,
    pub waveform: Option<WaveformRef>,
}

/// A meter sample in the local application schema.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterReading {
    pub sequence: Revision,
    pub left_peak: f32,
    pub right_peak: f32,
}

/// A deterministic record of one attempted command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceEntry {
    pub command: Command,
    pub outcome: TraceOutcome,
}

/// Trace outcomes are intentionally explicit, including rejected commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraceOutcome {
    Published(Revision),
    Rejected(BackendError),
}

/// Errors owned by the experiment's application contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    StaleRevision {
        expected: Revision,
        supplied: Revision,
    },
    UnknownSample(u32),
    QueryTooLarge {
        bytes: usize,
        maximum: usize,
    },
    StateTooLarge {
        bytes: usize,
        maximum: usize,
    },
    InvalidState(String),
    Core(CoreError),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, supplied } => {
                write!(
                    f,
                    "stale revision: expected {expected}, supplied {supplied}"
                )
            }
            Self::UnknownSample(id) => write!(f, "unknown sample {id}"),
            Self::QueryTooLarge { bytes, maximum } => {
                write!(f, "search query is {bytes} bytes, maximum is {maximum}")
            }
            Self::StateTooLarge { bytes, maximum } => {
                write!(
                    f,
                    "state publication is {bytes} bytes, maximum is {maximum}"
                )
            }
            Self::InvalidState(reason) => write!(f, "invalid state publication: {reason}"),
            Self::Core(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<CoreError> for BackendError {
    fn from(error: CoreError) -> Self {
        Self::Core(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AppState {
    query: String,
    selected_sample: Option<u32>,
    preview_sample: Option<u32>,
    favorite_samples: BTreeSet<u32>,
    indexing_percent: u8,
    waveform: Option<WaveformRef>,
}

/// The serde/postcard representation is private to this experiment. Fields
/// whose cardinality is known up front are fixed-size values rather than
/// guest-controlled sequences, so decoding cannot allocate for an unbounded
/// favorite list or resource key.
#[derive(Deserialize, Serialize)]
struct WireState {
    query: String,
    selected_sample: Option<u32>,
    preview_sample: Option<u32>,
    indexing_percent: u8,
    favorite_mask: u8,
    waveform: Option<WireWaveformRef>,
}

#[derive(Deserialize, Serialize)]
struct WireWaveformRef {
    resource_key: [u8; WAVEFORM_KEY_BYTES.len()],
    byte_len: u32,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            query: String::new(),
            selected_sample: None,
            preview_sample: None,
            favorite_samples: BTreeSet::new(),
            indexing_percent: 100,
            waveform: None,
        }
    }
}

/// The synthetic backend. It deliberately has no GUI, renderer, Punks, audio
/// device, Wasmtime, FFI, or transport dependency.
pub struct SyntheticBackend {
    samples: Vec<SampleSummary>,
    state: AppState,
    publisher: StatePublisher,
    resources: ResourceRegistry,
    waveform_handle: ResourceHandle,
    waveform_ref: WaveformRef,
    meter: LatestTelemetry,
    trace: Vec<TraceEntry>,
}

impl fmt::Debug for SyntheticBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyntheticBackend")
            .field("samples", &self.samples)
            .field("state", &self.state)
            .field("waveform_ref", &self.waveform_ref)
            .field("trace_entries", &self.trace.len())
            .finish()
    }
}

impl SyntheticBackend {
    /// Builds the fixture and publishes its empty state at revision one.
    pub fn new() -> Result<Self, BackendError> {
        let samples = vec![
            SampleSummary {
                id: SAMPLE_KICK,
                name: "kick",
                duration_ms: 420,
                favorite: false,
            },
            SampleSummary {
                id: SAMPLE_SNARE,
                name: "snare",
                duration_ms: 310,
                favorite: false,
            },
            SampleSummary {
                id: SAMPLE_HAT,
                name: "hat",
                duration_ms: 180,
                favorite: false,
            },
        ];
        let resources = ResourceRegistry::new(4, 64 * 1024)?;
        let waveform = make_waveform();
        let waveform_handle = resources.insert(&waveform)?;
        let waveform_ref = WaveformRef {
            resource_key: WAVEFORM_KEY,
            byte_len: waveform.len(),
        };
        let mut backend = Self {
            samples,
            state: AppState::default(),
            publisher: StatePublisher::new(MAX_STATE_BYTES)?,
            resources,
            waveform_handle,
            waveform_ref,
            meter: LatestTelemetry::new(METER_WIDTH)?,
            trace: Vec::new(),
        };
        backend.publish_state(backend.state.clone())?;
        Ok(backend)
    }

    /// Applies one semantic command using the experiment's reject-on-stale
    /// policy. Rejected commands are retained in the deterministic trace but
    /// cannot replace the last valid state.
    pub fn dispatch(&mut self, command: Command) -> Result<StateSnapshot, BackendError> {
        let supplied = command.based_on_revision();
        let expected = self.current_revision();
        if supplied != expected {
            let error = BackendError::StaleRevision { expected, supplied };
            self.trace.push(TraceEntry {
                command,
                outcome: TraceOutcome::Rejected(error.clone()),
            });
            return Err(error);
        }

        let mut candidate = self.state.clone();
        let result = match &command {
            Command::SetSearch { query, .. } => {
                if query.len() > MAX_QUERY_BYTES {
                    Err(BackendError::QueryTooLarge {
                        bytes: query.len(),
                        maximum: MAX_QUERY_BYTES,
                    })
                } else {
                    candidate.query = query.clone();
                    Ok(())
                }
            }
            Command::SelectSample { sample_id, .. } => self.require_sample(*sample_id).map(|()| {
                candidate.selected_sample = Some(*sample_id);
                candidate.waveform = Some(self.waveform_ref.clone());
            }),
            Command::StartPreview { sample_id, .. } => self.require_sample(*sample_id).map(|()| {
                candidate.preview_sample = Some(*sample_id);
            }),
            Command::StopPreview { .. } => {
                candidate.preview_sample = None;
                Ok(())
            }
            Command::ToggleFavorite { sample_id, .. } => {
                self.require_sample(*sample_id).map(|()| {
                    if !candidate.favorite_samples.insert(*sample_id) {
                        candidate.favorite_samples.remove(sample_id);
                    }
                })
            }
        };
        if let Err(error) = result {
            self.trace.push(TraceEntry {
                command,
                outcome: TraceOutcome::Rejected(error.clone()),
            });
            return Err(error);
        }

        let revision = match self.publish_state(candidate.clone()) {
            Ok(revision) => revision,
            Err(error) => {
                self.trace.push(TraceEntry {
                    command,
                    outcome: TraceOutcome::Rejected(error.clone()),
                });
                return Err(error);
            }
        };
        self.state = candidate;
        self.trace.push(TraceEntry {
            command,
            outcome: TraceOutcome::Published(revision),
        });
        self.read_state()
    }

    /// Reads the latest state publication and decodes the local application
    /// schema. The immutable publication is the source of the returned
    /// revision and visible state; it is not a view of mutable backend fields.
    pub fn read_state(&self) -> Result<StateSnapshot, BackendError> {
        let publication = self
            .publisher
            .read()
            .ok_or_else(|| BackendError::InvalidState("no initial publication".into()))?;
        let state = decode_state(publication.payload())?;
        let visible_samples = self.visible_samples(&state);
        Ok(StateSnapshot {
            revision: publication.revision(),
            query: state.query,
            visible_samples,
            selected_sample: state.selected_sample,
            preview_sample: state.preview_sample,
            favorite_samples: state.favorite_samples,
            indexing_percent: state.indexing_percent,
            waveform: state.waveform,
        })
    }

    /// Maps the one waveform reference to an immutable core-owned view.
    pub fn map_waveform(&self, reference: &WaveformRef) -> Result<ResourceView, BackendError> {
        if reference != &self.waveform_ref {
            return Err(BackendError::InvalidState(
                "unknown waveform reference".into(),
            ));
        }
        Ok(self.resources.get(self.waveform_handle)?)
    }

    /// Publishes a meter sample without allocating or retaining history.
    pub fn publish_meter(&self, left_peak: f32, right_peak: f32) -> Result<Revision, BackendError> {
        if !left_peak.is_finite() || !right_peak.is_finite() {
            return Err(BackendError::InvalidState("meter must be finite".into()));
        }
        Ok(self
            .meter
            .publish(&[left_peak.to_bits() as usize, right_peak.to_bits() as usize])?)
    }

    /// Reads the newest meter sample. Older samples may have been overwritten.
    pub fn read_meter(&self) -> Result<MeterReading, BackendError> {
        let mut values = [0; METER_WIDTH];
        let sequence = self.meter.read_into(&mut values)?;
        Ok(MeterReading {
            sequence,
            left_peak: f32::from_bits(values[0] as u32),
            right_peak: f32::from_bits(values[1] as u32),
        })
    }

    /// Returns a copy of the deterministic command trace.
    pub fn trace(&self) -> Vec<TraceEntry> {
        self.trace.clone()
    }

    /// Returns the number of immutable registry entries. The fixture should
    /// retain exactly one waveform resource throughout normal operation.
    pub fn live_resource_count(&self) -> usize {
        self.resources.len()
    }

    /// Test-only lifecycle probe: release the registry lease while a state
    /// publication still contains the reference. Existing views remain valid,
    /// while a new map operation must fail closed.
    #[cfg(test)]
    fn release_waveform_for_test(&self) -> Result<(), BackendError> {
        Ok(self.resources.release(self.waveform_handle)?)
    }

    fn current_revision(&self) -> Revision {
        self.publisher.latest_revision().unwrap_or(0)
    }

    fn require_sample(&self, id: u32) -> Result<(), BackendError> {
        self.samples
            .iter()
            .any(|sample| sample.id == id)
            .then_some(())
            .ok_or(BackendError::UnknownSample(id))
    }

    fn visible_samples(&self, state: &AppState) -> Vec<SampleSummary> {
        let query = state.query.to_ascii_lowercase();
        self.samples
            .iter()
            .filter(|sample| query.is_empty() || sample.name.contains(&query))
            .map(|sample| SampleSummary {
                favorite: state.favorite_samples.contains(&sample.id),
                ..sample.clone()
            })
            .collect()
    }

    fn publish_state(&mut self, candidate: AppState) -> Result<Revision, BackendError> {
        let payload = encode_state(&candidate)?;
        // Decode before publishing so malformed or structurally incomplete
        // application payloads cannot replace the previous atomic publication.
        let decoded = decode_state(&payload)?;
        self.validate_state(&decoded)?;
        let revision = self.publisher.publish(STATE_SCHEMA, &payload)?;
        debug_assert_eq!(decoded, candidate);
        self.state = candidate;
        Ok(revision)
    }

    #[cfg(test)]
    fn try_publish_raw_for_test(&mut self, payload: &[u8]) -> Result<Revision, BackendError> {
        let candidate = decode_state(payload)?;
        self.validate_state(&candidate)?;
        let revision = self.publisher.publish(STATE_SCHEMA, payload)?;
        self.state = candidate;
        Ok(revision)
    }

    fn validate_state(&self, state: &AppState) -> Result<(), BackendError> {
        for sample_id in state
            .selected_sample
            .into_iter()
            .chain(state.preview_sample)
            .chain(state.favorite_samples.iter().copied())
        {
            self.require_sample(sample_id)?;
        }
        if let Some(reference) = &state.waveform
            && reference != &self.waveform_ref
        {
            return Err(BackendError::InvalidState(
                "waveform reference does not name the immutable resource".into(),
            ));
        }
        Ok(())
    }
}

/// Replays a fixed command list against a fresh backend and returns its trace.
pub fn replay_trace(
    commands: &[Command],
) -> Result<(Vec<TraceEntry>, StateSnapshot), BackendError> {
    let mut backend = SyntheticBackend::new()?;
    for command in commands.iter().cloned() {
        let _ = backend.dispatch(command);
    }
    Ok((backend.trace(), backend.read_state()?))
}

fn make_waveform() -> Vec<u8> {
    (0..64)
        .map(|index| {
            let phase = index as f32 / 63.0;
            ((phase * std::f32::consts::TAU).sin().abs() * 255.0) as u8
        })
        .collect()
}

fn encode_state(state: &AppState) -> Result<Vec<u8>, BackendError> {
    if state.query.len() > MAX_QUERY_BYTES {
        return Err(BackendError::QueryTooLarge {
            bytes: state.query.len(),
            maximum: MAX_QUERY_BYTES,
        });
    }
    let mut favorite_mask = 0;
    for sample_id in &state.favorite_samples {
        favorite_mask |= favorite_bit(*sample_id)
            .ok_or_else(|| BackendError::InvalidState("unknown favorite sample".into()))?;
    }
    let waveform = state
        .waveform
        .as_ref()
        .map(|reference| {
            if reference.resource_key != WAVEFORM_KEY || reference.byte_len > u32::MAX as usize {
                return Err(BackendError::InvalidState(
                    "unknown waveform reference".into(),
                ));
            }
            Ok(WireWaveformRef {
                resource_key: WAVEFORM_KEY_BYTES,
                byte_len: reference.byte_len as u32,
            })
        })
        .transpose()?;
    let wire = WireState {
        query: state.query.clone(),
        selected_sample: state.selected_sample,
        preview_sample: state.preview_sample,
        indexing_percent: state.indexing_percent,
        favorite_mask,
        waveform,
    };
    let bytes = postcard::to_allocvec(&wire)
        .map_err(|error| BackendError::InvalidState(format!("postcard encode failed: {error}")))?;
    if bytes.len() > MAX_STATE_BYTES {
        return Err(BackendError::StateTooLarge {
            bytes: bytes.len(),
            maximum: MAX_STATE_BYTES,
        });
    }
    Ok(bytes)
}

fn decode_state(bytes: &[u8]) -> Result<AppState, BackendError> {
    if bytes.len() > MAX_STATE_BYTES {
        return Err(BackendError::StateTooLarge {
            bytes: bytes.len(),
            maximum: MAX_STATE_BYTES,
        });
    }
    let (wire, trailing) = postcard::take_from_bytes::<WireState>(bytes)
        .map_err(|error| BackendError::InvalidState(format!("postcard decode failed: {error}")))?;
    if !trailing.is_empty() {
        return Err(BackendError::InvalidState("trailing state bytes".into()));
    }
    if wire.query.len() > MAX_QUERY_BYTES {
        return Err(BackendError::QueryTooLarge {
            bytes: wire.query.len(),
            maximum: MAX_QUERY_BYTES,
        });
    }
    if wire.indexing_percent > 100 {
        return Err(BackendError::InvalidState(
            "indexing percentage exceeds 100".into(),
        ));
    }
    let mut favorite_samples = BTreeSet::new();
    for sample_id in [SAMPLE_KICK, SAMPLE_SNARE, SAMPLE_HAT] {
        if wire.favorite_mask & favorite_bit(sample_id).expect("fixture IDs have bits") != 0 {
            favorite_samples.insert(sample_id);
        }
    }
    if wire.favorite_mask & !known_favorite_mask() != 0 {
        return Err(BackendError::InvalidState(
            "favorite mask contains unknown samples".into(),
        ));
    }
    let waveform = match wire.waveform {
        None => None,
        Some(reference) => {
            if reference.resource_key != WAVEFORM_KEY_BYTES {
                return Err(BackendError::InvalidState(
                    "unknown waveform resource key".into(),
                ));
            }
            Some(WaveformRef {
                resource_key: WAVEFORM_KEY,
                byte_len: reference.byte_len as usize,
            })
        }
    };
    Ok(AppState {
        query: wire.query,
        selected_sample: wire.selected_sample,
        preview_sample: wire.preview_sample,
        favorite_samples,
        indexing_percent: wire.indexing_percent,
        waveform,
    })
}

fn favorite_bit(sample_id: u32) -> Option<u8> {
    match sample_id {
        SAMPLE_KICK => Some(1 << 0),
        SAMPLE_SNARE => Some(1 << 1),
        SAMPLE_HAT => Some(1 << 2),
        _ => None,
    }
}

fn known_favorite_mask() -> u8 {
    favorite_bit(SAMPLE_KICK).expect("fixture IDs have bits")
        | favorite_bit(SAMPLE_SNARE).expect("fixture IDs have bits")
        | favorite_bit(SAMPLE_HAT).expect("fixture IDs have bits")
}

#[cfg(test)]
fn encode_wire_state_for_test(wire: &WireState) -> Vec<u8> {
    postcard::to_allocvec(wire).expect("test wire state is serializable")
}

#[cfg(test)]
fn valid_wire_state() -> WireState {
    WireState {
        query: String::new(),
        selected_sample: None,
        preview_sample: None,
        indexing_percent: 100,
        favorite_mask: 0,
        waveform: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_revision(revision: Revision, command: impl FnOnce(Revision) -> Command) -> Command {
        command(revision)
    }

    #[test]
    fn command_state_and_waveform_use_separate_planes() {
        let mut backend = SyntheticBackend::new().unwrap();
        assert_eq!(backend.read_state().unwrap().revision, 1);
        assert_eq!(backend.live_resource_count(), 1);

        let state = backend
            .dispatch(Command::SetSearch {
                query: "snare".into(),
                based_on_revision: 1,
            })
            .unwrap();
        assert_eq!(state.visible_samples[0].name, "snare");
        let state = backend
            .dispatch(Command::SelectSample {
                sample_id: SAMPLE_SNARE,
                based_on_revision: state.revision,
            })
            .unwrap();
        let reference = state.waveform.clone().expect("selection publishes ref");
        let first_view = backend.map_waveform(&reference).unwrap();
        assert_eq!(first_view.bytes().len(), reference.byte_len);
        assert_eq!(backend.live_resource_count(), 1);

        let state = backend
            .dispatch(Command::StartPreview {
                sample_id: SAMPLE_SNARE,
                based_on_revision: state.revision,
            })
            .unwrap();
        assert_eq!(state.preview_sample, Some(SAMPLE_SNARE));
        let second_view = backend.map_waveform(&reference).unwrap();
        assert_eq!(first_view.bytes(), second_view.bytes());
    }

    #[test]
    fn stale_commands_are_rejected_without_a_new_publication() {
        let mut backend = SyntheticBackend::new().unwrap();
        let before = backend.read_state().unwrap();
        let error = backend
            .dispatch(Command::SetSearch {
                query: "late".into(),
                based_on_revision: before.revision - 1,
            })
            .unwrap_err();
        assert_eq!(
            error,
            BackendError::StaleRevision {
                expected: before.revision,
                supplied: before.revision - 1,
            }
        );
        assert_eq!(backend.read_state().unwrap(), before);
        assert!(matches!(
            backend.trace()[0].outcome,
            TraceOutcome::Rejected(_)
        ));
    }

    #[test]
    fn malformed_candidate_preserves_last_atomic_publication() {
        let mut backend = SyntheticBackend::new().unwrap();
        let before = backend.read_state().unwrap();
        let error = backend
            .try_publish_raw_for_test(b"not-a-state")
            .unwrap_err();
        assert!(matches!(error, BackendError::InvalidState(_)));
        assert_eq!(backend.read_state().unwrap(), before);
    }

    #[test]
    fn oversized_state_is_rejected_before_postcard_decode() {
        let mut backend = SyntheticBackend::new().unwrap();
        let before = backend.read_state().unwrap();
        let error = backend
            .try_publish_raw_for_test(&vec![0xff; MAX_STATE_BYTES + 1])
            .unwrap_err();
        assert_eq!(
            error,
            BackendError::StateTooLarge {
                bytes: MAX_STATE_BYTES + 1,
                maximum: MAX_STATE_BYTES,
            }
        );
        assert_eq!(backend.read_state().unwrap(), before);
    }

    #[test]
    fn postcard_state_keeps_favorites_and_resource_reference_fixed_size() {
        let mut wire = valid_wire_state();
        wire.favorite_mask = favorite_bit(SAMPLE_SNARE).unwrap();
        wire.waveform = Some(WireWaveformRef {
            resource_key: WAVEFORM_KEY_BYTES,
            byte_len: 64,
        });
        let decoded = decode_state(&encode_wire_state_for_test(&wire)).unwrap();
        assert_eq!(decoded.favorite_samples, BTreeSet::from([SAMPLE_SNARE]));
        assert_eq!(decoded.waveform.unwrap().byte_len, 64);

        wire.favorite_mask = 0b1000;
        let error = decode_state(&encode_wire_state_for_test(&wire)).unwrap_err();
        assert!(matches!(error, BackendError::InvalidState(_)));
    }

    #[test]
    fn waveform_reference_fails_closed_after_resource_release() {
        let mut backend = SyntheticBackend::new().unwrap();
        let state = backend
            .dispatch(Command::SelectSample {
                sample_id: SAMPLE_KICK,
                based_on_revision: 1,
            })
            .unwrap();
        let reference = state.waveform.unwrap();
        let view = backend.map_waveform(&reference).unwrap();
        backend.release_waveform_for_test().unwrap();
        assert_eq!(view.bytes().len(), reference.byte_len);
        assert!(matches!(
            backend.map_waveform(&reference),
            Err(BackendError::Core(CoreError::StaleResource(_)))
        ));
    }

    #[test]
    fn meter_keeps_only_the_newest_coherent_value() {
        let backend = SyntheticBackend::new().unwrap();
        for index in 0..500 {
            backend
                .publish_meter(index as f32, (index as f32) + 0.5)
                .unwrap();
        }
        let reading = backend.read_meter().unwrap();
        assert_eq!(reading.sequence, 500);
        assert_eq!(reading.left_peak, 499.0);
        assert_eq!(reading.right_peak, 499.5);
    }

    #[test]
    fn deterministic_trace_replays_identically() {
        let commands = vec![
            at_revision(1, |revision| Command::SetSearch {
                query: "s".into(),
                based_on_revision: revision,
            }),
            at_revision(2, |revision| Command::SelectSample {
                sample_id: SAMPLE_SNARE,
                based_on_revision: revision,
            }),
            at_revision(3, |revision| Command::ToggleFavorite {
                sample_id: SAMPLE_SNARE,
                based_on_revision: revision,
            }),
        ];
        let first = replay_trace(&commands).unwrap();
        let second = replay_trace(&commands).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.1.revision, 4);
        assert!(first.1.favorite_samples.contains(&SAMPLE_SNARE));
    }

    #[test]
    fn invalid_command_does_not_advance_revision() {
        let mut backend = SyntheticBackend::new().unwrap();
        let before = backend.read_state().unwrap();
        let error = backend
            .dispatch(Command::SelectSample {
                sample_id: 99,
                based_on_revision: before.revision,
            })
            .unwrap_err();
        assert_eq!(error, BackendError::UnknownSample(99));
        assert_eq!(backend.read_state().unwrap(), before);
    }
}

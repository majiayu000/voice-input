use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const ASR_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioChunk {
    pub session_id: SessionId,
    pub sequence: u64,
    pub sample_rate: u32,
    pub samples: Vec<i16>,
}

impl AudioChunk {
    pub fn duration(&self) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.samples.len() as f64 / self.sample_rate as f64)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    pub id: SessionId,
    pub sample_rate: u32,
    pub started_at: Instant,
}

impl SessionContext {
    pub fn new() -> Self {
        Self {
            id: SessionId::new(),
            sample_rate: ASR_SAMPLE_RATE,
            started_at: Instant::now(),
        }
    }
}

impl Default for SessionContext {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptUpdate {
    pub text: String,
    pub stable: bool,
    pub audio_end: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub text: String,
    pub confidence: Option<f32>,
}

impl Transcript {
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            confidence: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefineRequest {
    pub transcript: String,
    pub partials: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    Idle,
    Capturing,
    Finalizing,
    Inserting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackCue {
    Ready,
    CaptureStarted,
    Finalizing,
    Completed,
    Error,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyReport {
    pub capture_ms: u64,
    pub first_audio_ms: Option<u64>,
    pub finalize_ms: u64,
    pub refine_ms: u64,
    pub insert_ms: u64,
    pub stop_to_inserted_ms: u64,
    pub total_ms: u64,
    pub audio_chunks: u64,
    pub audio_samples: u64,
    pub dropped_chunks: u64,
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub text: String,
    pub latency: LatencyReport,
}

#[derive(Debug)]
pub(crate) struct SessionTiming {
    started: Instant,
    first_audio: Option<Instant>,
    stop_requested: Option<Instant>,
    asr_final: Option<Instant>,
    refine_final: Option<Instant>,
    inserted: Option<Instant>,
    audio_chunks: u64,
    audio_samples: u64,
    dropped_chunks: u64,
}

impl SessionTiming {
    pub(crate) fn new(started: Instant) -> Self {
        Self {
            started,
            first_audio: None,
            stop_requested: None,
            asr_final: None,
            refine_final: None,
            inserted: None,
            audio_chunks: 0,
            audio_samples: 0,
            dropped_chunks: 0,
        }
    }

    pub(crate) fn record_audio(&mut self, samples: usize) {
        self.first_audio.get_or_insert_with(Instant::now);
        self.audio_chunks += 1;
        self.audio_samples += samples as u64;
    }

    pub(crate) fn record_drop(&mut self) {
        self.dropped_chunks += 1;
    }

    pub(crate) fn stop(&mut self) {
        self.stop_requested = Some(Instant::now());
    }

    pub(crate) fn asr_final(&mut self) {
        self.asr_final = Some(Instant::now());
    }

    pub(crate) fn refine_final(&mut self) {
        self.refine_final = Some(Instant::now());
    }

    pub(crate) fn inserted(&mut self) {
        self.inserted = Some(Instant::now());
    }

    pub(crate) fn report(&self) -> LatencyReport {
        let stop = self.stop_requested.unwrap_or(self.started);
        let asr = self.asr_final.unwrap_or(stop);
        let refined = self.refine_final.unwrap_or(asr);
        let inserted = self.inserted.unwrap_or(refined);

        LatencyReport {
            capture_ms: millis(stop.saturating_duration_since(self.started)),
            first_audio_ms: self
                .first_audio
                .map(|at| millis(at.saturating_duration_since(self.started))),
            finalize_ms: millis(asr.saturating_duration_since(stop)),
            refine_ms: millis(refined.saturating_duration_since(asr)),
            insert_ms: millis(inserted.saturating_duration_since(refined)),
            stop_to_inserted_ms: millis(inserted.saturating_duration_since(stop)),
            total_ms: millis(inserted.saturating_duration_since(self.started)),
            audio_chunks: self.audio_chunks,
            audio_samples: self.audio_samples,
            dropped_chunks: self.dropped_chunks,
        }
    }
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

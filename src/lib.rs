pub mod asr;
pub mod backend;
pub mod config;
pub mod domain;
pub mod metrics;
pub mod models;
pub mod pipeline;
pub mod platform;
pub mod ports;
pub mod refiner;
pub mod runtime;
pub mod service;
pub mod status;
mod tls;

pub use config::{InsertionMode, VoiceConfig};
pub use domain::{
    AudioChunk, Completion, EngineState, FeedbackCue, LatencyReport, RefineRequest, SessionContext,
    SessionId, Transcript, TranscriptUpdate,
};
pub use metrics::{read_latency_history, FileLatencySink, LatencySample, LatencySummary};
pub use models::{install_model, model_catalog, InstalledModel, ModelArtifact, ModelPreset};
pub use pipeline::VoiceEngine;
pub use ports::{Feedback, LatencySink, Refiner, StreamingAsr, TextInjector};
pub use runtime::{RuntimeCommand, RuntimeEffects, RuntimeExit, VoiceRuntime};
pub use service::{ServiceManager, ServicePaths, ServiceStatus};
pub use status::{FileStatusPublisher, InstanceLease, RuntimePhase, RuntimeSnapshot};

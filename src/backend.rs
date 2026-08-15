use crate::domain::{
    AudioChunk, FeedbackCue, RefineRequest, SessionContext, Transcript, TranscriptUpdate,
};
use crate::ports::{Feedback, LatencySink, Refiner, StreamingAsr, TextInjector};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

pub struct ScriptedAsr {
    text: String,
    active: bool,
}

impl ScriptedAsr {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            active: false,
        }
    }
}

#[async_trait]
impl StreamingAsr for ScriptedAsr {
    async fn begin(&mut self, _session: &SessionContext) -> anyhow::Result<()> {
        self.active = true;
        Ok(())
    }

    async fn push_audio(&mut self, _chunk: AudioChunk) -> anyhow::Result<Vec<TranscriptUpdate>> {
        anyhow::ensure!(self.active, "scripted ASR session is not active");
        Ok(Vec::new())
    }

    async fn finish(&mut self) -> anyhow::Result<Transcript> {
        anyhow::ensure!(self.active, "scripted ASR session is not active");
        self.active = false;
        Ok(Transcript {
            text: self.text.clone(),
            confidence: Some(1.0),
        })
    }

    async fn cancel(&mut self) -> anyhow::Result<()> {
        self.active = false;
        Ok(())
    }
}

#[derive(Default)]
pub struct IdentityRefiner;

#[async_trait]
impl Refiner for IdentityRefiner {
    async fn refine(&mut self, request: RefineRequest) -> anyhow::Result<String> {
        Ok(request.transcript)
    }
}

#[derive(Clone, Default)]
pub struct MemoryInjector {
    inserted: Arc<Mutex<Vec<String>>>,
}

impl MemoryInjector {
    pub fn values(&self) -> Vec<String> {
        self.inserted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl TextInjector for MemoryInjector {
    async fn insert(&mut self, text: &str) -> anyhow::Result<()> {
        self.inserted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(text.to_owned());
        Ok(())
    }
}

#[derive(Default)]
pub struct StdoutInjector;

#[async_trait]
impl TextInjector for StdoutInjector {
    async fn insert(&mut self, text: &str) -> anyhow::Result<()> {
        println!("{text}");
        Ok(())
    }
}

#[derive(Default)]
pub struct NoopFeedback;

impl Feedback for NoopFeedback {
    fn emit(&mut self, _cue: FeedbackCue) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NoopLatencySink;

impl LatencySink for NoopLatencySink {
    fn record(&mut self, _report: &crate::domain::LatencyReport) -> anyhow::Result<()> {
        Ok(())
    }
}

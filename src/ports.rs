use crate::domain::{
    AudioChunk, FeedbackCue, LatencyReport, RefineRequest, SessionContext, Transcript,
    TranscriptUpdate,
};
use async_trait::async_trait;

#[async_trait]
pub trait StreamingAsr: Send {
    async fn begin(&mut self, session: &SessionContext) -> anyhow::Result<()>;

    async fn push_audio(&mut self, chunk: AudioChunk) -> anyhow::Result<Vec<TranscriptUpdate>>;

    async fn finish(&mut self) -> anyhow::Result<Transcript>;

    async fn cancel(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait]
pub trait Refiner: Send {
    async fn refine(&mut self, request: RefineRequest) -> anyhow::Result<String>;
}

#[async_trait]
pub trait TextInjector: Send {
    async fn insert(&mut self, text: &str) -> anyhow::Result<()>;
}

pub trait Feedback: Send {
    fn emit(&mut self, cue: FeedbackCue) -> anyhow::Result<()>;
}

pub trait LatencySink: Send {
    fn record(&mut self, report: &LatencyReport) -> anyhow::Result<()>;
}

#[async_trait]
impl<T> StreamingAsr for Box<T>
where
    T: StreamingAsr + ?Sized,
{
    async fn begin(&mut self, session: &SessionContext) -> anyhow::Result<()> {
        (**self).begin(session).await
    }

    async fn push_audio(&mut self, chunk: AudioChunk) -> anyhow::Result<Vec<TranscriptUpdate>> {
        (**self).push_audio(chunk).await
    }

    async fn finish(&mut self) -> anyhow::Result<Transcript> {
        (**self).finish().await
    }

    async fn cancel(&mut self) -> anyhow::Result<()> {
        (**self).cancel().await
    }
}

#[async_trait]
impl<T> Refiner for Box<T>
where
    T: Refiner + ?Sized,
{
    async fn refine(&mut self, request: RefineRequest) -> anyhow::Result<String> {
        (**self).refine(request).await
    }
}

#[async_trait]
impl<T> TextInjector for Box<T>
where
    T: TextInjector + ?Sized,
{
    async fn insert(&mut self, text: &str) -> anyhow::Result<()> {
        (**self).insert(text).await
    }
}

impl<T> Feedback for Box<T>
where
    T: Feedback + ?Sized,
{
    fn emit(&mut self, cue: FeedbackCue) -> anyhow::Result<()> {
        (**self).emit(cue)
    }
}

impl<T> LatencySink for Box<T>
where
    T: LatencySink + ?Sized,
{
    fn record(&mut self, report: &LatencyReport) -> anyhow::Result<()> {
        (**self).record(report)
    }
}

use crate::domain::{
    AudioChunk, Completion, EngineState, RefineRequest, SessionContext, SessionTiming,
};
use crate::ports::{Refiner, StreamingAsr, TextInjector};

pub struct VoiceEngine<A, R, I> {
    asr: A,
    refiner: R,
    injector: I,
    state: EngineState,
    session: Option<SessionContext>,
    timing: Option<SessionTiming>,
    partials: Vec<String>,
}

impl<A, R, I> VoiceEngine<A, R, I>
where
    A: StreamingAsr,
    R: Refiner,
    I: TextInjector,
{
    pub fn new(asr: A, refiner: R, injector: I) -> Self {
        Self {
            asr,
            refiner,
            injector,
            state: EngineState::Idle,
            session: None,
            timing: None,
            partials: Vec::new(),
        }
    }

    pub fn state(&self) -> EngineState {
        self.state
    }

    pub fn session(&self) -> Option<&SessionContext> {
        self.session.as_ref()
    }

    pub async fn begin(&mut self) -> anyhow::Result<SessionContext> {
        anyhow::ensure!(
            self.state == EngineState::Idle,
            "cannot start capture while engine is {:?}",
            self.state
        );

        let session = SessionContext::new();
        self.asr.begin(&session).await?;
        self.timing = Some(SessionTiming::new(session.started_at));
        self.session = Some(session.clone());
        self.partials.clear();
        self.state = EngineState::Capturing;
        Ok(session)
    }

    pub async fn push_audio(&mut self, chunk: AudioChunk) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.state == EngineState::Capturing,
            "cannot push audio while engine is {:?}",
            self.state
        );
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("active session is missing"))?;
        if chunk.session_id != session.id {
            if let Some(timing) = self.timing.as_mut() {
                timing.record_drop();
            }
            return Ok(());
        }

        if let Some(timing) = self.timing.as_mut() {
            timing.record_audio(chunk.samples.len());
        }
        let updates = self.asr.push_audio(chunk).await?;
        self.partials.extend(
            updates
                .into_iter()
                .filter(|update| update.stable && !update.text.trim().is_empty())
                .map(|update| update.text),
        );
        Ok(())
    }

    pub async fn finish(&mut self) -> anyhow::Result<Option<Completion>> {
        anyhow::ensure!(
            self.state == EngineState::Capturing,
            "cannot finish capture while engine is {:?}",
            self.state
        );
        self.state = EngineState::Finalizing;
        if let Some(timing) = self.timing.as_mut() {
            timing.stop();
        }

        let transcript = match self.asr.finish().await {
            Ok(transcript) => transcript,
            Err(error) => {
                self.reset();
                return Err(error);
            }
        };
        if let Some(timing) = self.timing.as_mut() {
            timing.asr_final();
        }

        if transcript.text.trim().is_empty() {
            let latency = self
                .timing
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("session timing is missing"))?
                .report();
            self.reset();
            tracing::debug!(?latency, "empty transcript; nothing to insert");
            return Ok(None);
        }

        let refined = match self
            .refiner
            .refine(RefineRequest {
                transcript: transcript.text,
                partials: self.partials.clone(),
            })
            .await
        {
            Ok(text) => text,
            Err(error) => {
                self.reset();
                return Err(error);
            }
        };
        if let Some(timing) = self.timing.as_mut() {
            timing.refine_final();
        }

        if refined.trim().is_empty() {
            self.reset();
            return Ok(None);
        }

        self.state = EngineState::Inserting;
        if let Err(error) = self.injector.insert(&refined).await {
            self.reset();
            return Err(error);
        }
        if let Some(timing) = self.timing.as_mut() {
            timing.inserted();
        }

        let latency = self
            .timing
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("session timing is missing"))?
            .report();
        let completion = Completion {
            text: refined,
            latency,
        };
        self.reset();
        Ok(Some(completion))
    }

    pub async fn cancel(&mut self) -> anyhow::Result<()> {
        if self.state != EngineState::Idle {
            self.asr.cancel().await?;
        }
        self.reset();
        Ok(())
    }

    fn reset(&mut self) {
        self.state = EngineState::Idle;
        self.session = None;
        self.timing = None;
        self.partials.clear();
    }

    pub fn into_parts(self) -> (A, R, I) {
        (self.asr, self.refiner, self.injector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{IdentityRefiner, MemoryInjector, ScriptedAsr};
    use crate::domain::ASR_SAMPLE_RATE;

    fn chunk(session: &SessionContext, sequence: u64) -> AudioChunk {
        AudioChunk {
            session_id: session.id,
            sequence,
            sample_rate: ASR_SAMPLE_RATE,
            samples: vec![0; 1_280],
        }
    }

    #[tokio::test]
    async fn completes_capture_refine_insert_cycle() {
        let injector = MemoryInjector::default();
        let observer = injector.clone();
        let mut engine =
            VoiceEngine::new(ScriptedAsr::new("hello locally"), IdentityRefiner, injector);

        let session = engine.begin().await.unwrap();
        assert_eq!(engine.state(), EngineState::Capturing);
        engine.push_audio(chunk(&session, 0)).await.unwrap();
        let result = engine.finish().await.unwrap().unwrap();

        assert_eq!(result.text, "hello locally");
        assert_eq!(result.latency.audio_chunks, 1);
        assert_eq!(result.latency.audio_samples, 1_280);
        assert_eq!(observer.values(), vec!["hello locally"]);
        assert_eq!(engine.state(), EngineState::Idle);
    }

    #[tokio::test]
    async fn ignores_audio_from_old_session() {
        let mut engine = VoiceEngine::new(
            ScriptedAsr::new("ok"),
            IdentityRefiner,
            MemoryInjector::default(),
        );
        let current = engine.begin().await.unwrap();
        let mut stale = chunk(&current, 0);
        stale.session_id = crate::domain::SessionId::new();

        engine.push_audio(stale).await.unwrap();
        let result = engine.finish().await.unwrap().unwrap();
        assert_eq!(result.latency.audio_chunks, 0);
        assert_eq!(result.latency.dropped_chunks, 1);
    }

    #[tokio::test]
    async fn rejects_overlapping_capture() {
        let mut engine = VoiceEngine::new(
            ScriptedAsr::new("ok"),
            IdentityRefiner,
            MemoryInjector::default(),
        );
        engine.begin().await.unwrap();
        assert!(engine.begin().await.is_err());
        engine.cancel().await.unwrap();
        assert_eq!(engine.state(), EngineState::Idle);
    }
}

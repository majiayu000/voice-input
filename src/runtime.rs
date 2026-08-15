use crate::config::VoiceConfig;
use crate::domain::{EngineState, FeedbackCue};
use crate::pipeline::VoiceEngine;
use crate::platform::{AudioCapture, HotkeyEvent};
use crate::ports::{Feedback, LatencySink, Refiner, StreamingAsr, TextInjector};
use crate::status::{RuntimePhase, RuntimeSnapshot, StatusPublisher};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeCommand {
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeExit {
    ShutdownRequested,
    HotkeyStreamClosed,
}

pub struct RuntimeEffects {
    status: Box<dyn StatusPublisher>,
    feedback: Box<dyn Feedback>,
    latency: Box<dyn LatencySink>,
}

impl RuntimeEffects {
    pub fn new(
        status: impl StatusPublisher + 'static,
        feedback: impl Feedback + 'static,
        latency: impl LatencySink + 'static,
    ) -> Self {
        Self {
            status: Box::new(status),
            feedback: Box::new(feedback),
            latency: Box::new(latency),
        }
    }
}

pub struct VoiceRuntime<C, A, R, I> {
    config: VoiceConfig,
    capture: C,
    engine: VoiceEngine<A, R, I>,
    effects: RuntimeEffects,
    snapshot: RuntimeSnapshot,
}

impl<C, A, R, I> VoiceRuntime<C, A, R, I>
where
    C: AudioCapture,
    A: StreamingAsr,
    R: Refiner,
    I: TextInjector,
{
    pub fn new(
        config: VoiceConfig,
        capture: C,
        engine: VoiceEngine<A, R, I>,
        effects: RuntimeEffects,
    ) -> Self {
        let snapshot = RuntimeSnapshot::starting(config.hotkey.clone());
        Self {
            config,
            capture,
            engine,
            effects,
            snapshot,
        }
    }

    pub async fn run(
        mut self,
        mut hotkeys: mpsc::Receiver<HotkeyEvent>,
        mut commands: mpsc::Receiver<RuntimeCommand>,
    ) -> anyhow::Result<RuntimeExit> {
        self.publish_status();
        let (audio_sender, mut audio_receiver) = mpsc::channel(self.config.audio.channel_capacity);
        match self
            .capture
            .prepare(&self.config.audio, audio_sender.clone())
        {
            Ok(device) => tracing::info!(
                device = %device.name,
                input_sample_rate = device.input_sample_rate,
                output_sample_rate = device.output_sample_rate,
                "microphone is warm and ready"
            ),
            Err(error) => {
                self.snapshot.record_error(error.to_string());
                self.emit_feedback(FeedbackCue::Error);
                self.snapshot.transition(RuntimePhase::Stopped);
                self.publish_status();
                return Err(error);
            }
        }
        self.snapshot.transition(RuntimePhase::Ready);
        self.publish_status();
        self.emit_feedback(FeedbackCue::Ready);

        let mut command_stream_open = true;
        let exit = loop {
            tokio::select! {
                command = commands.recv(), if command_stream_open => {
                    match command {
                        Some(RuntimeCommand::Shutdown) => break RuntimeExit::ShutdownRequested,
                        None => command_stream_open = false,
                    }
                }
                event = hotkeys.recv() => {
                    let Some(event) = event else {
                        break RuntimeExit::HotkeyStreamClosed;
                    };
                    if self
                        .handle_hotkey(
                            event,
                            &audio_sender,
                            &mut audio_receiver,
                            &mut commands,
                        )
                        .await
                    {
                        break RuntimeExit::ShutdownRequested;
                    }
                }
                chunk = audio_receiver.recv(), if self.engine.state() == EngineState::Capturing => {
                    if let Some(chunk) = chunk {
                        if let Err(error) = self.engine.push_audio(chunk).await {
                            self.recover_session("streaming ASR rejected an audio chunk", error).await;
                        }
                    }
                }
            }
        };

        self.shutdown().await;
        Ok(exit)
    }

    async fn handle_hotkey(
        &mut self,
        event: HotkeyEvent,
        audio_sender: &mpsc::Sender<crate::domain::AudioChunk>,
        audio_receiver: &mut mpsc::Receiver<crate::domain::AudioChunk>,
        commands: &mut mpsc::Receiver<RuntimeCommand>,
    ) -> bool {
        match event {
            HotkeyEvent::Pressed if self.engine.state() == EngineState::Idle => {
                match self.engine.begin().await {
                    Ok(session) => {
                        match self.capture.start(
                            session.id,
                            &self.config.audio,
                            audio_sender.clone(),
                        ) {
                            Ok(device) => {
                                self.snapshot.active_session = Some(session.id.to_string());
                                self.snapshot.last_error = None;
                                self.snapshot.transition(RuntimePhase::Capturing);
                                self.publish_status();
                                self.emit_feedback(FeedbackCue::CaptureStarted);
                                tracing::info!(
                                    session = %session.id,
                                    device = %device.name,
                                    input_sample_rate = device.input_sample_rate,
                                    output_sample_rate = device.output_sample_rate,
                                    "capture started"
                                );
                            }
                            Err(error) => {
                                self.recover_session("failed to start microphone", error)
                                    .await;
                            }
                        }
                    }
                    Err(error) => {
                        self.record_recoverable_error("failed to start voice session", error)
                    }
                }
            }
            HotkeyEvent::Released if self.engine.state() == EngineState::Capturing => {
                if let Err(error) = self.capture.stop() {
                    self.record_recoverable_error("failed to stop microphone", error);
                }
                while let Ok(chunk) = audio_receiver.try_recv() {
                    if let Err(error) = self.engine.push_audio(chunk).await {
                        self.recover_session("failed to process trailing audio", error)
                            .await;
                        return false;
                    }
                }
                self.snapshot.transition(RuntimePhase::Finalizing);
                self.publish_status();
                self.emit_feedback(FeedbackCue::Finalizing);
                let completion = {
                    let finish = self.engine.finish();
                    tokio::pin!(finish);
                    tokio::select! {
                        completion = &mut finish => Some(completion),
                        command = commands.recv() => {
                            match command {
                                Some(RuntimeCommand::Shutdown) => None,
                                None => Some(finish.await),
                            }
                        }
                    }
                };
                let Some(completion) = completion else {
                    return true;
                };
                match completion {
                    Ok(Some(completion)) => {
                        self.snapshot.sessions_completed += 1;
                        self.snapshot.last_latency = Some(completion.latency.clone());
                        self.snapshot.last_error = None;
                        if let Err(error) = self.effects.latency.record(&completion.latency) {
                            tracing::warn!(%error, "failed to record latency sample");
                        }
                        self.emit_feedback(FeedbackCue::Completed);
                        match serde_json::to_string(&completion.latency) {
                            Ok(latency) => tracing::info!(
                                text_chars = completion.text.chars().count(),
                                latency = %latency,
                                "dictation completed"
                            ),
                            Err(error) => {
                                tracing::warn!(%error, "failed to serialize latency report")
                            }
                        }
                    }
                    Ok(None) => tracing::info!("dictation produced no text"),
                    Err(error) => self.record_recoverable_error("dictation failed", error),
                }
                self.snapshot.active_session = None;
                self.snapshot.transition(RuntimePhase::Ready);
                self.publish_status();
            }
            _ => {}
        }
        false
    }

    async fn recover_session(&mut self, context: &'static str, error: anyhow::Error) {
        if let Err(stop_error) = self.capture.stop() {
            tracing::error!(%stop_error, "failed to stop capture during recovery");
        }
        if let Err(cancel_error) = self.engine.cancel().await {
            tracing::error!(%cancel_error, "failed to cancel ASR session during recovery");
        }
        self.snapshot.active_session = None;
        self.record_recoverable_error(context, error);
        self.snapshot.transition(RuntimePhase::Ready);
        self.publish_status();
    }

    fn record_recoverable_error(&mut self, context: &'static str, error: anyhow::Error) {
        tracing::error!(%error, "{context}");
        self.snapshot.record_error(format!("{context}: {error}"));
        self.publish_status();
        self.emit_feedback(FeedbackCue::Error);
    }

    async fn shutdown(&mut self) {
        self.snapshot.transition(RuntimePhase::Stopping);
        self.publish_status();
        if self.engine.state() != EngineState::Idle {
            if let Err(error) = self.capture.stop() {
                tracing::error!(%error, "failed to stop capture during shutdown");
            }
            if let Err(error) = self.engine.cancel().await {
                tracing::error!(%error, "failed to cancel ASR during shutdown");
            }
        }
        self.snapshot.active_session = None;
        self.snapshot.transition(RuntimePhase::Stopped);
        self.publish_status();
        self.emit_feedback(FeedbackCue::Stopped);
    }

    fn publish_status(&mut self) {
        if let Err(error) = self.effects.status.publish(&self.snapshot) {
            tracing::warn!(%error, "failed to publish runtime status");
        }
    }

    fn emit_feedback(&mut self, cue: FeedbackCue) {
        if let Err(error) = self.effects.feedback.emit(cue) {
            tracing::warn!(?cue, %error, "failed to emit feedback cue");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        IdentityRefiner, MemoryInjector, NoopFeedback, NoopLatencySink, ScriptedAsr,
    };
    use crate::domain::{AudioChunk, SessionId, ASR_SAMPLE_RATE};
    use crate::platform::AudioDeviceReport;
    use crate::ports::StreamingAsr;
    use crate::status::StatusPublisher;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Notify;

    #[derive(Default)]
    struct FakeCapture;

    impl AudioCapture for FakeCapture {
        fn prepare(
            &mut self,
            _config: &crate::config::AudioConfig,
            _sender: mpsc::Sender<AudioChunk>,
        ) -> anyhow::Result<AudioDeviceReport> {
            Ok(device())
        }

        fn start(
            &mut self,
            session_id: SessionId,
            _config: &crate::config::AudioConfig,
            sender: mpsc::Sender<AudioChunk>,
        ) -> anyhow::Result<AudioDeviceReport> {
            sender.try_send(AudioChunk {
                session_id,
                sequence: 0,
                sample_rate: ASR_SAMPLE_RATE,
                samples: vec![0; 1_280],
            })?;
            Ok(device())
        }

        fn stop(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn device() -> AudioDeviceReport {
        AudioDeviceReport {
            name: "fake microphone".to_owned(),
            input_sample_rate: ASR_SAMPLE_RATE,
            output_sample_rate: ASR_SAMPLE_RATE,
            channels: 1,
            sample_format: "i16".to_owned(),
            native_16khz: true,
        }
    }

    #[derive(Clone, Default)]
    struct MemoryStatus(Arc<Mutex<Vec<RuntimeSnapshot>>>);

    impl StatusPublisher for MemoryStatus {
        fn publish(&mut self, snapshot: &RuntimeSnapshot) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(snapshot.clone());
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct MemoryFeedback(Arc<Mutex<Vec<FeedbackCue>>>);

    impl Feedback for MemoryFeedback {
        fn emit(&mut self, cue: FeedbackCue) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(cue);
            Ok(())
        }
    }

    #[tokio::test]
    async fn headless_runtime_completes_session_and_stops_cleanly() {
        let injector = MemoryInjector::default();
        let inserted = injector.clone();
        let statuses = MemoryStatus::default();
        let observed_statuses = statuses.clone();
        let feedback = MemoryFeedback::default();
        let observed_feedback = feedback.clone();
        let engine = VoiceEngine::new(ScriptedAsr::new("hello runtime"), IdentityRefiner, injector);
        let runtime = VoiceRuntime::new(
            VoiceConfig::default(),
            FakeCapture,
            engine,
            RuntimeEffects::new(statuses, feedback, NoopLatencySink),
        );
        let (hotkey_sender, hotkeys) = mpsc::channel(4);
        hotkey_sender.send(HotkeyEvent::Pressed).await.unwrap();
        hotkey_sender.send(HotkeyEvent::Released).await.unwrap();
        drop(hotkey_sender);
        let (_command_sender, commands) = mpsc::channel(1);

        let exit = runtime.run(hotkeys, commands).await.unwrap();

        assert_eq!(exit, RuntimeExit::HotkeyStreamClosed);
        assert_eq!(inserted.values(), vec!["hello runtime"]);
        let phases: Vec<_> = observed_statuses
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|snapshot| snapshot.phase)
            .collect();
        assert!(phases.contains(&RuntimePhase::Ready));
        assert!(phases.contains(&RuntimePhase::Capturing));
        assert!(phases.contains(&RuntimePhase::Finalizing));
        assert_eq!(phases.last(), Some(&RuntimePhase::Stopped));
        assert_eq!(
            *observed_feedback.0.lock().unwrap(),
            vec![
                FeedbackCue::Ready,
                FeedbackCue::CaptureStarted,
                FeedbackCue::Finalizing,
                FeedbackCue::Completed,
                FeedbackCue::Stopped,
            ]
        );
    }

    struct BlockingFinishAsr {
        finalizing: Arc<Notify>,
        active: bool,
    }

    #[async_trait]
    impl StreamingAsr for BlockingFinishAsr {
        async fn begin(&mut self, _session: &crate::domain::SessionContext) -> anyhow::Result<()> {
            self.active = true;
            Ok(())
        }

        async fn push_audio(
            &mut self,
            _chunk: AudioChunk,
        ) -> anyhow::Result<Vec<crate::domain::TranscriptUpdate>> {
            Ok(Vec::new())
        }

        async fn finish(&mut self) -> anyhow::Result<crate::domain::Transcript> {
            self.finalizing.notify_one();
            std::future::pending().await
        }

        async fn cancel(&mut self) -> anyhow::Result<()> {
            self.active = false;
            Ok(())
        }
    }

    #[tokio::test]
    async fn shutdown_cancels_a_stuck_finalization() {
        let finalizing = Arc::new(Notify::new());
        let engine = VoiceEngine::new(
            BlockingFinishAsr {
                finalizing: Arc::clone(&finalizing),
                active: false,
            },
            IdentityRefiner,
            MemoryInjector::default(),
        );
        let statuses = MemoryStatus::default();
        let observed_statuses = statuses.clone();
        let runtime = VoiceRuntime::new(
            VoiceConfig::default(),
            FakeCapture,
            engine,
            RuntimeEffects::new(statuses, NoopFeedback, NoopLatencySink),
        );
        let (hotkey_sender, hotkeys) = mpsc::channel(4);
        let (command_sender, commands) = mpsc::channel(1);
        let task = tokio::spawn(runtime.run(hotkeys, commands));

        hotkey_sender.send(HotkeyEvent::Pressed).await.unwrap();
        hotkey_sender.send(HotkeyEvent::Released).await.unwrap();
        finalizing.notified().await;
        command_sender.send(RuntimeCommand::Shutdown).await.unwrap();

        let exit = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("runtime did not honor shutdown while finalizing")
            .unwrap()
            .unwrap();
        assert_eq!(exit, RuntimeExit::ShutdownRequested);
        assert_eq!(
            observed_statuses.0.lock().unwrap().last().unwrap().phase,
            RuntimePhase::Stopped
        );
    }
}

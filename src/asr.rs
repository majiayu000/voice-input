use crate::config::AsrConfig;
#[cfg(target_os = "macos")]
use crate::config::VadConfig;
#[cfg(target_os = "macos")]
use crate::domain::ASR_SAMPLE_RATE;
use crate::domain::{AudioChunk, SessionContext, Transcript, TranscriptUpdate};
use crate::ports::StreamingAsr;
use async_trait::async_trait;
use std::path::Path;

#[cfg(target_os = "macos")]
mod whisper {
    use super::*;
    use std::ffi::c_void;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    };
    use std::thread;
    use tokio::sync::oneshot;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    struct WorkerConfig {
        language: String,
        initial_prompt: Option<String>,
        threads: i32,
    }

    struct AbortContext {
        current_generation: Arc<AtomicU64>,
        generation: u64,
    }

    unsafe extern "C" fn should_abort(user_data: *mut c_void) -> bool {
        // SAFETY: `transcribe` keeps this context alive for the complete
        // synchronous `WhisperState::full` call and whisper.cpp invokes the
        // callback only during that call.
        let context = unsafe { &*(user_data.cast::<AbortContext>()) };
        context.current_generation.load(Ordering::Acquire) != context.generation
    }

    enum WorkerCommand {
        Transcribe {
            generation: u64,
            samples: Vec<f32>,
            response: oneshot::Sender<Result<String, String>>,
        },
        Shutdown,
    }

    pub struct LocalWhisperAsr {
        sender: mpsc::Sender<WorkerCommand>,
        worker: Option<thread::JoinHandle<()>>,
        current_generation: Arc<AtomicU64>,
        active_generation: Option<u64>,
        samples: Vec<f32>,
        max_samples: usize,
        vad: VadConfig,
    }

    impl LocalWhisperAsr {
        pub fn new(config: &AsrConfig) -> anyhow::Result<Self> {
            let model_path = config.model_path.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "asr.model_path is not configured; run `voice-input model install --preset balanced`"
                )
            })?;
            anyhow::ensure!(
                model_path.is_file(),
                "Whisper model does not exist at {}",
                model_path.display()
            );

            let (sender, receiver) = mpsc::channel();
            let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
            let current_generation = Arc::new(AtomicU64::new(0));
            let worker_generation = Arc::clone(&current_generation);
            let model_path = model_path.to_path_buf();
            let worker_config = WorkerConfig {
                language: config.language.clone(),
                initial_prompt: config.initial_prompt.clone(),
                threads: config.threads.min(i32::MAX as usize) as i32,
            };
            let use_gpu = config.use_gpu;
            let flash_attention = config.flash_attention;
            let model_path_string = model_path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Whisper model path is not valid UTF-8"))?
                .to_owned();
            let worker = thread::Builder::new()
                .name("voice-input-whisper".to_owned())
                .spawn(move || {
                    whisper_rs::install_logging_hooks();
                    let mut context_parameters = WhisperContextParameters::default();
                    context_parameters
                        .use_gpu(use_gpu)
                        .flash_attn(flash_attention);
                    let context = match WhisperContext::new_with_params(
                        &model_path_string,
                        context_parameters,
                    ) {
                        Ok(context) => context,
                        Err(error) => {
                            let _ = startup_sender.send(Err(format!(
                                "failed to load Whisper model {}: {error}",
                                model_path.display()
                            )));
                            return;
                        }
                    };
                    let mut state = match context.create_state() {
                        Ok(state) => {
                            let _ = startup_sender.send(Ok(()));
                            state
                        }
                        Err(error) => {
                            let _ = startup_sender.send(Err(format!(
                                "failed to prewarm Whisper inference state: {error}"
                            )));
                            return;
                        }
                    };

                    while let Ok(command) = receiver.recv() {
                        match command {
                            WorkerCommand::Transcribe {
                                generation,
                                samples,
                                response,
                            } => {
                                let result = transcribe(
                                    &mut state,
                                    &worker_config,
                                    &worker_generation,
                                    generation,
                                    &samples,
                                )
                                .map_err(|error| error.to_string());
                                let _ = response.send(result);
                            }
                            WorkerCommand::Shutdown => break,
                        }
                    }
                })?;

            match startup_receiver.recv() {
                Ok(Ok(())) => Ok(Self {
                    sender,
                    worker: Some(worker),
                    current_generation,
                    active_generation: None,
                    samples: Vec::new(),
                    max_samples: (config.max_audio_seconds as usize)
                        .saturating_mul(ASR_SAMPLE_RATE as usize),
                    vad: config.vad.clone(),
                }),
                Ok(Err(error)) => {
                    let _ = worker.join();
                    anyhow::bail!(error)
                }
                Err(error) => {
                    let _ = worker.join();
                    anyhow::bail!("Whisper model worker stopped during startup: {error}")
                }
            }
        }
    }

    #[async_trait]
    impl StreamingAsr for LocalWhisperAsr {
        async fn begin(&mut self, _session: &SessionContext) -> anyhow::Result<()> {
            anyhow::ensure!(
                self.active_generation.is_none(),
                "Whisper session is already active"
            );
            let generation = self.current_generation.fetch_add(1, Ordering::AcqRel) + 1;
            self.active_generation = Some(generation);
            self.samples.clear();
            Ok(())
        }

        async fn push_audio(&mut self, chunk: AudioChunk) -> anyhow::Result<Vec<TranscriptUpdate>> {
            anyhow::ensure!(
                self.active_generation.is_some(),
                "Whisper session is not active"
            );
            anyhow::ensure!(
                chunk.sample_rate == ASR_SAMPLE_RATE,
                "Whisper requires 16 kHz audio, received {} Hz",
                chunk.sample_rate
            );
            anyhow::ensure!(
                self.samples.len().saturating_add(chunk.samples.len()) <= self.max_samples,
                "dictation exceeded the configured {} second ASR limit",
                self.max_samples / ASR_SAMPLE_RATE as usize
            );
            self.samples.extend(
                chunk
                    .samples
                    .into_iter()
                    .map(|sample| sample as f32 / 32_768.0),
            );
            Ok(Vec::new())
        }

        async fn finish(&mut self) -> anyhow::Result<Transcript> {
            let generation = self
                .active_generation
                .take()
                .ok_or_else(|| anyhow::anyhow!("Whisper session is not active"))?;
            let samples = trim_silence(std::mem::take(&mut self.samples), &self.vad);
            if samples.is_empty() {
                return Ok(Transcript::empty());
            }
            let (response_sender, response_receiver) = oneshot::channel();
            self.sender
                .send(WorkerCommand::Transcribe {
                    generation,
                    samples,
                    response: response_sender,
                })
                .map_err(|_| anyhow::anyhow!("Whisper model worker has stopped"))?;
            let text = response_receiver
                .await
                .map_err(|_| anyhow::anyhow!("Whisper model worker dropped the result"))?
                .map_err(anyhow::Error::msg)?;
            Ok(Transcript {
                text: text.trim().to_owned(),
                confidence: None,
            })
        }

        async fn cancel(&mut self) -> anyhow::Result<()> {
            self.current_generation.fetch_add(1, Ordering::AcqRel);
            self.active_generation = None;
            self.samples.clear();
            Ok(())
        }
    }

    impl Drop for LocalWhisperAsr {
        fn drop(&mut self) {
            self.current_generation.fetch_add(1, Ordering::AcqRel);
            let _ = self.sender.send(WorkerCommand::Shutdown);
            if let Some(worker) = self.worker.take() {
                if worker.join().is_err() {
                    tracing::warn!("Whisper model worker panicked");
                }
            }
        }
    }

    fn transcribe(
        state: &mut whisper_rs::WhisperState,
        config: &WorkerConfig,
        current_generation: &Arc<AtomicU64>,
        generation: u64,
        samples: &[f32],
    ) -> anyhow::Result<String> {
        let mut parameters = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        parameters.set_n_threads(config.threads);
        parameters.set_language(Some(config.language.as_str()));
        if let Some(prompt) = config.initial_prompt.as_deref() {
            parameters.set_initial_prompt(prompt);
        }
        parameters.set_translate(false);
        parameters.set_no_context(true);
        parameters.set_suppress_blank(true);
        parameters.set_suppress_nst(true);
        parameters.set_print_progress(false);
        parameters.set_print_realtime(false);
        parameters.set_print_special(false);
        parameters.set_print_timestamps(false);
        let mut abort_context = AbortContext {
            current_generation: Arc::clone(current_generation),
            generation,
        };
        // SAFETY: `abort_context` has a stable address until `state.full`
        // returns, and `should_abort` only reads its atomic generation token.
        unsafe {
            parameters.set_abort_callback(Some(should_abort));
            parameters.set_abort_callback_user_data(
                (&mut abort_context as *mut AbortContext).cast::<c_void>(),
            );
        }
        state.full(parameters, samples)?;
        Ok(state
            .as_iter()
            .map(|segment| segment.to_string())
            .collect::<Vec<_>>()
            .join("")
            .trim()
            .to_owned())
    }
}

#[cfg(target_os = "macos")]
pub use whisper::LocalWhisperAsr;

#[cfg(not(target_os = "macos"))]
pub struct LocalWhisperAsr;

#[cfg(not(target_os = "macos"))]
impl LocalWhisperAsr {
    pub fn new(_config: &AsrConfig) -> anyhow::Result<Self> {
        anyhow::bail!("the in-process Whisper provider currently supports macOS only")
    }
}

#[cfg(not(target_os = "macos"))]
#[async_trait]
impl StreamingAsr for LocalWhisperAsr {
    async fn begin(&mut self, _session: &SessionContext) -> anyhow::Result<()> {
        anyhow::bail!("the in-process Whisper provider currently supports macOS only")
    }

    async fn push_audio(&mut self, _chunk: AudioChunk) -> anyhow::Result<Vec<TranscriptUpdate>> {
        anyhow::bail!("the in-process Whisper provider currently supports macOS only")
    }

    async fn finish(&mut self) -> anyhow::Result<Transcript> {
        anyhow::bail!("the in-process Whisper provider currently supports macOS only")
    }
}

#[cfg(any(target_os = "macos", test))]
fn trim_silence(samples: Vec<f32>, config: &VadConfig) -> Vec<f32> {
    if !config.enabled || samples.is_empty() {
        return samples;
    }
    let frame_samples = (ASR_SAMPLE_RATE as usize * 20) / 1_000;
    let active_frames: Vec<_> = samples
        .chunks(frame_samples)
        .enumerate()
        .filter_map(|(index, frame)| {
            let energy =
                frame.iter().map(|sample| sample * sample).sum::<f32>() / frame.len().max(1) as f32;
            (energy.sqrt() >= config.rms_threshold).then_some(index)
        })
        .collect();
    let minimum_frames = (config.min_speech_ms as usize).div_ceil(20);
    if active_frames.len() < minimum_frames {
        return Vec::new();
    }
    let padding = (config.speech_pad_ms as usize * ASR_SAMPLE_RATE as usize) / 1_000;
    let start = active_frames[0]
        .saturating_mul(frame_samples)
        .saturating_sub(padding);
    let end = active_frames[active_frames.len() - 1]
        .saturating_add(1)
        .saturating_mul(frame_samples)
        .saturating_add(padding)
        .min(samples.len());
    samples[start..end].to_vec()
}

pub fn model_is_ready(config: &AsrConfig) -> bool {
    config
        .model_path
        .as_deref()
        .map(Path::is_file)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vad_trims_silence_and_keeps_padding() {
        let mut samples = vec![0.0; ASR_SAMPLE_RATE as usize];
        samples.extend(vec![0.2; ASR_SAMPLE_RATE as usize / 5]);
        samples.extend(vec![0.0; ASR_SAMPLE_RATE as usize]);
        let config = VadConfig {
            speech_pad_ms: 100,
            ..VadConfig::default()
        };

        let trimmed = trim_silence(samples, &config);

        assert_eq!(trimmed.len(), ASR_SAMPLE_RATE as usize * 4 / 10);
    }

    #[test]
    fn vad_rejects_short_noise() {
        let samples = vec![0.2; ASR_SAMPLE_RATE as usize / 100];

        assert!(trim_silence(samples, &VadConfig::default()).is_empty());
    }
}

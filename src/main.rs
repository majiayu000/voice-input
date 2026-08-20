use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;
use tracing_subscriber::EnvFilter;
use voice_input::asr::{model_is_ready, LocalWhisperAsr};
use voice_input::backend::{IdentityRefiner, ScriptedAsr, StdoutInjector};
use voice_input::config::{AsrBackend, RefinerBackend};
use voice_input::domain::ASR_SAMPLE_RATE;
use voice_input::platform::{
    accessibility_is_trusted, install_hotkey, install_shutdown_handler, probe_audio_device,
    record_wav, request_permission, run_main_event_loop, system_permission_snapshot, AudioCapture,
    MacAudioCapture, MacClipboardInjector, MacSystemFeedback, MainEventLoopHandle,
    PlatformServiceManager,
};
use voice_input::ports::{Refiner, StreamingAsr, TextInjector};
use voice_input::refiner::OpenAiCompatibleRefiner;
use voice_input::{
    install_model, model_catalog, read_latency_history, FileLatencySink, FileStatusPublisher,
    InstanceLease, LatencySummary, ModelPreset, RuntimeCommand, RuntimeEffects, ServiceManager,
    ServicePaths, VoiceConfig, VoiceEngine, VoiceRuntime,
};

#[derive(Debug, Parser)]
#[command(name = "voice-input")]
#[command(about = "Low-latency, local-first voice input foundation for macOS")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a default configuration file if it does not exist.
    Init,
    /// Inspect microphone, Accessibility permission, and active configuration.
    Doctor {
        /// Ask macOS to show the Accessibility permission prompt when needed.
        #[arg(long)]
        prompt_accessibility: bool,
    },
    /// Record 16 kHz mono PCM to a WAV file, without invoking a model.
    Record {
        #[arg(long, default_value_t = 3.0)]
        seconds: f64,
        #[arg(long, default_value = "voice-input-probe.wav")]
        output: PathBuf,
    },
    /// Insert text into the currently focused application.
    Inject {
        text: String,
        /// Print the text instead of using the macOS pasteboard and Command-V.
        #[arg(long)]
        stdout: bool,
    },
    /// Install a stable binary and user LaunchAgent without starting it.
    Install,
    /// Start or restart the installed user LaunchAgent.
    Start,
    /// Stop the installed user LaunchAgent without uninstalling it.
    Stop,
    /// Print structured launchd and runtime status.
    Status,
    /// Stop the service and remove its binary and LaunchAgent.
    Uninstall,
    /// Summarize recorded end-to-end latency as p50/p95 JSON.
    Benchmark {
        /// Read a specific latency JSONL file instead of the service history.
        #[arg(long)]
        input: Option<PathBuf>,
    },
    /// Inspect or install local ASR models.
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
    /// Transcribe a 16 kHz mono WAV through the configured local ASR.
    Transcribe { input: PathBuf },
    /// Capture one utterance for a fixed duration and run the complete local pipeline.
    Dictate {
        #[arg(long, default_value_t = 5.0)]
        seconds: f64,
        /// Print final text instead of inserting it into the focused application.
        #[arg(long)]
        stdout: bool,
    },
    /// Run the press-and-hold dictation daemon.
    Daemon {
        /// Fixed transcript used to verify capture -> refine -> insertion before ASR is added.
        #[arg(long, default_value = "")]
        mock_text: String,
        /// Print final text instead of inserting it into the focused application.
        #[arg(long)]
        stdout: bool,
    },
    /// Versioned JSON control plane for the macOS application.
    Control {
        #[command(subcommand)]
        command: ControlCommand,
    },
    /// Inspect or request macOS permissions for this executable identity.
    Permission {
        #[command(subcommand)]
        command: PermissionCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ControlCommand {
    /// Print one aggregate GUI snapshot as JSON.
    Snapshot,
    /// Apply a SettingsPatch JSON object read from standard input.
    Apply,
    /// Send a short user-initiated probe through the configured LLM refiner.
    TestRefiner,
}

#[derive(Debug, Subcommand)]
enum PermissionCommand {
    /// Print the permission state for this exact executable identity.
    Snapshot,
    /// Request one permission and print the resulting state.
    Request {
        #[arg(value_enum)]
        permission: PermissionArgument,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PermissionArgument {
    Microphone,
    Accessibility,
    InputMonitoring,
}

impl From<PermissionArgument> for voice_input::PermissionKind {
    fn from(value: PermissionArgument) -> Self {
        match value {
            PermissionArgument::Microphone => Self::Microphone,
            PermissionArgument::Accessibility => Self::Accessibility,
            PermissionArgument::InputMonitoring => Self::InputMonitoring,
        }
    }
}

#[derive(Debug, Subcommand)]
enum ModelCommand {
    /// Print the built-in, integrity-pinned model catalog.
    List,
    /// Download, verify, and configure a local Whisper model.
    Install {
        #[arg(long, value_enum, default_value_t = ModelPresetArgument::Balanced)]
        preset: ModelPresetArgument,
        /// Atomically replace a corrupt or unexpected file at the destination.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModelPresetArgument {
    Fast,
    Balanced,
    Quality,
}

impl From<ModelPresetArgument> for ModelPreset {
    fn from(value: ModelPresetArgument) -> Self {
        match value {
            ModelPresetArgument::Fast => Self::Fast,
            ModelPresetArgument::Balanced => Self::Balanced,
            ModelPresetArgument::Quality => Self::Quality,
        }
    }
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    platform: &'static str,
    config_path: PathBuf,
    config: VoiceConfig,
    accessibility_trusted: bool,
    audio: Result<voice_input::platform::AudioDeviceReport, String>,
    asr_backend: AsrBackend,
    asr_model_ready: bool,
    refiner_backend: RefinerBackend,
    refiner_endpoint: Option<String>,
}

#[derive(Debug, Serialize)]
struct TranscribeReport {
    input: PathBuf,
    audio_ms: u64,
    model_load_ms: u64,
    inference_ms: u64,
    real_time_factor: f64,
    text: String,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,whisper_rs=error")),
        )
        .with_target(false)
        .init();

    let Cli {
        config: config_path,
        command,
    } = Cli::parse();
    match command {
        Command::Init => {
            let path = VoiceConfig::write_default(config_path.as_deref())?;
            println!("{}", path.display());
        }
        Command::Doctor {
            prompt_accessibility,
        } => {
            let config = load_config(config_path.as_deref())?;
            let report = DoctorReport {
                platform: std::env::consts::OS,
                config_path: config_path
                    .clone()
                    .unwrap_or_else(VoiceConfig::default_path),
                accessibility_trusted: accessibility_is_trusted(prompt_accessibility),
                audio: probe_audio_device(config.audio.sample_rate)
                    .map_err(|error| error.to_string()),
                asr_backend: config.asr.backend,
                asr_model_ready: config.asr.backend == AsrBackend::Scripted
                    || model_is_ready(&config.asr),
                refiner_backend: config.refiner.backend,
                refiner_endpoint: (config.refiner.backend == RefinerBackend::OpenaiCompatible)
                    .then(|| config.refiner.base_url.clone()),
                config,
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Record { seconds, output } => {
            let config = load_config(config_path.as_deref())?;
            anyhow::ensure!(
                seconds.is_finite() && seconds > 0.0,
                "seconds must be positive"
            );
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let mut capture = MacAudioCapture::new();
            let report = runtime.block_on(record_wav(
                &mut capture,
                &config.audio,
                Duration::from_secs_f64(seconds),
                &output,
            ))?;
            println!(
                "recorded {:.2}s from {} to {}",
                seconds,
                report.name,
                output.display()
            );
        }
        Command::Inject { text, stdout } => {
            let config = load_config(config_path.as_deref())?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let mut injector = build_injector(&config, stdout);
            runtime.block_on(injector.insert(&text))?;
        }
        Command::Install => {
            VoiceConfig::write_default(config_path.as_deref())?;
            load_config(config_path.as_deref())?;
            let manager = service_manager(config_path.as_deref())?;
            let executable = std::env::current_exe()?;
            print_service_status(manager.install(&executable)?)?;
        }
        Command::Start => {
            let config = load_config(config_path.as_deref())?;
            validate_runtime_providers(&config)?;
            let manager = service_manager(config_path.as_deref())?;
            print_service_status(manager.start()?)?;
        }
        Command::Stop => {
            let manager = service_manager(config_path.as_deref())?;
            print_service_status(manager.stop()?)?;
        }
        Command::Status => {
            let manager = service_manager(config_path.as_deref())?;
            print_service_status(manager.status()?)?;
        }
        Command::Uninstall => {
            let manager = service_manager(config_path.as_deref())?;
            print_service_status(manager.uninstall()?)?;
        }
        Command::Benchmark { input } => {
            let path = match input {
                Some(path) => path,
                None => ServicePaths::discover(config_path.as_deref())?.latency_history,
            };
            let samples = read_latency_history(&path).with_context(|| {
                format!(
                    "failed to read latency history {}; complete a dictation or pass --input",
                    path.display()
                )
            })?;
            let summary = LatencySummary::from_samples(&samples)?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        Command::Model { command } => match command {
            ModelCommand::List => {
                println!("{}", serde_json::to_string_pretty(&model_catalog())?);
            }
            ModelCommand::Install { preset, force } => {
                let paths = ServicePaths::discover(config_path.as_deref())?;
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                let installed = runtime.block_on(install_model(
                    preset.into(),
                    &paths.data_dir.join("models"),
                    force,
                ))?;
                let mut config = VoiceConfig::load(config_path.as_deref())?;
                config.asr.backend = AsrBackend::Whisper;
                config.asr.model_path = Some(installed.path.clone());
                let saved_config = config.save(config_path.as_deref())?;
                println!("{}", serde_json::to_string_pretty(&installed)?);
                println!("configured {}", saved_config.display());
            }
        },
        Command::Transcribe { input } => {
            let config = load_config(config_path.as_deref())?;
            anyhow::ensure!(
                config.asr.backend == AsrBackend::Whisper,
                "transcribe requires asr.backend = \"whisper\""
            );
            let samples = read_mono_wav(&input)?;
            let audio_ms = (samples.len() as u64 * 1_000) / ASR_SAMPLE_RATE as u64;
            let load_started = std::time::Instant::now();
            let asr = LocalWhisperAsr::new(&config.asr)?;
            let model_load_ms = elapsed_ms(load_started.elapsed());
            let inference_started = std::time::Instant::now();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let text = runtime.block_on(transcribe_samples(asr, samples, config.audio.chunk_ms))?;
            let inference_ms = elapsed_ms(inference_started.elapsed());
            let report = TranscribeReport {
                input,
                audio_ms,
                model_load_ms,
                inference_ms,
                real_time_factor: if audio_ms == 0 {
                    0.0
                } else {
                    inference_ms as f64 / audio_ms as f64
                },
                text,
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Dictate { seconds, stdout } => {
            anyhow::ensure!(
                seconds.is_finite() && seconds > 0.0,
                "seconds must be positive"
            );
            let config = load_config(config_path.as_deref())?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(dictate_once(
                config,
                Duration::from_secs_f64(seconds),
                stdout,
            ))?;
        }
        Command::Daemon { mock_text, stdout } => {
            let config = load_config(config_path.as_deref())?;
            run_daemon(config, config_path.as_deref(), mock_text, stdout)?;
        }
        Command::Control { command } => match command {
            ControlCommand::Snapshot => {
                let config = load_config(config_path.as_deref())?;
                let config_path = config_path
                    .clone()
                    .unwrap_or_else(VoiceConfig::default_path);
                let paths = ServicePaths::discover(Some(&config_path))?;
                let service = service_manager(Some(&config_path))?.status()?;
                let snapshot =
                    voice_input::build_control_snapshot(&config, &config_path, &paths, service);
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            }
            ControlCommand::Apply => {
                let mut input = String::new();
                std::io::stdin().read_to_string(&mut input)?;
                anyhow::ensure!(!input.trim().is_empty(), "settings patch JSON is empty");
                let patch: voice_input::SettingsPatch = serde_json::from_str(&input)?;
                let (_, saved) = voice_input::apply_settings_patch(config_path.as_deref(), patch)?;
                println!("{}", serde_json::json!({ "saved": saved }));
            }
            ControlCommand::TestRefiner => {
                let mut config = load_config(config_path.as_deref())?;
                anyhow::ensure!(
                    config.refiner.backend == RefinerBackend::OpenaiCompatible,
                    "LLM refinement is not enabled"
                );
                config.refiner.failure_mode = voice_input::config::RefinerFailureMode::Fail;
                let model = config.refiner.model.clone();
                let endpoint = config.refiner.base_url.clone();
                let mut refiner = OpenAiCompatibleRefiner::new(&config.refiner)?;
                let started = std::time::Instant::now();
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                let text = runtime.block_on(refiner.refine(voice_input::RefineRequest {
                    transcript: "Voice Input connection test.".to_owned(),
                    partials: Vec::new(),
                }))?;
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "endpoint": endpoint,
                        "model": model,
                        "latency_ms": elapsed_ms(started.elapsed()),
                        "response_chars": text.chars().count()
                    })
                );
            }
        },
        Command::Permission { command } => {
            let snapshot = match command {
                PermissionCommand::Snapshot => system_permission_snapshot()?,
                PermissionCommand::Request { permission } => request_permission(permission.into())?,
            };
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        }
    }
    Ok(())
}

fn read_mono_wav(path: &std::path::Path) -> anyhow::Result<Vec<i16>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("failed to open WAV {}", path.display()))?;
    let specification = reader.spec();
    anyhow::ensure!(
        specification.channels == 1,
        "ASR corpus WAV must be mono, received {} channels",
        specification.channels
    );
    anyhow::ensure!(
        specification.sample_rate == ASR_SAMPLE_RATE,
        "ASR corpus WAV must be 16 kHz, received {} Hz",
        specification.sample_rate
    );
    anyhow::ensure!(
        specification.sample_format == hound::SampleFormat::Int
            && specification.bits_per_sample == 16,
        "ASR corpus WAV must use signed 16-bit PCM"
    );
    reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

async fn transcribe_samples(
    mut asr: LocalWhisperAsr,
    samples: Vec<i16>,
    chunk_ms: u32,
) -> anyhow::Result<String> {
    let session = voice_input::SessionContext::new();
    asr.begin(&session).await?;
    let chunk_samples = (ASR_SAMPLE_RATE as usize * chunk_ms as usize) / 1_000;
    for (sequence, samples) in samples.chunks(chunk_samples.max(1)).enumerate() {
        asr.push_audio(voice_input::AudioChunk {
            session_id: session.id,
            sequence: sequence as u64,
            sample_rate: ASR_SAMPLE_RATE,
            samples: samples.to_vec(),
        })
        .await?;
    }
    Ok(asr.finish().await?.text)
}

fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

async fn dictate_once(config: VoiceConfig, duration: Duration, stdout: bool) -> anyhow::Result<()> {
    let asr = build_asr(&config, String::new())?;
    let refiner = build_refiner(&config)?;
    let injector = build_injector(&config, stdout);
    let mut engine = VoiceEngine::new(asr, refiner, injector);
    let mut capture = MacAudioCapture::new();
    let (audio_sender, mut audio_receiver) =
        tokio::sync::mpsc::channel(config.audio.channel_capacity);
    let device = capture.prepare(&config.audio, audio_sender.clone())?;
    let session = engine.begin().await?;
    capture.start(session.id, &config.audio, audio_sender)?;
    tracing::info!(
        device = %device.name,
        seconds = duration.as_secs_f64(),
        "one-shot dictation is capturing"
    );
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            chunk = audio_receiver.recv() => {
                match chunk {
                    Some(chunk) => engine.push_audio(chunk).await?,
                    None => break,
                }
            }
        }
    }
    capture.stop()?;
    while let Ok(chunk) = audio_receiver.try_recv() {
        engine.push_audio(chunk).await?;
    }
    match engine.finish().await? {
        Some(completion) => println!("{}", serde_json::to_string_pretty(&completion.latency)?),
        None => tracing::info!("no speech was detected"),
    }
    Ok(())
}

fn load_config(path: Option<&std::path::Path>) -> anyhow::Result<VoiceConfig> {
    let config = VoiceConfig::load(path)?;
    config.validate()?;
    Ok(config)
}

fn validate_runtime_providers(config: &VoiceConfig) -> anyhow::Result<()> {
    if config.asr.backend == AsrBackend::Whisper {
        let path = config.asr.model_path.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "asr.model_path is not configured; run `voice-input model install --preset balanced`"
            )
        })?;
        anyhow::ensure!(
            path.is_file(),
            "configured Whisper model does not exist at {}",
            path.display()
        );
    }
    if config.refiner.backend == RefinerBackend::OpenaiCompatible {
        OpenAiCompatibleRefiner::new(&config.refiner)?;
    }
    Ok(())
}

fn service_manager(
    config_path: Option<&std::path::Path>,
) -> anyhow::Result<PlatformServiceManager> {
    Ok(PlatformServiceManager::new(ServicePaths::discover(
        config_path,
    )?))
}

fn print_service_status(status: voice_input::ServiceStatus) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}

fn run_daemon(
    config: VoiceConfig,
    config_path: Option<&std::path::Path>,
    mock_text: String,
    stdout: bool,
) -> anyhow::Result<()> {
    if !stdout && !accessibility_is_trusted(true) {
        tracing::warn!(
            "Accessibility permission is not active; text insertion will fail until it is granted"
        );
    }

    let paths = ServicePaths::discover(config_path)?;
    let _instance_lease = InstanceLease::acquire(&paths.instance_lock)?;
    let status = FileStatusPublisher::new(&paths.runtime_status)?;
    let (hotkey_guard, hotkey_receiver) = install_hotkey(&config.hotkey)
        .with_context(|| format!("failed to register global hotkey {}", config.hotkey))?;
    let injector = build_injector(&config, stdout);
    let asr = build_asr(&config, mock_text)?;
    let refiner = build_refiner(&config)?;
    let engine = VoiceEngine::new(asr, refiner, injector);
    let runtime_config = config.clone();
    let (command_sender, command_receiver) = tokio::sync::mpsc::channel(2);
    let event_loop = MainEventLoopHandle::new();
    let signal_guard = install_shutdown_handler(command_sender.clone(), event_loop.clone())?;
    let worker_event_loop = event_loop.clone();
    let worker = std::thread::Builder::new()
        .name("voice-input-runtime".to_owned())
        .spawn(move || -> anyhow::Result<voice_input::RuntimeExit> {
            // CoreAudio stream handles are thread-affine on macOS. Construct
            // and drop the adapter on the runtime thread that owns it.
            let feedback = MacSystemFeedback::new(runtime_config.feedback.audible)?;
            let latency = FileLatencySink::new(&paths.latency_history)?;
            let voice_runtime = VoiceRuntime::new(
                runtime_config,
                MacAudioCapture::new(),
                engine,
                RuntimeEffects::new(status, feedback, latency),
            );
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let result = runtime.block_on(voice_runtime.run(hotkey_receiver, command_receiver));
            worker_event_loop.request_exit();
            result
        })?;

    println!(
        "voice-input is starting; after the ready cue, hold {} to capture and release to finish (Ctrl-C to stop)",
        config.hotkey
    );
    if stdout {
        println!("final text will be printed to stdout");
    }
    run_main_event_loop(&event_loop);
    let _ = command_sender.try_send(RuntimeCommand::Shutdown);
    let exit = worker
        .join()
        .map_err(|_| anyhow::anyhow!("voice runtime thread panicked"))??;
    tracing::info!(?exit, "voice runtime stopped");
    drop(signal_guard);
    drop(hotkey_guard);
    Ok(())
}

fn build_asr(config: &VoiceConfig, mock_text: String) -> anyhow::Result<Box<dyn StreamingAsr>> {
    if !mock_text.is_empty() {
        return Ok(Box::new(ScriptedAsr::new(mock_text)));
    }
    match config.asr.backend {
        AsrBackend::Whisper => Ok(Box::new(LocalWhisperAsr::new(&config.asr)?)),
        AsrBackend::Scripted => Ok(Box::new(ScriptedAsr::new(String::new()))),
    }
}

fn build_refiner(config: &VoiceConfig) -> anyhow::Result<Box<dyn Refiner>> {
    match config.refiner.backend {
        RefinerBackend::Identity => Ok(Box::new(IdentityRefiner)),
        RefinerBackend::OpenaiCompatible => {
            Ok(Box::new(OpenAiCompatibleRefiner::new(&config.refiner)?))
        }
    }
}

fn build_injector(config: &VoiceConfig, stdout: bool) -> Box<dyn TextInjector> {
    if stdout {
        Box::<StdoutInjector>::default()
    } else {
        Box::new(MacClipboardInjector::new(
            Duration::from_millis(config.insertion.restore_clipboard_after_ms),
            config.insertion.mode,
        ))
    }
}

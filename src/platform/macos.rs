use super::{AudioCapture, AudioDeviceReport, HotkeyEvent};
use crate::config::{AudioConfig, InsertionMode};
use crate::domain::{AudioChunk, FeedbackCue, SessionId};
use crate::ports::{Feedback, TextInjector};
use crate::runtime::RuntimeCommand;
use accessibility::{AXAttribute, AXUIElement};
use anyhow::Context;
use async_trait::async_trait;
use core_foundation::base::{CFType, TCFType};
use core_foundation::runloop::{
    kCFRunLoopCommonModes, kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopSource,
};
use core_foundation::string::CFString;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use macos_accessibility_client::accessibility::application_is_trusted;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting, NSSound,
    NSWorkspace,
};
use objc2_foundation::{NSArray, NSData, NSString};
use rubato::{
    calculate_cutoff, Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::{Handle as SignalHandle, Signals};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc as std_mpsc, Arc,
};
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct MacSystemFeedback {
    sender: Option<std_mpsc::SyncSender<FeedbackCue>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl MacSystemFeedback {
    pub fn new(enabled: bool) -> anyhow::Result<Self> {
        if !enabled {
            return Ok(Self {
                sender: None,
                worker: None,
            });
        }
        let (sender, receiver) = std_mpsc::sync_channel(8);
        let worker = thread::Builder::new()
            .name("voice-input-feedback".to_owned())
            .spawn(move || {
                while let Ok(cue) = receiver.recv() {
                    if let Err(error) = play_feedback(cue) {
                        tracing::warn!(?cue, %error, "failed to play feedback sound");
                    }
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }
}

impl Feedback for MacSystemFeedback {
    fn emit(&mut self, cue: FeedbackCue) -> anyhow::Result<()> {
        let Some(sender) = &self.sender else {
            return Ok(());
        };
        match sender.try_send(cue) {
            Ok(()) => Ok(()),
            Err(std_mpsc::TrySendError::Full(_)) => {
                tracing::debug!(?cue, "feedback queue is full; dropping cue");
                Ok(())
            }
            Err(std_mpsc::TrySendError::Disconnected(_)) => {
                anyhow::bail!("feedback worker has stopped")
            }
        }
    }
}

impl Drop for MacSystemFeedback {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                tracing::warn!("feedback worker panicked");
            }
        }
    }
}

fn play_feedback(cue: FeedbackCue) -> anyhow::Result<()> {
    let name = match cue {
        FeedbackCue::Ready | FeedbackCue::Stopped => return Ok(()),
        FeedbackCue::CaptureStarted => "Tink",
        FeedbackCue::Finalizing => "Pop",
        FeedbackCue::Completed => "Glass",
        FeedbackCue::Error => "Basso",
    };
    let sound = NSSound::soundNamed(&NSString::from_str(name))
        .ok_or_else(|| anyhow::anyhow!("macOS sound {name} is unavailable"))?;
    anyhow::ensure!(sound.play(), "macOS rejected sound playback for {name}");
    Ok(())
}

pub struct MacAudioCapture {
    stream: Option<cpal::Stream>,
    normalizer_sender: Option<std_mpsc::SyncSender<NativeAudioMessage>>,
    normalizer_thread: Option<thread::JoinHandle<anyhow::Result<()>>>,
    active: Arc<AtomicBool>,
    dropped_native_packets: Arc<AtomicU64>,
    report: Option<AudioDeviceReport>,
}

impl MacAudioCapture {
    pub fn new() -> Self {
        Self {
            stream: None,
            normalizer_sender: None,
            normalizer_thread: None,
            active: Arc::new(AtomicBool::new(false)),
            dropped_native_packets: Arc::new(AtomicU64::new(0)),
            report: None,
        }
    }
}

impl Default for MacAudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioCapture for MacAudioCapture {
    fn prepare(
        &mut self,
        config: &AudioConfig,
        sender: mpsc::Sender<AudioChunk>,
    ) -> anyhow::Result<AudioDeviceReport> {
        if let Some(report) = &self.report {
            return Ok(report.clone());
        }
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("no default microphone is available"))?;
        let (supported, native_target_rate) = choose_input_config(&device, config.sample_rate)?;
        let channels = supported.channels();
        let sample_format = supported.sample_format();
        let stream_config: cpal::StreamConfig = supported.clone().into();
        let output_chunk_samples =
            ((config.sample_rate as u64 * config.chunk_ms as u64) / 1_000).max(1) as usize;
        let (normalizer_sender, normalizer_receiver) = std_mpsc::sync_channel(16);
        let source_rate = stream_config.sample_rate.0;
        let target_rate = config.sample_rate;
        let normalizer_thread = thread::Builder::new()
            .name("voice-input-resampler".to_owned())
            .spawn(move || {
                normalizer_loop(
                    source_rate,
                    target_rate,
                    output_chunk_samples,
                    normalizer_receiver,
                    sender,
                )
            })?;
        let active = Arc::clone(&self.active);
        let dropped_native_packets = Arc::clone(&self.dropped_native_packets);

        let stream = match sample_format {
            SampleFormat::I8 => build_native_input_stream::<i8>(
                &device,
                &stream_config,
                channels,
                normalizer_sender.clone(),
                Arc::clone(&active),
                Arc::clone(&dropped_native_packets),
            )?,
            SampleFormat::I16 => build_native_input_stream::<i16>(
                &device,
                &stream_config,
                channels,
                normalizer_sender.clone(),
                Arc::clone(&active),
                Arc::clone(&dropped_native_packets),
            )?,
            SampleFormat::I32 => build_native_input_stream::<i32>(
                &device,
                &stream_config,
                channels,
                normalizer_sender.clone(),
                Arc::clone(&active),
                Arc::clone(&dropped_native_packets),
            )?,
            SampleFormat::I64 => build_native_input_stream::<i64>(
                &device,
                &stream_config,
                channels,
                normalizer_sender.clone(),
                Arc::clone(&active),
                Arc::clone(&dropped_native_packets),
            )?,
            SampleFormat::U8 => build_native_input_stream::<u8>(
                &device,
                &stream_config,
                channels,
                normalizer_sender.clone(),
                Arc::clone(&active),
                Arc::clone(&dropped_native_packets),
            )?,
            SampleFormat::U16 => build_native_input_stream::<u16>(
                &device,
                &stream_config,
                channels,
                normalizer_sender.clone(),
                Arc::clone(&active),
                Arc::clone(&dropped_native_packets),
            )?,
            SampleFormat::U32 => build_native_input_stream::<u32>(
                &device,
                &stream_config,
                channels,
                normalizer_sender.clone(),
                Arc::clone(&active),
                Arc::clone(&dropped_native_packets),
            )?,
            SampleFormat::U64 => build_native_input_stream::<u64>(
                &device,
                &stream_config,
                channels,
                normalizer_sender.clone(),
                Arc::clone(&active),
                Arc::clone(&dropped_native_packets),
            )?,
            SampleFormat::F32 => build_native_input_stream::<f32>(
                &device,
                &stream_config,
                channels,
                normalizer_sender.clone(),
                Arc::clone(&active),
                Arc::clone(&dropped_native_packets),
            )?,
            SampleFormat::F64 => build_native_input_stream::<f64>(
                &device,
                &stream_config,
                channels,
                normalizer_sender.clone(),
                active,
                dropped_native_packets,
            )?,
            format => anyhow::bail!("unsupported microphone sample format: {format}"),
        };
        stream.play()?;

        let report = AudioDeviceReport {
            name: device.name()?,
            input_sample_rate: stream_config.sample_rate.0,
            output_sample_rate: config.sample_rate,
            channels,
            sample_format: sample_format.to_string(),
            native_16khz: native_target_rate,
        };
        self.stream = Some(stream);
        self.normalizer_sender = Some(normalizer_sender);
        self.normalizer_thread = Some(normalizer_thread);
        self.report = Some(report.clone());
        Ok(report)
    }

    fn start(
        &mut self,
        session_id: SessionId,
        config: &AudioConfig,
        sender: mpsc::Sender<AudioChunk>,
    ) -> anyhow::Result<AudioDeviceReport> {
        let report = self.prepare(config, sender)?;
        anyhow::ensure!(
            !self.active.load(Ordering::Acquire),
            "audio capture is already active"
        );
        self.normalizer_sender
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("audio normalizer is not ready"))?
            .send(NativeAudioMessage::Start(session_id))?;
        self.active.store(true, Ordering::Release);
        Ok(report)
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        self.active.store(false, Ordering::Release);
        let (ack_sender, ack_receiver) = std_mpsc::sync_channel(1);
        self.normalizer_sender
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("audio normalizer is not ready"))?
            .send(NativeAudioMessage::Stop(ack_sender))?;
        ack_receiver.recv_timeout(Duration::from_secs(2))?;
        let dropped = self.dropped_native_packets.swap(0, Ordering::AcqRel);
        if dropped > 0 {
            tracing::warn!(dropped, "native audio packets were dropped during capture");
        }
        Ok(())
    }
}

impl Drop for MacAudioCapture {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        self.stream.take();
        if let Some(sender) = self.normalizer_sender.take() {
            let _ = sender.send(NativeAudioMessage::Shutdown);
        }
        if let Some(worker) = self.normalizer_thread.take() {
            match worker.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(%error, "audio normalizer stopped with an error"),
                Err(_) => tracing::warn!("audio normalizer thread panicked"),
            }
        }
    }
}

fn choose_input_config(
    device: &cpal::Device,
    sample_rate: u32,
) -> anyhow::Result<(cpal::SupportedStreamConfig, bool)> {
    let mut candidates = device
        .supported_input_configs()?
        .filter(|candidate| {
            candidate.min_sample_rate().0 <= sample_rate
                && candidate.max_sample_rate().0 >= sample_rate
        })
        .collect::<Vec<_>>();

    candidates.sort_by_key(|candidate| {
        let mono_penalty = u8::from(candidate.channels() != 1);
        let format_penalty = u8::from(candidate.sample_format() != SampleFormat::F32);
        (mono_penalty, format_penalty, candidate.channels())
    });
    if let Some(candidate) = candidates.into_iter().next() {
        return Ok((
            candidate.with_sample_rate(cpal::SampleRate(sample_rate)),
            true,
        ));
    }
    Ok((device.default_input_config()?, false))
}

#[derive(Debug)]
enum NativeAudioMessage {
    Start(SessionId),
    Samples(Vec<f32>),
    Stop(std_mpsc::SyncSender<()>),
    Shutdown,
}

fn build_native_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: u16,
    sender: std_mpsc::SyncSender<NativeAudioMessage>,
    active: Arc<AtomicBool>,
    dropped_native_packets: Arc<AtomicU64>,
) -> anyhow::Result<cpal::Stream>
where
    T: Sample + SizedSample + Copy + Send + 'static,
    f32: FromSample<T>,
{
    let error_callback = |error| tracing::error!(%error, "microphone stream failed");

    let stream = device.build_input_stream(
        config,
        move |input: &[T], _| {
            if !active.load(Ordering::Acquire) {
                return;
            }
            let mut samples = Vec::with_capacity(input.len() / channels as usize);
            for frame in input.chunks(channels as usize) {
                if frame.is_empty() {
                    continue;
                }
                let mono =
                    frame.iter().copied().map(f32::from_sample).sum::<f32>() / frame.len() as f32;
                samples.push(mono);
            }
            if !samples.is_empty()
                && sender
                    .try_send(NativeAudioMessage::Samples(samples))
                    .is_err()
            {
                dropped_native_packets.fetch_add(1, Ordering::Relaxed);
            }
        },
        error_callback,
        None,
    )?;
    Ok(stream)
}

fn normalizer_loop(
    source_rate: u32,
    target_rate: u32,
    output_chunk_samples: usize,
    receiver: std_mpsc::Receiver<NativeAudioMessage>,
    sender: mpsc::Sender<AudioChunk>,
) -> anyhow::Result<()> {
    while let Ok(message) = receiver.recv() {
        match message {
            NativeAudioMessage::Start(session_id) => normalize_audio(
                session_id,
                source_rate,
                target_rate,
                output_chunk_samples,
                &receiver,
                &sender,
            )?,
            NativeAudioMessage::Shutdown => break,
            NativeAudioMessage::Samples(_) | NativeAudioMessage::Stop(_) => {
                tracing::warn!("audio normalizer received data without an active session");
            }
        }
    }
    Ok(())
}

fn normalize_audio(
    session_id: SessionId,
    source_rate: u32,
    target_rate: u32,
    output_chunk_samples: usize,
    receiver: &std_mpsc::Receiver<NativeAudioMessage>,
    sender: &mpsc::Sender<AudioChunk>,
) -> anyhow::Result<()> {
    let sequence = AtomicU64::new(0);
    let mut output = Vec::with_capacity(output_chunk_samples * 2);
    let mut stop_ack = None;

    if source_rate == target_rate {
        while let Ok(message) = receiver.recv() {
            match message {
                NativeAudioMessage::Samples(samples) => {
                    output.extend(samples.into_iter().map(float_to_i16));
                    emit_normalized_chunks(
                        session_id,
                        target_rate,
                        output_chunk_samples,
                        &sequence,
                        &mut output,
                        sender,
                    );
                }
                NativeAudioMessage::Stop(ack) => {
                    stop_ack = Some(ack);
                    break;
                }
                NativeAudioMessage::Start(_) | NativeAudioMessage::Shutdown => {
                    anyhow::bail!("audio session control messages arrived out of order")
                }
            }
        }
    } else {
        let sinc_len = 128;
        let window = WindowFunction::Blackman2;
        let parameters = SincInterpolationParameters {
            sinc_len,
            f_cutoff: calculate_cutoff(sinc_len, window),
            interpolation: SincInterpolationType::Quadratic,
            oversampling_factor: 256,
            window,
        };
        let input_frames = 1_024;
        let mut resampler = SincFixedIn::<f32>::new(
            target_rate as f64 / source_rate as f64,
            1.0,
            parameters,
            input_frames,
            1,
        )?;
        let mut delay_remaining = resampler.output_delay();
        let mut input = Vec::with_capacity(input_frames * 2);
        let mut total_input_samples = 0usize;
        let mut emitted_samples = 0usize;

        while let Ok(message) = receiver.recv() {
            match message {
                NativeAudioMessage::Samples(samples) => {
                    total_input_samples += samples.len();
                    input.extend_from_slice(&samples);
                    while input.len() >= input_frames {
                        let remainder = input.split_off(input_frames);
                        let block = std::mem::replace(&mut input, remainder);
                        let resampled = resampler.process(&[block], None)?;
                        append_resampled(&resampled[0], &mut delay_remaining, &mut output);
                        emitted_samples += emit_normalized_chunks(
                            session_id,
                            target_rate,
                            output_chunk_samples,
                            &sequence,
                            &mut output,
                            sender,
                        );
                    }
                }
                NativeAudioMessage::Stop(ack) => {
                    let resampled = if input.is_empty() {
                        resampler.process_partial::<Vec<f32>>(None, None)?
                    } else {
                        resampler.process_partial(Some(&[input]), None)?
                    };
                    append_resampled(&resampled[0], &mut delay_remaining, &mut output);

                    let expected_samples = ((total_input_samples as u128 * target_rate as u128)
                        / source_rate as u128) as usize;
                    for _ in 0..4 {
                        if emitted_samples + output.len() >= expected_samples {
                            break;
                        }
                        let delayed = resampler.process_partial::<Vec<f32>>(None, None)?;
                        append_resampled(&delayed[0], &mut delay_remaining, &mut output);
                    }
                    output.truncate(expected_samples.saturating_sub(emitted_samples));
                    stop_ack = Some(ack);
                    break;
                }
                NativeAudioMessage::Start(_) | NativeAudioMessage::Shutdown => {
                    anyhow::bail!("audio session control messages arrived out of order")
                }
            }
        }
    }

    if !output.is_empty() {
        emit_chunk(
            session_id,
            target_rate,
            sequence.fetch_add(1, Ordering::Relaxed),
            output,
            sender,
        );
    }
    if let Some(ack) = stop_ack {
        let _ = ack.send(());
    }
    Ok(())
}

fn append_resampled(samples: &[f32], delay_remaining: &mut usize, output: &mut Vec<i16>) {
    let skip = (*delay_remaining).min(samples.len());
    *delay_remaining -= skip;
    output.extend(samples[skip..].iter().copied().map(float_to_i16));
}

fn emit_normalized_chunks(
    session_id: SessionId,
    sample_rate: u32,
    chunk_samples: usize,
    sequence: &AtomicU64,
    output: &mut Vec<i16>,
    sender: &mpsc::Sender<AudioChunk>,
) -> usize {
    let mut emitted = 0;
    while output.len() >= chunk_samples {
        let remainder = output.split_off(chunk_samples);
        let samples = std::mem::replace(output, remainder);
        emit_chunk(
            session_id,
            sample_rate,
            sequence.fetch_add(1, Ordering::Relaxed),
            samples,
            sender,
        );
        emitted += chunk_samples;
    }
    emitted
}

fn emit_chunk(
    session_id: SessionId,
    sample_rate: u32,
    sequence: u64,
    samples: Vec<i16>,
    sender: &mpsc::Sender<AudioChunk>,
) {
    let chunk = AudioChunk {
        session_id,
        sequence,
        sample_rate,
        samples,
    };
    if let Err(error) = sender.try_send(chunk) {
        tracing::warn!(%error, "normalized audio queue is full; dropping one chunk");
    }
}

fn float_to_i16(sample: f32) -> i16 {
    let scaled = sample.clamp(-1.0, 1.0) * i16::MAX as f32;
    scaled.round() as i16
}

pub fn probe_audio_device(sample_rate: u32) -> anyhow::Result<AudioDeviceReport> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("no default microphone is available"))?;
    let default = device.default_input_config()?;
    let native_16khz = choose_input_config(&device, sample_rate)
        .map(|(_, native)| native)
        .unwrap_or(false);
    Ok(AudioDeviceReport {
        name: device.name()?,
        input_sample_rate: default.sample_rate().0,
        output_sample_rate: sample_rate,
        channels: default.channels(),
        sample_format: default.sample_format().to_string(),
        native_16khz,
    })
}

pub fn accessibility_is_trusted(prompt: bool) -> bool {
    if prompt {
        macos_accessibility_client::accessibility::application_is_trusted_with_prompt()
    } else {
        application_is_trusted()
    }
}

pub enum MacHotkeyGuard {
    Global(GlobalHotkeyGuard),
    Function(FunctionHotkeyGuard),
}

pub struct GlobalHotkeyGuard {
    manager: GlobalHotKeyManager,
    hotkey: HotKey,
    running: Arc<AtomicBool>,
}

impl Drop for GlobalHotkeyGuard {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Err(error) = self.manager.unregister(self.hotkey) {
            tracing::warn!(%error, "failed to unregister global hotkey");
        }
    }
}

pub struct FunctionHotkeyGuard {
    _tap: CGEventTap<'static>,
    source: CFRunLoopSource,
    run_loop: CFRunLoop,
}

impl Drop for FunctionHotkeyGuard {
    fn drop(&mut self) {
        self.run_loop
            .remove_source(&self.source, unsafe { kCFRunLoopCommonModes });
    }
}

pub fn install_hotkey(
    specification: &str,
) -> anyhow::Result<(MacHotkeyGuard, mpsc::Receiver<HotkeyEvent>)> {
    if specification.eq_ignore_ascii_case("fn") || specification.eq_ignore_ascii_case("function") {
        return install_function_hotkey();
    }
    let hotkey: HotKey = specification.parse()?;
    let manager = GlobalHotKeyManager::new()?;
    manager.register(hotkey)?;
    let (sender, receiver) = mpsc::channel(16);
    let running = Arc::new(AtomicBool::new(true));
    let thread_running = Arc::clone(&running);

    thread::Builder::new()
        .name("voice-input-hotkey".to_owned())
        .spawn(move || {
            while thread_running.load(Ordering::Acquire) {
                let event =
                    match GlobalHotKeyEvent::receiver().recv_timeout(Duration::from_millis(100)) {
                        Ok(event) => event,
                        Err(_) => continue,
                    };
                if event.id != hotkey.id() {
                    continue;
                }
                let mapped = match event.state {
                    HotKeyState::Pressed => HotkeyEvent::Pressed,
                    HotKeyState::Released => HotkeyEvent::Released,
                };
                if sender.blocking_send(mapped).is_err() {
                    break;
                }
            }
        })?;

    Ok((
        MacHotkeyGuard::Global(GlobalHotkeyGuard {
            manager,
            hotkey,
            running,
        }),
        receiver,
    ))
}

fn install_function_hotkey() -> anyhow::Result<(MacHotkeyGuard, mpsc::Receiver<HotkeyEvent>)> {
    let (sender, receiver) = mpsc::channel(16);
    let pressed = Arc::new(AtomicBool::new(false));
    let callback_pressed = Arc::clone(&pressed);
    let tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::ListenOnly,
        vec![CGEventType::FlagsChanged],
        move |_proxy, _event_type, event| {
            let is_pressed = event
                .get_flags()
                .contains(CGEventFlags::CGEventFlagSecondaryFn);
            let was_pressed = callback_pressed.swap(is_pressed, Ordering::AcqRel);
            if is_pressed != was_pressed {
                let mapped = if is_pressed {
                    HotkeyEvent::Pressed
                } else {
                    HotkeyEvent::Released
                };
                if let Err(error) = sender.try_send(mapped) {
                    tracing::warn!(%error, "Fn hotkey queue is full; dropping transition");
                }
            }
            None
        },
    )
    .map_err(|_| {
        anyhow::anyhow!("failed to create the Fn event tap; grant Accessibility permission")
    })?;
    let source = tap
        .mach_port
        .create_runloop_source(0)
        .map_err(|_| anyhow::anyhow!("failed to create the Fn event-tap run-loop source"))?;
    let run_loop = CFRunLoop::get_main();
    run_loop.add_source(&source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    Ok((
        MacHotkeyGuard::Function(FunctionHotkeyGuard {
            _tap: tap,
            source,
            run_loop,
        }),
        receiver,
    ))
}

#[derive(Clone)]
pub struct MainEventLoopHandle {
    exit_requested: Arc<AtomicBool>,
}

impl MainEventLoopHandle {
    pub fn new() -> Self {
        Self {
            exit_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn request_exit(&self) {
        self.exit_requested.store(true, Ordering::Release);
    }
}

impl Default for MainEventLoopHandle {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run_main_event_loop(handle: &MainEventLoopHandle) {
    // The global shortcut adapter is Carbon-backed while the Fn adapter owns a
    // Core Foundation run-loop source. Pump both with finite timeouts so every
    // hotkey implementation shares the same explicit shutdown boundary.
    while !handle.exit_requested.load(Ordering::Acquire) {
        // SAFETY: This is the process main thread. A finite EventTimeout has no
        // borrowed inputs and Carbon owns all event-loop state.
        unsafe {
            RunCurrentEventLoop(0.01);
        }
        CFRunLoop::run_in_mode(
            unsafe { kCFRunLoopDefaultMode },
            Duration::from_millis(40),
            true,
        );
    }
}

pub struct ShutdownSignalGuard {
    handle: SignalHandle,
    worker: Option<thread::JoinHandle<()>>,
}

impl Drop for ShutdownSignalGuard {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                tracing::warn!("shutdown signal thread panicked");
            }
        }
    }
}

pub fn install_shutdown_handler(
    sender: mpsc::Sender<RuntimeCommand>,
    event_loop: MainEventLoopHandle,
) -> anyhow::Result<ShutdownSignalGuard> {
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    let handle = signals.handle();
    let worker = thread::Builder::new()
        .name("voice-input-signals".to_owned())
        .spawn(move || {
            if signals.forever().next().is_some() {
                let _ = sender.try_send(RuntimeCommand::Shutdown);
                event_loop.request_exit();
            }
        })?;
    Ok(ShutdownSignalGuard {
        handle,
        worker: Some(worker),
    })
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn RunCurrentEventLoop(timeout: f64) -> i32;
}

pub struct MacClipboardInjector {
    restore_after: Duration,
    mode: InsertionMode,
}

impl MacClipboardInjector {
    pub fn new(restore_after: Duration, mode: InsertionMode) -> Self {
        Self {
            restore_after,
            mode,
        }
    }
}

#[async_trait]
impl TextInjector for MacClipboardInjector {
    async fn insert(&mut self, text: &str) -> anyhow::Result<()> {
        let text = text.to_owned();
        let restore_after = self.restore_after;
        let mode = self.mode;
        tokio::task::spawn_blocking(move || insert_text(&text, restore_after, mode)).await??;
        Ok(())
    }
}

fn insert_text(text: &str, restore_after: Duration, mode: InsertionMode) -> anyhow::Result<()> {
    match mode {
        InsertionMode::Accessibility => insert_with_accessibility(text),
        InsertionMode::Clipboard => paste_preserving_clipboard(text, restore_after),
        InsertionMode::Auto => match insert_with_accessibility(text) {
            Ok(()) => {
                tracing::debug!(method = "accessibility", "text inserted");
                Ok(())
            }
            Err(error) => {
                tracing::debug!(%error, "direct Accessibility insertion unavailable; using pasteboard");
                paste_preserving_clipboard(text, restore_after)
            }
        },
    }
}

fn insert_with_accessibility(text: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        application_is_trusted(),
        "Accessibility permission is not active"
    );
    let frontmost = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .ok_or_else(|| anyhow::anyhow!("macOS has no frontmost application"))?;
    let application = AXUIElement::application(frontmost.processIdentifier());
    application
        .set_messaging_timeout(0.75)
        .context("failed to configure the frontmost application Accessibility timeout")?;
    let focused_attribute = AXAttribute::<CFType>::new(&CFString::new("AXFocusedUIElement"));
    let focused = application
        .attribute(&focused_attribute)
        .context("failed to query the focused Accessibility element")?
        .downcast_into::<AXUIElement>()
        .ok_or_else(|| anyhow::anyhow!("focused Accessibility element has an unexpected type"))?;
    focused
        .set_messaging_timeout(0.75)
        .context("failed to configure the focused Accessibility element timeout")?;
    let selected_text_attribute = AXAttribute::<CFType>::new(&CFString::new("AXSelectedText"));
    anyhow::ensure!(
        focused
            .is_settable(&selected_text_attribute)
            .context("failed to inspect AXSelectedText writability")?,
        "focused element does not support writable AXSelectedText"
    );
    focused
        .set_attribute(&selected_text_attribute, CFString::new(text).into_CFType())
        .context("failed to set AXSelectedText")?;
    tracing::debug!(method = "accessibility", "text inserted");
    Ok(())
}

fn paste_preserving_clipboard(text: &str, restore_after: Duration) -> anyhow::Result<()> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let snapshot = PasteboardSnapshot::capture(&pasteboard)?;
    write_text_to_pasteboard(&pasteboard, text)?;
    let injected_change_count = pasteboard.changeCount();
    if let Err(error) = post_command_v() {
        snapshot.restore(&pasteboard)?;
        return Err(error);
    }
    thread::sleep(restore_after);
    if pasteboard.changeCount() == injected_change_count {
        snapshot.restore(&pasteboard)?;
    } else {
        tracing::info!(
            "pasteboard changed after insertion; preserving the newer user/application contents"
        );
    }
    tracing::debug!(method = "pasteboard", "text inserted");
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PasteboardSnapshot {
    items: Vec<Vec<PasteboardEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PasteboardEntry {
    type_name: String,
    bytes: Vec<u8>,
}

impl PasteboardSnapshot {
    fn capture(pasteboard: &NSPasteboard) -> anyhow::Result<Self> {
        let mut snapshot = Self { items: Vec::new() };
        let Some(items) = pasteboard.pasteboardItems() else {
            return Ok(snapshot);
        };
        for item in items.to_vec() {
            let mut entries = Vec::new();
            for type_name in item.types().to_vec() {
                let data = item.dataForType(&type_name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "pasteboard item type {} could not be materialized",
                        type_name
                    )
                })?;
                entries.push(PasteboardEntry {
                    type_name: type_name.to_string(),
                    bytes: data.to_vec(),
                });
            }
            snapshot.items.push(entries);
        }
        Ok(snapshot)
    }

    fn restore(&self, pasteboard: &NSPasteboard) -> anyhow::Result<()> {
        pasteboard.clearContents();
        if self.items.is_empty() {
            return Ok(());
        }
        let mut items = Vec::with_capacity(self.items.len());
        for entries in &self.items {
            let item = NSPasteboardItem::new();
            for entry in entries {
                let type_name = NSString::from_str(&entry.type_name);
                let data = NSData::with_bytes(&entry.bytes);
                anyhow::ensure!(
                    item.setData_forType(&data, &type_name),
                    "failed to restore pasteboard type {}",
                    entry.type_name
                );
            }
            let item: Retained<ProtocolObject<dyn NSPasteboardWriting>> =
                ProtocolObject::from_retained(item);
            items.push(item);
        }
        let objects = NSArray::from_retained_slice(&items);
        anyhow::ensure!(
            pasteboard.writeObjects(&objects),
            "failed to restore pasteboard items"
        );
        Ok(())
    }
}

fn write_text_to_pasteboard(pasteboard: &NSPasteboard, text: &str) -> anyhow::Result<()> {
    pasteboard.clearContents();
    let item = NSPasteboardItem::new();
    // SAFETY: AppKit exports NSPasteboardTypeString as an immutable process-
    // lifetime NSString constant.
    let text_type = unsafe { NSPasteboardTypeString };
    anyhow::ensure!(
        item.setString_forType(&NSString::from_str(text), text_type),
        "failed to write text to pasteboard item"
    );
    let item: Retained<ProtocolObject<dyn NSPasteboardWriting>> =
        ProtocolObject::from_retained(item);
    let items = NSArray::from_retained_slice(&[item]);
    anyhow::ensure!(
        pasteboard.writeObjects(&items),
        "failed to write text to pasteboard"
    );
    Ok(())
}

fn post_command_v() -> anyhow::Result<()> {
    const VIRTUAL_KEY_V: u16 = 0x09;
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| anyhow::anyhow!("failed to create a CoreGraphics event source"))?;
    let down = CGEvent::new_keyboard_event(source.clone(), VIRTUAL_KEY_V, true)
        .map_err(|_| anyhow::anyhow!("failed to create Command-V key-down event"))?;
    down.set_flags(CGEventFlags::CGEventFlagCommand);
    down.post(CGEventTapLocation::HID);

    let up = CGEvent::new_keyboard_event(source, VIRTUAL_KEY_V, false)
        .map_err(|_| anyhow::anyhow!("failed to create Command-V key-up event"))?;
    up.set_flags(CGEventFlags::CGEventFlagCommand);
    up.post(CGEventTapLocation::HID);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{float_to_i16, normalizer_loop, NativeAudioMessage, PasteboardSnapshot};
    use crate::domain::SessionId;
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSData, NSString};

    #[test]
    fn float_pcm_conversion_clamps() {
        assert_eq!(float_to_i16(2.0), i16::MAX);
        assert_eq!(float_to_i16(-2.0), -i16::MAX);
        assert_eq!(float_to_i16(0.0), 0);
    }

    #[test]
    fn resampler_preserves_one_second_duration() {
        let (native_sender, native_receiver) = std::sync::mpsc::sync_channel(128);
        let (output_sender, mut output_receiver) = tokio::sync::mpsc::channel(64);
        let session_id = SessionId::new();
        let worker = std::thread::spawn(move || {
            normalizer_loop(48_000, 16_000, 1_280, native_receiver, output_sender)
        });

        native_sender
            .send(NativeAudioMessage::Start(session_id))
            .unwrap();
        let mut remaining = 48_000;
        while remaining > 0 {
            let frames = remaining.min(512);
            native_sender
                .send(NativeAudioMessage::Samples(vec![0.0; frames]))
                .unwrap();
            remaining -= frames;
        }
        let (ack_sender, ack_receiver) = std::sync::mpsc::sync_channel(1);
        native_sender
            .send(NativeAudioMessage::Stop(ack_sender))
            .unwrap();
        ack_receiver.recv().unwrap();
        native_sender.send(NativeAudioMessage::Shutdown).unwrap();
        worker.join().unwrap().unwrap();

        let mut samples = 0;
        let mut expected_sequence = 0;
        while let Ok(chunk) = output_receiver.try_recv() {
            assert_eq!(chunk.sample_rate, 16_000);
            assert_eq!(chunk.sequence, expected_sequence);
            samples += chunk.samples.len();
            expected_sequence += 1;
        }
        assert_eq!(samples, 16_000);
    }

    #[test]
    fn pasteboard_snapshot_round_trips_multiple_items_and_types() {
        let pasteboard = NSPasteboard::pasteboardWithUniqueName();
        let first = NSPasteboardItem::new();
        let plain = NSString::from_str("public.utf8-plain-text");
        let html = NSString::from_str("public.html");
        assert!(first.setData_forType(&NSData::with_bytes(b"hello"), &plain));
        assert!(first.setData_forType(&NSData::with_bytes(b"<b>hello</b>"), &html));
        let second = NSPasteboardItem::new();
        let custom = NSString::from_str("com.lifcc.voiceinput.test");
        assert!(second.setData_forType(&NSData::with_bytes(&[0, 1, 2, 255]), &custom));
        let objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = vec![
            ProtocolObject::from_retained(first),
            ProtocolObject::from_retained(second),
        ];
        assert!(pasteboard.writeObjects(&NSArray::from_retained_slice(&objects)));
        let expected = PasteboardSnapshot::capture(&pasteboard).unwrap();

        pasteboard.clearContents();
        expected.restore(&pasteboard).unwrap();

        assert_eq!(PasteboardSnapshot::capture(&pasteboard).unwrap(), expected);
    }
}

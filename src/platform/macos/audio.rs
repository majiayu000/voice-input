use crate::config::AudioConfig;
use crate::domain::{AudioChunk, SessionId};
use crate::platform::{AudioCapture, AudioDeviceReport};
use anyhow::Context;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use rubato::{
    calculate_cutoff, Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc as std_mpsc, Arc,
};
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc;

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

    fn open_stream(
        &mut self,
        config: &AudioConfig,
        sender: mpsc::Sender<AudioChunk>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.stream.is_none()
                && self.normalizer_sender.is_none()
                && self.normalizer_thread.is_none(),
            "microphone stream resources are already allocated"
        );
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("no default microphone is available"))?;
        let (supported, _native_target_rate) = choose_input_config(&device, config.sample_rate)?;
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

        self.stream = Some(stream);
        self.normalizer_sender = Some(normalizer_sender);
        self.normalizer_thread = Some(normalizer_thread);
        Ok(())
    }

    fn release_stream(&mut self) -> anyhow::Result<()> {
        self.active.store(false, Ordering::Release);
        self.stream.take();
        if let Some(sender) = self.normalizer_sender.take() {
            sender.send(NativeAudioMessage::Shutdown)?;
        }
        if let Some(worker) = self.normalizer_thread.take() {
            return match worker.join() {
                Ok(result) => result,
                Err(_) => anyhow::bail!("audio normalizer thread panicked"),
            };
        }
        Ok(())
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
        _sender: mpsc::Sender<AudioChunk>,
    ) -> anyhow::Result<AudioDeviceReport> {
        if let Some(report) = &self.report {
            return Ok(report.clone());
        }
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("no default microphone is available"))?;
        let (supported, native_target_rate) = choose_input_config(&device, config.sample_rate)?;
        let report = AudioDeviceReport {
            name: device.name()?,
            input_sample_rate: supported.sample_rate().0,
            output_sample_rate: config.sample_rate,
            channels: supported.channels(),
            sample_format: supported.sample_format().to_string(),
            native_16khz: native_target_rate,
        };
        self.report = Some(report.clone());
        Ok(report)
    }

    fn start(
        &mut self,
        session_id: SessionId,
        config: &AudioConfig,
        sender: mpsc::Sender<AudioChunk>,
    ) -> anyhow::Result<AudioDeviceReport> {
        anyhow::ensure!(
            !self.active.load(Ordering::Acquire),
            "audio capture is already active"
        );
        let report = self.prepare(config, sender.clone())?;
        self.open_stream(config, sender)?;
        if let Err(error) = self
            .normalizer_sender
            .as_ref()
            .context("audio normalizer is not ready")?
            .send(NativeAudioMessage::Start(session_id))
        {
            let cleanup = self.release_stream();
            return match cleanup {
                Ok(()) => Err(error.into()),
                Err(cleanup) => Err(anyhow::anyhow!(
                    "failed to start audio normalizer: {error}; cleanup also failed: {cleanup}"
                )),
            };
        }
        self.active.store(true, Ordering::Release);
        if let Err(error) = self
            .stream
            .as_ref()
            .context("microphone stream is not prepared")?
            .play()
        {
            let cleanup = self.stop();
            return match cleanup {
                Ok(()) => Err(error.into()),
                Err(cleanup) => Err(anyhow::anyhow!(
                    "failed to activate microphone stream: {error}; cleanup also failed: {cleanup}"
                )),
            };
        }
        Ok(report)
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        if self.normalizer_sender.is_none() {
            return Ok(());
        }
        self.active.store(false, Ordering::Release);
        let session_result = (|| -> anyhow::Result<()> {
            self.stream
                .as_ref()
                .context("microphone stream is not prepared")?
                .pause()?;
            let (ack_sender, ack_receiver) = std_mpsc::sync_channel(1);
            self.normalizer_sender
                .as_ref()
                .context("audio normalizer is not ready")?
                .send(NativeAudioMessage::Stop(ack_sender))?;
            ack_receiver.recv_timeout(Duration::from_secs(2))?;
            Ok(())
        })();
        let release_result = self.release_stream();
        let dropped = self.dropped_native_packets.swap(0, Ordering::AcqRel);
        if dropped > 0 {
            tracing::warn!(dropped, "native audio packets were dropped during capture");
        }
        match (session_result, release_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(release)) => Err(anyhow::anyhow!(
                "{error}; stream cleanup also failed: {release}"
            )),
        }
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
pub(super) enum NativeAudioMessage {
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

pub(super) fn normalizer_loop(
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

pub(super) fn float_to_i16(sample: f32) -> i16 {
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

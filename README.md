# Voice Input

`voice-input` is a local, low-latency voice-input runtime for macOS. It embeds
whisper.cpp through Rust, keeps the selected model and inference state warm on
a dedicated worker, and exposes an optional OpenAI-compatible refinement stage.

`Voice Input` is a neutral engineering name, not a finalized product brand.
Product-facing identifiers are isolated in the CLI/service composition layer so
they can be replaced without coupling the ASR, audio, or refinement core.

## Implemented foundation

- press-and-hold global hotkey or native Fn event tap on macOS;
- 16 kHz mono microphone capture in bounded 80 ms chunks;
- non-blocking audio callback with explicit backpressure;
- strict `Idle -> Capturing -> Finalizing -> Inserting` state machine;
- streaming ASR, refiner, and text-injector ports;
- in-process Whisper/Metal inference with a reusable model and state;
- lightweight RMS VAD and bounded in-memory utterance buffering;
- integrity-pinned, resumable model installation;
- optional local or explicitly-authorized remote LLM refinement;
- direct Accessibility insertion with an automatic Command-V fallback;
- complete multi-item, multi-type pasteboard snapshot and guarded restoration;
- non-blocking audible lifecycle feedback;
- per-session latency reports, JSONL history, and p50/p95 summaries;
- atomic runtime status and an exclusive single-instance lease;
- graceful SIGINT/SIGTERM shutdown with capture and ASR cancellation;
- signed per-user app container, LaunchAgent installation, and structured
  service status;
- microphone and permission diagnostics;
- WAV recording probe and a scripted ASR for end-to-end testing.

## Build and diagnose

```bash
brew install cmake
cargo build -p voice-input
cargo run -p voice-input -- init
cargo run -p voice-input -- model list
cargo run -p voice-input -- model install --preset balanced
cargo run -p voice-input -- doctor
cargo run -p voice-input -- record --seconds 3 --output /tmp/voice-input.wav
cargo run --release -p voice-input -- transcribe /tmp/voice-input.wav
cargo run --release -p voice-input -- dictate --seconds 5 --stdout
cargo run -p voice-input -- benchmark
```

`model install` downloads into the application data directory, resumes an
interrupted HTTP transfer, verifies the expected size and SHA-256, atomically
publishes the model, and updates the canonical TOML configuration. No model is
committed to the repository or silently downloaded when the daemon starts.

`doctor --prompt-accessibility` opens the macOS Accessibility permission prompt.
For an installed service, run the installed helper once so macOS associates the
permission with the stable `Voice Input` bundle rather than the terminal:

```bash
"$HOME/Library/Application Support/voice-input/Voice Input.app/Contents/MacOS/voice-input" doctor --prompt-accessibility
```

Microphone permission is requested by macOS the first time the signed app
container opens the input device. Both permissions require an explicit user
choice in System Settings; the installer never edits the TCC database.

## Run as a user service

Build a release binary, install it at a stable per-user path, and start it:

```bash
cargo build --release -p voice-input
./target/release/voice-input install
./target/release/voice-input start
./target/release/voice-input status
```

`install` writes the default config when needed, builds an ad-hoc signed
`~/Library/Application Support/voice-input/Voice Input.app` container with a
stable bundle identifier and microphone usage declaration, and writes
`~/Library/LaunchAgents/com.lifcc.voiceinput.plist`. The LaunchAgent points at the
bundle's helper; this gives macOS a stable privacy-permission subject instead of
running an anonymous copied CLI. It does not start the service; the lifecycle
remains explicit. `start` bootstraps or restarts it. Runtime health is atomically
published as JSON under the application data directory.
Completed-session latency is appended under `metrics/latency.jsonl` in the same
directory. `benchmark` summarizes the history; `benchmark --input <path>` can
analyze an exported JSONL file.

Development installs use ad-hoc signing. Set
`VOICE_INPUT_CODESIGN_IDENTITY` to a persistent Apple Development or Developer
ID identity for a distributable build whose designated requirement survives
binary updates. With ad-hoc signing, reinstalling a changed binary changes its
code hash and macOS may require permission approval again.

```bash
./target/release/voice-input stop
./target/release/voice-input uninstall
```

`uninstall` removes the installed app container, plist, and current runtime
status. It keeps configuration and logs so diagnostics and user choices are not destroyed.
Only one daemon can own the instance lease, whether it was started manually or
by launchd.

## Run local ASR

The `balanced` preset is the recommended interactive baseline. Fixing the
language avoids language-detection work and can materially reduce tail latency.
An initial prompt can steer vocabulary and output style without an LLM:

```toml
[asr]
backend = "whisper"
model_path = "/path/to/ggml-small-q5_1.bin"
language = "zh"
initial_prompt = "以下是简体中文普通话的准确语音转写。"
threads = 4
use_gpu = true
flash_attention = true
max_audio_seconds = 120

[asr.vad]
enabled = true
rms_threshold = 0.008
min_speech_ms = 80
speech_pad_ms = 120
```

`transcribe` accepts a signed 16-bit, mono, 16 kHz WAV and reports model warmup,
inference latency, and real-time factor as JSON. `dictate` records one fixed
duration from the real microphone and executes ASR, refinement, and insertion
without relying on a hotkey. These are diagnostic and benchmark surfaces; the
daemon is the normal interactive path.

## Verify the OS path without ASR

First test insertion only:

```bash
cargo run -p voice-input -- inject "Voice Input insertion works"
```

Then start the daemon with a fixed transcript:

```bash
cargo run -p voice-input -- daemon --mock-text "本地语音链路已经贯通"
```

Hold `Control+Shift+Space`, speak, and release. Audio is captured and passed
through the same streaming port that a real model will use; the scripted ASR
returns the fixed text so hotkey, capture, state transitions, timing, and text
insertion can be tested independently of model choice.

Use `--stdout` to avoid text injection while testing:

```bash
cargo run -p voice-input -- daemon --mock-text "smoke test" --stdout
```

## Hotkey, insertion, and feedback

The default configuration is conservative and works without claiming the Fn
key. To use press-and-hold Fn, select the native event-tap adapter:

```toml
hotkey = "fn"

[insertion]
mode = "auto"
restore_clipboard_after_ms = 160

[feedback]
audible = true
```

`insertion.mode = "auto"` first writes `AXSelectedText` in the focused control,
which does not touch the pasteboard. If the target control does not expose a
writable Accessibility attribute, the adapter falls back to Command-V. The
fallback snapshots every materializable pasteboard item and type, then restores
the snapshot only if neither the user nor the target application changed the
pasteboard after injection. Use `"accessibility"` to prohibit fallback or
`"clipboard"` to force it.

Feedback sounds run on their own bounded worker. They never execute on the
CoreAudio callback or inference path and can be disabled with `audible = false`.

## Optional LLM refinement

Identity refinement is the default and adds no network access or model latency.
An OpenAI-compatible local server can be enabled explicitly:

```toml
[refiner]
backend = "openai_compatible"
base_url = "http://127.0.0.1:11434/v1"
model = "your-local-model"
api_key_env = "LOCAL_LLM_API_KEY"
timeout_ms = 900
max_output_tokens = 512
allow_remote = false
failure_mode = "bypass"
system_prompt = "Polish the transcript without changing its meaning. Return only the corrected text."
```

Loopback is allowed by default. A non-loopback endpoint requires both HTTPS and
`allow_remote = true`, so audio-derived text cannot leave the machine because
of an accidental URL change. API key values are read from the named environment
variable and never stored in TOML. `failure_mode = "bypass"` returns the raw ASR
text on timeout or provider failure; `"fail"` prevents insertion instead.

## Provider boundary

A local ASR adapter implements `StreamingAsr`:

```rust,ignore
#[async_trait]
impl StreamingAsr for LocalAsr {
    async fn begin(&mut self, session: &SessionContext) -> Result<()>;
    async fn push_audio(&mut self, chunk: AudioChunk) -> Result<Vec<TranscriptUpdate>>;
    async fn finish(&mut self) -> Result<Transcript>;
}
```

The contract is 16 kHz mono signed PCM. Models should remain loaded across
sessions. Audio callbacks never wait for inference; chunks cross a bounded
channel into the model worker.

The in-process provider ingests bounded chunks during capture and performs final
decoding after release. The model context and `WhisperState` are initialized
before the daemon announces readiness, then reused across sessions. An atomic
generation token lets shutdown abort native inference without leaking callback
state.

The LLM stage implements `Refiner`. `IdentityRefiner` and
`OpenAiCompatibleRefiner` are selected only at the composition root; neither the
domain state machine nor the macOS adapters depend on a provider.

The boundary and lifecycle contracts are documented in
[`docs/architecture.md`](docs/architecture.md).

## Current boundaries

- macOS only;
- `Control+Shift+Space` remains the default; `hotkey = "fn"` uses the native
  event-tap path and requires Accessibility/Input Monitoring permission;
- native microphone rates are resampled to 16 kHz on a dedicated high-quality
  sinc-resampler thread;
- generated output is plain text, although rich/image/file clipboard contents
  are preserved across the Command-V fallback;
- final Whisper decoding starts after release; stable partial decoding and
  speculative refinement are not implemented yet;
- the built-in RMS VAD trims edge silence but is not a neural endpoint detector;
- model presets are downloaded on explicit request, not bundled;
- one same-machine TTS sample and one microphone loopback prove the path and
  latency mechanics, not broad accuracy or Typeless equivalence.

## Local performance evidence

Measured on this machine (Apple M3 Max), release build, warm Metal kernels, one
5.926 second Mandarin TTS sample:

| Preset | Fixed language | ASR latency | RTF | Observed result |
| --- | --- | ---: | ---: | --- |
| `quality` (`large-v3-turbo-q5_0`) | no | 1,558 ms | 0.263 | Accurate simplified Chinese |
| `balanced` (`small-q5_1`) | no | 611 ms | 0.103 | Accurate content, traditional Chinese |
| `balanced` + `language = "zh"` + simplified prompt | yes | 196 ms | 0.033 | Accurate simplified Chinese |

A separate seven-second real microphone loopback completed final ASR in 192 ms
with 88 chunks and zero queue drops. Treat these as engineering smoke evidence;
a multi-speaker corpus and p50/p95 run are still required for a product claim.

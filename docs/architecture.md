# Voice Input Architecture

## Native Application Boundary

The macOS application is a thin SwiftUI shell. It owns menu-bar presentation,
first-run guidance, system permission affordances, login-item registration, and
the cursor-adjacent transient status panel. The Rust executable remains the
only owner of audio capture, Fn monitoring, VAD, Whisper, optional LLM
refinement, text insertion, metrics, and LaunchAgent state.

The boundary is a versioned JSON control plane rather than duplicated config
models or direct TOML mutation:

```text
SwiftUI App
  ├── RuntimeBridge (Process control + direct atomic status reads)
  ├── PermissionSettingsService (System Settings links only)
  ├── CandidateOverlayController (read-only AX cursor location)
  └── AppModel (presentation state)
           │
           ▼
installed voice-input permission snapshot / request
voice-input control snapshot / apply / test-refiner
           │
           ▼
Rust config, service, model, runtime, and refiner modules
```

`control snapshot` is a read-only aggregate with an explicit schema version.
`control apply` accepts a deny-unknown-fields patch and saves through the
canonical Rust validation path. Long-running operations such as model download
and service restart remain explicit Rust commands. This keeps Swift independent
of TOML shape and keeps product presentation out of the speech core.

The GUI owns login behavior through `SMAppService.mainApp`. The helper
LaunchAgent is installed with `RunAtLoad=false` and `KeepAlive=false`; it starts
only when the menu-bar application explicitly bootstraps and kickstarts it.
This prevents the settings toggle from claiming the service is disabled while
launchd silently starts it anyway.

Interactive phase feedback does not spawn the control CLI. `AppModel` reads the
atomically replaced runtime JSON every 125 ms and accepts it only when its PID
matches launchd's current process. A five-second control refresh reconciles
service, model, configuration, and runtime-helper permission state. Historical
runtime data can populate “recent text” and latency, but never drives a live
candidate overlay or ready/error state.

## Objective

Build a local-first, low-latency macOS dictation runtime whose audio path stays
warm, bounded, observable, and independent of any particular ASR or LLM. The
engineering phase covered here makes the runtime safe to operate as a user
LaunchAgent. The current P2 layer adds in-process Whisper/Metal and an optional
OpenAI-compatible Refiner without moving provider concerns into the domain.

## Current Evidence

| Area | Evidence | Implication |
| --- | --- | --- |
| Entrypoint | `src/main.rs` exposes diagnostics, recording, injection, and the daemon | Keep argument parsing and report rendering in the binary |
| Core models | `domain.rs` owns sessions, engine states, transcripts, and latency facts | Domain state can stay free of OS and filesystem handles |
| Runtime | `pipeline.rs` owns the strict per-dictation state machine | Add one application runtime above it; do not create a second session state machine |
| Adapters | `platform/macos.rs` owns CoreAudio, Carbon/event-tap hotkeys, Accessibility, pasteboard, and feedback effects | launchd, signals, and filesystem status belong behind adapter contracts too |
| Configuration | `config.rs` is the canonical TOML schema | LaunchAgent configuration must point at this file, not duplicate values in its plist |
| Diagnostics | tracing events and `LatencyReport` exist | Persist a small current-state snapshot; keep logs as detailed history |
| Tests | pipeline and resampler tests are headless | Runtime lifecycle and service rendering also need fake/contract tests |
| Local ASR | `asr.rs` owns the Whisper worker, VAD, generation cancellation, and session buffer | Native inference stays out of the async scheduler and audio callback |
| Model supply | `models.rs` owns a pinned artifact catalog and atomic installer | Startup never performs an implicit network download |
| LLM | `refiner.rs` owns the OpenAI-compatible HTTP contract and privacy gates | The pipeline sees only the `Refiner` trait and can retain identity fallback |

## Reference Models Considered

| Reference | Borrow | Do not copy | Source |
| --- | --- | --- | --- |
| Tokio | One runtime owns scheduling; cancellation and blocking rules are explicit | Nested runtimes or CPU inference on async workers | Tokio runtime documentation |
| Tauri/Wry | Keep platform capabilities outside application logic | Plugin/codegen machinery for a single-purpose daemon | Tauri architecture documentation |
| ripgrep/Cargo | Thin CLI, reusable operations, boundary-specific reporting | Many crates before boundaries need independent reuse | Cargo and ripgrep source layouts |
| tracing | Producers emit facts while the binary chooses formatting/export | Global subscriber installation from library code | tracing documentation |

## Chosen Shape

State ownership model: one `VoiceRuntime` owns all mutable daemon resources and
serializes typed hotkey/shutdown events; `VoiceEngine` remains the sole owner of
per-dictation workflow state.

```text
product/app
  - main.rs: CLI parsing, dependency composition, user-facing output

core/domain
  - domain.rs: session/transcript/latency models and invariants
  - pipeline.rs: Idle -> Capturing -> Finalizing -> Inserting

runtime/application
  - runtime.rs: daemon lifecycle, event dispatch, shutdown, status facts
  - service.rs: service operations and structured service status contracts
  - metrics.rs: append-only latency samples and deterministic summaries
  - models.rs: explicit download, resume, integrity check, atomic publication

adapters/backends
  - asr.rs: resident whisper.cpp/Metal worker and utterance-edge VAD
  - refiner.rs: optional OpenAI-compatible text refinement
  - platform/macos.rs: audio, hotkey, Accessibility, pasteboard, signals
  - platform/launchd.rs: user LaunchAgent installation and control
  - status.rs: atomic filesystem status and exclusive instance lease

plugins/components
  - StreamingAsr, Refiner, TextInjector, AudioCapture, StatusPublisher,
    Feedback, LatencySink

testing/headless
  - fake audio/status adapters and pure plist/status contract tests
```

The architecture stays in one crate while there is one native ASR provider.
`whisper-rs` is macOS-target-specific, so Linux contract checks do not build or
link Metal. Move ASR providers into separate crates when a second native runtime
lands or their independent release/build cost becomes material.

## Source Of Truth And Migration Debt

| Contract | Source of truth | Consumers | Duplicate/fork | Action |
| --- | --- | --- | --- | --- |
| Dictation state | `VoiceEngine` | runtime, tests | No duplicate after P0 convergence | Dispatch moved to `VoiceRuntime`; old loop deleted from `main.rs` |
| Runtime health | `RuntimeSnapshot` | status CLI, diagnostics | Logs are historical facts, not current state | Atomically publish one snapshot |
| Configuration | `VoiceConfig` TOML | runtime, installer, doctor | plist arguments can drift | plist only supplies config path; TOML remains canonical |
| Service registration | generated plist | launchd | No checked-in static plist | Render deterministically from `ServicePaths` |
| Model capability | `VoiceConfig` plus provider traits | daemon, doctor, transcribe | Scripted backend remains a deterministic OS-path probe | Keep scripted selection behind explicit mock input |
| Model artifacts | `models.rs` pinned catalog | installer, operator docs | Hugging Face metadata can drift | Verify size and SHA-256 before atomic publication |
| LLM credentials | environment variable named by TOML | Refiner adapter only | Never copy secret values into status/config | Resolve at provider construction and fail closed if missing |

## Boundary Contracts

| Contract | Owner | Allowed dependencies | Forbidden dependencies | Tests |
| --- | --- | --- | --- | --- |
| Session state | `VoiceEngine` | domain and provider traits | OS handles, files, environment | `pipeline::tests::*` |
| Daemon lifecycle | `VoiceRuntime` | engine, audio/status ports, typed events | launchctl, plist, CLI parsing | `runtime::tests::*` |
| Audio callback | macOS adapter | bounded non-blocking queues | ASR waits, filesystem/network IO | resampler duration and queue tests |
| Current status | `StatusPublisher` implementation | latest-value channel and atomic file replace | mutating runtime state or synchronous disk IO on the hot path | `status::tests::file_status_is_atomic_and_readable` |
| Single instance | `InstanceLease` | one exclusive file lock | PID-only race checks | `status::tests::exclusive_lease_rejects_second_owner` |
| Service lifecycle | `ServiceManager`/launchd adapter | signed app container, explicit binary/config/log paths | model logic, audio handles, TCC database mutation | bundle/plist installer and status parser tests |
| Shutdown | runtime plus signal adapter | typed shutdown event and resource cleanup | process-global exit from core | runtime shutdown test |
| Observability | runtime snapshots and tracing facts | serializer/subscriber adapters | silent user-visible degradation | snapshot transition tests |
| Text insertion | macOS adapter | AXSelectedText, bounded Accessibility timeouts, pasteboard snapshot | model logic, unguarded clipboard overwrite | pasteboard multi-item/type round trip plus target-app smoke |
| Feedback | `Feedback` port and macOS sound worker | typed cues and bounded non-blocking queue | audio callback or inference work | runtime cue-order test |
| Latency history | `LatencySink` and `metrics.rs` | bounded queue, append-only JSONL, pure percentile aggregation | filesystem IO in the runtime hot path | metrics round-trip and percentile tests |
| Local ASR | `LocalWhisperAsr` | one resident native context/state, dedicated worker, bounded utterance | inference on Tokio/CoreAudio threads, model reload per session | VAD tests, WAV benchmark, real microphone smoke |
| Model installation | `models.rs` | explicit HTTPS, Range retry, expected size/hash, atomic rename | implicit startup download, accepting partial/corrupt files | catalog contract and live verified download |
| LLM refinement | `OpenAiCompatibleRefiner` | loopback by default, HTTPS remote opt-in, env secret, timeout, bypass/fail | implicit cloud egress, secret serialization | URL/privacy/failure/success contract tests |

## Compatibility And Deletion Plan

| Path | Why it exists | Owner | Keep until | Delete/converge when |
| --- | --- | --- | --- | --- |
| `daemon --mock-text` | Exercises the entire OS path before ASR selection | voice app | A real ASR passes end-to-end latency/accuracy gates | Make ASR config required and move scripted backend to tests/dev flag |
| Clipboard Command-V insertion | Broad application compatibility | macOS adapter | Direct Accessibility insertion covers target apps | Keep as explicit `auto` fallback or forced `clipboard` mode |
| `Control+Shift+Space` default | Lower-permission onboarding path | configuration | Fn event-tap rollout is proven across the target-app matrix | Keep both adapters; let configuration own the product default |
| Final-only Whisper decode | Preserves accuracy while the provider baseline is measured | ASR adapter | Stable partials beat it on a representative corpus | Add speculative/stable partial decoding without moving text retraction into the OS adapter |

## P0/P1/P2 Roadmap

| Priority | Work | State | Verification |
| --- | --- | --- | --- |
| P0 | Runtime owner, single instance, atomic state, graceful shutdown | Implemented | Runtime/status tests plus SIGINT microphone smoke |
| P0 | signed app container plus launchd install/start/stop/status/uninstall | Implemented | Bundle/plist installer contract tests and structured status checks |
| P0 | CLI and operator documentation | Implemented | CLI help and README command review |
| P1 | Direct Accessibility insertion and complete pasteboard preservation | Implemented; broader app matrix remains a release gate | Multi-item/type round-trip test; TextEdit AX and fallback smoke |
| P1 | Feedback adapter and configurable Fn/event-tap hotkey | Implemented | Runtime cue-order test; Fn key-code daemon smoke with a warm microphone |
| P1 | Latency history and benchmark harness | Implemented | Metrics tests and `voice-input benchmark [--input <jsonl>]` |
| P2 | Local ASR provider, warm state, VAD, model supply | Implemented baseline; final-only decoding remains | Provider/VAD tests, pinned live download, WAV and microphone smoke |
| P2 | Optional local/remote LLM refiners | Implemented | Success, timeout/bypass, URL and privacy contract tests |
| P3 | Stable partial ASR and corpus evaluator | Pending | Same-machine multi-speaker p50/p95 plus CER/WER beats final-only baseline |

## Error And Performance Policy

- Failure to acquire the instance lease or initialize audio/hotkeys is fatal at
  startup and visible through service status/logs.
- The installed helper lives inside a signed app container with the dedicated
  `com.lifcc.voiceinput.runtime` bundle identifier, Audio Input entitlement, and
  microphone usage string. The GUI reads and requests permissions through that
  exact executable. Accessibility and microphone consent remain explicit user
  decisions; installation never mutates macOS TCC data.
  Development installs are ad-hoc signed, while release installs can supply a
  persistent signing identity so the designated requirement survives updates.
- A single dictation failure is recoverable: capture stops, the engine resets,
  an error snapshot is emitted, and the daemon remains ready.
- Status publication failure is diagnostic-only and logged; it must never block
  the audio callback or ASR path.
- Audio callbacks only enqueue bounded native packets. Resampling and inference
  run outside callbacks. CPU-heavy model work must use dedicated workers.
- The Whisper context and state are constructed before `ready`, then owned by
  one inference thread. Per-session audio is bounded by `max_audio_seconds`.
- Cancellation increments a generation token read by the native abort callback;
  callback storage remains owned for the synchronous native call.
- Model download is never part of daemon startup. Interrupted transfers resume,
  and only a verified artifact can replace the configured model.
- Remote refinement is fail-closed unless HTTPS and explicit `allow_remote` are
  both present. The default Refiner is local identity.
- Status, feedback, and latency persistence use latest-value or bounded worker
  queues; none performs filesystem or AppKit work on the audio callback.
- Support exports redact `last_text` and defensive credential fields before
  copying diagnostics; configuration exposes only an environment variable name,
  never its value.
- Shutdown stops capture, drains no new user work, cancels the active ASR
  session, publishes `stopped`, and releases the instance lease.

## Non-Goals

- Claiming Typeless-equivalent accuracy from a TTS sample or loopback recording.
- Bundling model weights or downloading them implicitly.
- Multi-user system daemons, Windows/Linux service managers, UI frameworks, or
  a general plugin registry.

## Open Questions

- Does `small-q5_1` remain below the product latency budget across live speakers,
  accents, noise, English, and mixed Chinese/English?
- Which stable-partial strategy reduces stop-to-insert without regressions or
  text retraction in the focused application?
- Which target applications reject AXSelectedText and therefore require the
  guarded pasteboard fallback?
- Should optional LLM refinement run only after release, or speculatively on
  stable partials under a strict latency budget?

## Readiness

P0/P1 and the P2 provider baseline are implemented and locally verified on an
Apple M3 Max. A warm `small-q5_1` run produced 196 ms final ASR for a 5.926
second fixed-language sample; a microphone loopback produced 192 ms final ASR
with zero dropped chunks. These prove the mechanics, not accuracy equivalence.
Stable partials, a repeatable multi-speaker corpus, percentile/CER/WER gates,
and a broad target-app matrix remain before a production or Typeless claim.

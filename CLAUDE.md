# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**rusty_glauxb** is a GPU-accelerated HUD globe visualization tool written in Rust. It renders Earth as a cube-sphere and overlays multiple real-time and static data layers — flight tracking, weather, crime rates, business locations, housing prices, and more — in a single interactive 3D map.

Core design goals:
- **Cube-sphere geometry**: Earth modeled as a subdivided cube projected onto a sphere for uniform texel density and clean LOD
- **Aggressive culling**: frustum culling, back-face culling, and LOD-based geometry/data culling to maintain 60fps with dense data layers
- **Live data via WebSockets**: real-time feeds (flights, weather) stream through persistent socket connections with configurable refresh rates
- **User-controlled filtering**: per-layer visibility toggles, refresh rate sliders, and data source configuration exposed in a HUD panel
- **Unified data model**: static datasets (housing, crime, POI) and dynamic feeds share a common `DataLayer` trait so rendering and filtering code is layer-agnostic

## Technology Stack

- **Rendering**: `wgpu` (WebGPU backend, cross-platform)
- **Windowing**: `winit`
- **Math**: `glam`
- **Async runtime**: `tokio` with `tokio-tungstenite` for WebSocket feeds
- **Serialization**: `serde` + `serde_json`
- **GUI/HUD overlay**: `egui` with `egui-wgpu` backend
- **HTTP static data**: `reqwest`

## Commands

```bash
# Build
cargo build
cargo build --release

# Run
cargo run
cargo run --release

# Test
cargo test                        # all tests
cargo test <test_name>            # single test by name
cargo test -- --nocapture         # show println output

# Lint / format
cargo clippy -- -D warnings
cargo fmt
cargo fmt -- --check              # CI check (no writes)
```

## Architecture

### Globe Geometry (`src/globe/`)
The cube-sphere is built by subdividing each face of a unit cube into an NxN grid, then normalizing each vertex to the unit sphere. This gives 6 equal-area patches with no polar singularities. LOD is per-face: camera distance determines subdivision level. Backfaces (faces whose normal points away from camera) are culled on the CPU before upload.

### Data Layer System (`src/layers/`)
Each data source implements the `DataLayer` trait:
```rust
pub trait DataLayer: Send + Sync {
    fn name(&self) -> &str;
    fn update(&mut self, ctx: &UpdateCtx) -> anyhow::Result<()>;
    fn visible_points(&self, frustum: &Frustum) -> &[GeoPoint];
}
```
Static layers load once at startup (or lazily on demand). Dynamic layers hold a `tokio::sync::watch` channel updated by a background task that maintains a WebSocket connection.

### Rendering Pipeline (`src/renderer/`)
Single wgpu render pass per frame:
1. Globe mesh with PBR-lite material + satellite texture
2. Data point instanced draw call per visible layer (one draw call per layer, not per point)
3. egui overlay pass for HUD controls

Instanced rendering means adding 10k flight positions costs one draw call. Each layer uploads a `GpuPoint` buffer (lat/lon/alt packed into `vec4`) updated each frame from the watch channel.

### WebSocket Feed Manager (`src/feeds/`)
A `FeedManager` owns all live connections. Each feed runs in a dedicated `tokio::task`, parses incoming JSON into the layer's internal type, and sends through a `watch::Sender`. The main render loop reads the `watch::Receiver` without blocking. Reconnect logic uses exponential backoff.

### HUD (`src/hud/`)
Built with `egui`. Exposes per-layer toggles (visibility, refresh rate, data source URL). Settings persist to `~/.config/rusty_glauxb/settings.toml` via `serde`.

### Voice Pipeline (`src/voice/`)
Push-to-talk (Space bar) → STT → LLM → layer filter + camera focus.

```
audio.rs   — cpal records f32 samples while Space is held; stop() encodes to WAV (hound)
stt.rs     — SpeechToText trait; default impl calls OpenAI Whisper API (OPENAI_API_KEY)
llm.rs     — LlmClient trait; default impl calls Anthropic Claude Haiku (ANTHROPIC_API_KEY)
command.rs — VoiceCommand: layers: Vec<String>, location: Option<GeoTarget>, zoom hint
mod.rs     — VoiceController: owns AudioCapture + Arc<dyn STT> + Arc<dyn LLM>
```

Flow:
1. `start_recording()` — cpal stream begins filling a `Vec<f32>` buffer
2. `stop_and_process()` — encodes WAV, spawns a tokio task: STT → transcript → LLM → `VoiceCommand` sent over `mpsc`
3. `poll_command()` — called each frame from `App`; drains the channel non-blocking
4. `App` calls `LayerManager::apply_filter(&cmd.layers)` and moves the camera to `cmd.location`

The STT and LLM implementations are trait objects (`Arc<dyn SpeechToText>`, `Arc<dyn LlmClient>`), so swapping in a local Whisper (`whisper-rs`) or local LLM (Ollama HTTP) requires only changing the concrete type passed to `VoiceController::new`.

LLM system prompt (`llm::build_system_prompt`) lists available layer names and instructs the model to return JSON only. `llm::parse_command` deserialises the response into `VoiceCommand`.

## GitHub Workflow

### Issues
Use the following labels on GitHub Issues:
- `layer:<name>` — work scoped to a specific data layer (e.g. `layer:flights`)
- `renderer` — GPU/wgpu pipeline changes
- `globe` — cube-sphere geometry and LOD
- `feed` — WebSocket/HTTP data ingestion
- `hud` — egui overlay and settings
- `perf` — optimization work
- `bug`, `enhancement`, `chore`

### Project Board
Use a GitHub Project (kanban) with columns: **Backlog → In Progress → Review → Done**. Link issues to milestones:

| Milestone | Goal |
|-----------|------|
| `v0.1-globe` | Spinning cube-sphere renders with basic camera controls |
| `v0.2-layers` | Static data layers (POI, housing) visible on globe |
| `v0.3-live` | WebSocket flights + weather layer streaming |
| `v0.4-hud` | Full HUD with per-layer filters and settings persistence |
| `v1.0` | Performance pass, packaging, release |

Branch naming: `feat/<issue#>-short-description`, `fix/<issue#>-short-description`.

PRs must reference their issue (`Closes #N`) and pass CI before merge.

## CI/CD (GitHub Actions)

See [.github/workflows/ci.yml](.github/workflows/ci.yml). The pipeline runs on every push and PR to `main`:

1. `cargo fmt -- --check`
2. `cargo clippy -- -D warnings`
3. `cargo test`
4. (release builds on tags) `cargo build --release`

Release artifacts are uploaded when a `v*` tag is pushed.

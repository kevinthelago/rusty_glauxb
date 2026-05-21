# rusty_glauxb

**GPU-accelerated 3D globe HUD written in Rust.** Renders Earth as a cube-sphere and overlays real-time and static data — flight tracking, geospatial layers, driving directions, NATO military symbology, and more — in a single interactive 3D map.

Built on [wgpu](https://wgpu.rs) (WebGPU), [egui](https://github.com/emilk/egui), and [tokio](https://tokio.rs).

---

## Features

- **Cube-sphere globe** — uniform texel density, no polar singularities, per-face LOD
- **Live flight tracking** — ADS-B via ADS-B Exchange, ADS-B.fi, OpenSky, or FlightAware; real-time position updates over WebSocket / HTTP polling
- **Geospatial layers** — world map polygons (Natural Earth), OpenStreetMap roads/railways, rivers, lakes, tectonic plates, submarine cables, earthquake feeds, and more via a built-in GeoJSON download catalog
- **Driving directions** — geocoded routes rendered as a 3D line on the globe (OSRM or Google Maps)
- **NATO military symbology** — MIL-STD-2525D / APP-6A symbol frames with affiliation color-coding; SVG icon support per standard
- **Location follow** — GPS dot on the globe with camera lock (mock, WebSocket, or native Android)
- **Voice commands** — push-to-talk → Whisper STT → Claude LLM → layer filter + camera focus
- **Headless benchmark** — `bench` binary renders frames against real GPU hardware without a window
- **Unicode text** — multi-script font fallback chain (Hindi/Devanagari, Arabic, CJK, Korean, and more) via system fonts

---

## Requirements

- Rust 1.78+ (stable)
- A GPU with a WebGPU-capable driver (Vulkan, DX12, Metal, or OpenGL ES)
- For voice commands: `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` env vars

---

## Running

```bash
cargo run --release
```

Open the in-app console with `` ` `` or `~`.

### Headless Benchmark

```bash
cargo run --release --bin bench -- [--frames N] [--scenario <name>] [--software] [--json]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--frames N` | `300` | Frames to measure (10 warm-up excluded) |
| `--scenario` | `globe-only` | `globe-only`, `full-scene` (500 flights), `stress` (5 000 flights) |
| `--software` | off | Force software (CPU) adapter |
| `--json` | off | Emit results as JSON |

---

## Console Commands

Open with `` ` `` or `~`. Type `help` for a listing; `<cmd> help` for detailed usage.

| Command | Description |
|---------|-------------|
| `set` | Change a setting and save to disk |
| `get` | Read the current value of a setting |
| `list` | Show all settings and their current values |
| `source` | List datastreams and their current sources, or switch a source (`flights`, `directions`, `location`) |
| `refresh` | Force-refresh a data layer; busts disk cache for download layers |
| `layer` | Toggle layer visibility |
| `log` | View and control the in-app log stream |
| `geo` | Control the world-map polygon layer |
| `topo` | Load and control named GeoJSON topology layers |
| `download` | Browse and fetch GeoJSON datasets from the built-in catalog |
| `directions` | Get driving directions between two locations and render the route on the globe |
| `camera` | Control camera animation mode |
| `diag` | Print a performance diagnostics table (FPS, GPU/CPU time, RSS, draw calls) |
| `clip` | Copy any command's output (or filtered log entries) to the clipboard |
| `location` | Control device location services and the globe position dot |
| `aircraft` | Configure aircraft dot labels and click-to-follow |
| `milsym` | Toggle NATO military symbology mode and configure affiliations |
| `symbol` | Assign SVG icons per symbology standard; switch active standard (APP-6A / MIL-STD-2525D) |
| `clear` | Clear console output |
| `exit` | Close the console |
| `kill` | Stop pipeline streams (`kill <stream>`) or exit the app (`kill app`) |

Available layer names: `roads`, `railways`, `rivers`, `lakes`, `railroads`, `topo`.

`rivers`, `lakes`, and `railroads` are loaded automatically from Natural Earth on first run and cached locally for 30 days.

---

### Aircraft Display

Aircraft render as colored dots projected onto the globe. Mouse over for a tooltip; click to follow (when follow is enabled).

| Sub-command | Syntax | Description |
|-------------|--------|-------------|
| *(bare)* | `aircraft` | Show current label and follow settings |
| `label` | `aircraft label <field>` | Persistent label: `none` `callsign` `type` `altitude` `speed` `icao24` |
| `follow` | `aircraft follow on\|off` | Enable/disable click-to-follow |

Dot colors: green = single-engine, blue = multi-engine/airliner, red = military, amber = misc/balloon, gray = on ground.

---

### Location Services

| Command | Syntax | Description |
|---------|--------|-------------|
| `source location <src>` | `source location mock` | Switch the active source |
| `location` | `location` | Show current source and last fix |
| `location connect` | `location connect ws://ip:port` | Set WebSocket URL and connect |
| `location follow` | `location follow on\|off` | Lock camera to follow device position |
| `location status` | `location status` | Print lat / lon / alt / hdg / spd / acc |
| `location stop` | `location stop` | Stop location services |

| Source | Description |
|--------|-------------|
| `mock` | Animated circular path around San Francisco (always available) |
| `network` | WebSocket stream from a companion device |
| `native` | Android device GPS (Android only) |
| `off` | Stop location services and hide the dot |

---

### Military Symbology

`milsym` replaces entity dots with MIL-STD-2525D / APP-6A symbol frames. Frame shape and color encode affiliation; the icon inside the frame encodes function.

| Affiliation | Frame | Color |
|-------------|-------|-------|
| Friend | Rectangle | Blue |
| Hostile | Diamond | Red |
| Neutral | Square | Green |
| Unknown | Circle | Yellow |

| Sub-command | Syntax | Description |
|-------------|--------|-------------|
| `on` / `off` | `milsym on\|off` | Enable or disable symbology mode |
| `scale` | `milsym scale <f>` | Symbol size multiplier — icon size also scales with zoom |
| `echelon` | `milsym echelon on\|off` | Show/hide echelon indicator above frame |
| `affiliation` | `milsym affiliation civil\|military\|unknown <affil>` | Override affiliation for an aircraft category |

`symbol` manages SVG icon assignments per standard:

| Sub-command | Syntax | Description |
|-------------|--------|-------------|
| `assign` | `symbol assign <key> <path>` | Load an SVG file and assign to an icon key |
| `clear` | `symbol clear <key>` | Revert a key to the built-in hand-drawn icon |
| `standard` | `symbol standard app6a\|milstd2525d` | Switch the active standard |
| `list` | `symbol list` | List all keys with loaded/missing status |

Default affiliation mapping:

| ADS-B category | Default affiliation |
|----------------|---------------------|
| `single_engine`, `multi_engine` | `friend` |
| `military` | `hostile` |
| `misc`, `balloon`, other | `unknown` |

---

### Directions

```
directions <from> to <to>
directions <from> | <to>
```

Both endpoints can be GPS coordinates (`lat,lon`) or free-form place names.

| Example | Description |
|---------|-------------|
| `directions San Francisco to Los Angeles` | City names |
| `directions 37.7749,-122.4194 to 34.0522,-118.2437` | GPS coordinates |
| `directions JFK \| LAX` | Airport codes |

Switch routing source: `source directions osrm` or `source directions google` (requires `google_maps_api_key`).

Remove the route: `topo drop directions`.

---

### GeoJSON Download Catalog

`download` fetches datasets and loads them as named topo layers. Layers are persisted between sessions.

| Sub-command | Syntax | Description |
|-------------|--------|-------------|
| *(bare)* | `download` | List catalog sources and cache status |
| `<name>` | `download earthquakes` | Fetch a catalog source |
| `url` | `download url mydata <url>` | Fetch any GeoJSON URL |
| `clear` | `download clear <name>` | Remove a layer and delete its cache |

Catalog:

| Name | Description | Cache |
|------|-------------|-------|
| `graticules` | 10° lat/lon coordinate grid | 1 year |
| `coastlines` | High-detail coastlines | 1 year |
| `tectonic` | Tectonic plate boundaries | 1 year |
| `timezones` | Timezone boundaries | 1 year |
| `glaciers` | Glaciated areas | 1 year |
| `reefs` | Coral reefs and atolls | 1 year |
| `urban` | World urban areas | 1 year |
| `protected_areas` | National parks and protected lands | 1 year |
| `airports` | World airports (point data) | 30 days |
| `ports` | World seaports (point data) | 30 days |
| `populated_places` | World cities and towns (point data) | 30 days |
| `submarine_cables` | Undersea internet cable routes | 30 days |
| `earthquakes` | USGS significant earthquakes — past 30 days | 1 hour |
| `earthquakes_all` | USGS M2.5+ earthquakes — past 7 days | 1 hour |
| `volcanoes` | Active volcanoes worldwide | 7 days |

---

### Camera

| Sub-command | Syntax | Description |
|-------------|--------|-------------|
| `rotate` | `camera rotate [speed] [elasticity]` | Auto-rotate animation |
| `follow aircraft` | `camera follow aircraft <icao24>` | Lock onto an aircraft |
| `follow location` | `camera follow location` | Lock onto device GPS |
| `follow smoothing` | `camera follow smoothing <0–1>` | Follow lag (0 = instant, 1 = very slow) |
| `follow distance` | `camera follow distance <r>` | Camera radius in globe radii |
| `overlay on\|off` | `camera overlay on\|off` | Toggle the follow-mode HUD info panel |
| `none` | `camera none` | Free camera |
| `status` | `camera status` | Show current mode and parameters |

---

### Diagnostics

| Sub-command | Syntax | Description |
|-------------|--------|-------------|
| *(bare)* | `diag` | Rolling stats table (mean / p50 / p95 / p99 / max) |
| `reset` | `diag reset` | Clear the accumulator |
| `overlay on\|off` | `diag overlay on` | Compact fps/gpu/cpu line in the HUD corner |

Metrics: FPS, frame time (ms), GPU time (ms), CPU submission time (ms), RSS memory (MB), triangle count, draw calls.

---

### Settings Keys

| Key | Description |
|-----|-------------|
| `flight_source` | `adsbfi`, `opensky`, `adsbexchange`, `flightaware` |
| `refresh_secs` | Fetch interval in seconds |
| `camera_mode` | `none`, `rotate`, `follow`, or `follow_location` |
| `rotate_speed` | Rotation speed in degrees/second |
| `rotate_elasticity` | Rotation smoothing (0–1) |
| `follow_entity` | ICAO24 of aircraft to follow |
| `follow_smoothing` | Follow lag (0–1, default 0.15) |
| `follow_distance` | Camera radius while following (globe radii) |
| `idle_resume_secs` | Seconds idle before rotate resumes after a drag (`0` = disabled) |
| `directions_source` | `osrm` or `google` |
| `google_maps_api_key` | API key for Google Maps Directions |
| `milsym_scale` | Military symbol size multiplier |

---

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `` ` `` or `~` | Toggle console |
| `Space` (hold) | Push-to-talk voice command |
| Left-drag | Orbit globe |
| Scroll | Zoom |
| Click aircraft dot | Follow (when `aircraft follow on`) |
| Click country | Drill down to states/provinces |

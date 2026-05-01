# rusty_glauxb

GPU-accelerated HUD globe visualization in Rust. Renders Earth as a cube-sphere with real-time data layers — flight tracking, weather, and more — in a single interactive 3D map.

## Running

```bash
cargo run --release
```

Open the in-app console with the backtick key `` ` ``.

## Console Commands

Commands typed into the in-app console (`` ` `` to toggle). Type `help` for a listing; type `<cmd> help` for detailed usage of any command.

| Command | Description |
|---------|-------------|
| `set` | Change a setting and save to disk |
| `get` | Read the current value of a setting |
| `list` | Show all settings and their current values |
| `source` | List datastreams and their current sources, or switch a source |
| `refresh` | Force-refresh a data layer |
| `layer` | Toggle layer visibility |
| `log` | View and control the in-app log stream |
| `geo` | Control the world-map polygon layer |
| `topo` | Load and control named GeoJSON topology layers |
| `camera` | Control camera animation mode |
| `spectrum` | Toggle the RF spectrum analyzer overlay |
| `diag` | Print a performance diagnostics table (FPS, GPU/CPU time, RSS, draw calls) |
| `clip` | Copy any command's output (or filtered log entries) to the clipboard |
| `location` | Control device location services and the globe position dot |
| `clear` | Clear console output |
| `exit` | Close the console |
| `kill` | Stop pipeline streams (`kill <stream>`) or exit the app (`kill app`) |

Available layer names: `roads`, `railways`, `topo`.

### Location Services

The `location` command controls which GPS source feeds the globe's "you are here" dot.

| Sub-command | Syntax | Description |
|-------------|--------|-------------|
| *(bare)* | `location` | Show current source and last fix |
| `source` | `location source mock\|network\|native\|off` | Switch the active source |
| `connect` | `location connect ws://ip:port` | Set WebSocket URL and connect |
| `follow` | `location follow on\|off` | Lock camera to follow device position |
| `status` | `location status` | Print lat / lon / alt / hdg / spd / acc |
| `stop` | `location stop` | Stop location services |

Available sources:

| Source | Description |
|--------|-------------|
| `mock` | Animated circular path around San Francisco (always available, good for testing) |
| `network` | WebSocket stream from a companion device; set URL with `location connect` |
| `native` | Android device GPS via the system location manager (Android only) |
| `off` | Stop location services and hide the dot |

### Kill / Exit

`kill` is a multi-purpose command:

| Syntax | Description |
|--------|-------------|
| `kill` | Stop all pipeline streams (flights, roads, railways) |
| `kill <stream>` | Stop a single named stream, e.g. `kill flights` |
| `kill app` | Exit the application |

In-flight fetch tasks finish naturally when a stream is stopped; no new fetches are scheduled for it afterwards.

`exit` closes the console panel without affecting running processes.

### Diagnostics

The `diag` command samples performance stats collected over a rolling 300-frame (~5 s) window and prints a percentile table to the console.

| Sub-command | Syntax | Description |
|-------------|--------|-------------|
| *(bare)* | `diag` | Print the rolling stats table (mean / p50 / p95 / p99 / max) |
| `reset` | `diag reset` | Clear the accumulator and start fresh |
| `overlay on\|off` | `diag overlay on` | Show a compact fps/gpu/cpu line in the top-left HUD |

Metrics reported: FPS, frame time (ms), GPU time (ms, requires `TIMESTAMP_QUERY` adapter feature), CPU submission time (ms), RSS memory (MB), triangle count, and draw call count.

#### Headless Benchmark (`bench` binary)

Run against real GPU hardware without opening a window:

```bash
cargo run --release --bin bench -- [--frames N] [--scenario <name>] [--software] [--json]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--frames N` | `300` | Number of frames to measure (10 warm-up frames excluded) |
| `--scenario <name>` | `globe-only` | Scene load: `globe-only`, `full-scene` (500 flights), `stress` (5 000 flights) |
| `--software` | off | Force software (CPU) fallback adapter |
| `--json` | off | Emit results as JSON instead of ASCII table |

### Spectrum Analyzer

The spectrum overlay renders a live RF frequency/power bar chart anchored at the bottom of the screen.

| Sub-command | Syntax | Description |
|-------------|--------|-------------|
| *(bare)* | `spectrum` | Show usage hint |
| `on` / `off` | `spectrum on\|off` | Enable or disable the overlay |
| `source` | `spectrum source` | List available data sources |
| `source <src>` | `spectrum source dummy` | Switch to a named source |

Available sources: `dummy` (animated 2.4/5 GHz WiFi simulation, always available). Future sources: `wifi` (Android WiFi scan via JNI), `rtlsdr` (RTL-SDR USB dongle).

### World Map

The world map layer renders Natural Earth GeoJSON polygons on the globe surface. Click any country to drill down to its states/provinces. Click again to select a state. Use `geo back` (or the console command) to navigate back up to the country level.

| Sub-command | Syntax | Description |
|-------------|--------|-------------|
| `names` | `geo names on\|off` | Country/state name labels projected to screen space |
| `fill` | `geo fill on\|off` | Polygon fill (semi-transparent) |
| `borders` | `geo borders on\|off` | Polygon border outlines |
| `back` | `geo back` | Navigate up one administrative level |
| `status` | `geo status` | Show feature count, triangle count, and visibility state |
| *(bare)* | `geo` | Show current names/fill/borders toggle state |

Data is fetched from the Natural Earth GitHub mirror on first run and cached locally for 30 days.

### Log System

The in-app console captures all `DEBUG`+ events regardless of `RUST_LOG`. The terminal only shows `INFO`+ (or whatever `RUST_LOG` specifies). Log lines are formatted as:

```
[LEVEL][file:line][span] message
```

Where `[span]` is the enclosing function name when `#[tracing::instrument]` is used, otherwise omitted. Changing `log level` or `log module` only affects what is displayed going forward — it does not retroactively filter lines already in the console output.

### Camera Sub-commands

| Sub-command | Syntax | Description |
|-------------|--------|-------------|
| `rotate` | `camera rotate [speed] [elasticity]` | Start auto-rotate animation; optional speed (deg/s) and elasticity |
| `follow aircraft` | `camera follow aircraft <icao24>` | Lock camera onto an aircraft by ICAO24 hex code |
| `follow location` | `camera follow location` | Lock camera onto the device GPS position |
| `none` | `camera none` | Free camera — disable animation |
| `status` | `camera status` | Show current animation mode and parameters |

### Settings Keys

| Key | Description |
|-----|-------------|
| `flight_source` | Data source: `adsbfi`, `opensky`, `adsbexchange`, `flightaware` |
| `refresh_secs` | Fetch interval in seconds |
| `camera_mode` | `none`, `rotate`, or `follow` |
| `rotate_speed` | Rotation speed in degrees/second |
| `rotate_elasticity` | Rotation smoothing (0 = instant, 1 = very smooth) |
| `follow_entity` | ICAO24 hex code of the aircraft to follow |
| `follow_smoothing` | Follow smoothing factor |
| `follow_distance` | Camera distance when following |

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `` ` `` | Toggle console |
| `Space` (hold) | Push-to-talk voice command |
| Left-drag | Orbit globe |
| Scroll | Zoom |

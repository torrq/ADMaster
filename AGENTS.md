# ADMaster -- Pure-Rust Studio & DSP Architecture Guidelines

## 1. Repository Architecture (`v2.0.1`)

**ADMaster** is a pure-Rust desktop GUI studio (`admaster`) and headless CLI (`admaster-cli`) built with `egui` / `eframe`, `rustfft`, `rayon`, and external `ffmpeg` / `ffprobe` / `mkvmerge` subprocesses. Legacy Python sync scripts and DescribeAlign were retired in `v2.0.0`.

### Core Rust Modules (`src/`)
* **`src/main.rs` & `src/lib.rs`**: GUI entry point, native HiDPI icon rasterization via `resvg`, and release unit test suite.
* **`src/bin/admaster_cli.rs` & `src/cli.rs`**: Standalone zero-GUI CLI (`--no-default-features`), argument parser, JSON reporting, and terminal dual-meter progress display.
* **`src/app.rs`**: Desktop GUI (`egui` / `eframe`) featuring vertical Step 1 / Step 2 drag-and-drop staging, async `ffprobe` "Match Against" stream inspector, live output filename previews, 4-stage live progress dashboard with dual Overall/Current Task meters and clickable `Elapsed <-> ETA` timers, instant subprocess cancellation, filterable/searchable Text Log, and full-height two-pane Settings (including Appearance & Logs naming templates).
* **`src/visual_ui.rs`**: Layout C Friendly Report Card (Sync Summary banner with verdict badge, 4-card sync checklist, visual Movie Structure Map filmstrip, interactive hover alignment curve / anchor inspector, and Output Files Ready with Reveal/Open Folder actions).
* **`src/config.rs`**: Cross-platform `config.toml` persistence (`directories`), `PalMode` / `SyncMode`, log directory & token-based `log_filename_pattern` formatting, and external tool path discovery.
* **`src/media.rs`**: Async `ffprobe` media & audio stream probing (`probe_audio_streams`), cancellable subprocess runner (`run_cancellable_command` with instant `child.kill()` + partial file cleanup), live FFmpeg `-progress pipe:1` and `mkvmerge` progress parsing, atomic pre-decoding (`_tmp.wav` -> `os::rename`), and `-rf64 auto` WAV handling.
* **`src/pipeline.rs`**: End-to-end movie & batch orchestration, PAL speed probing, single-offset vs. non-linear warp dispatch, gap filling, ITU-R BS.775 stereo downmixing, EBU R128 Night Mode encoding, MKVMerge multiplexing, and session logging (defaults to source MKV directory).
* **`src/dsp/filters.rs`**: 4th-order zero-phase Butterworth SOS `filtfilt` 3-band speech suppression (`< 250 Hz` weight 1.0, `250 - 1200 Hz` speech weight 0.2, `> 1200 Hz` weight 0.8).
* **`src/dsp/fingerprint.rs`**: STFT acoustic landmark constellation hashing with 70th-percentile peak flooring and single-pass `O(n_matches)` bucket histogram matching for automatic 25fps PAL vs. 23.976fps speed probing.
* **`src/dsp/correlation.rs`**: 100-anchor parallel cross-correlation using thread-local cached 5-smooth `f32` SIMD FFTs with 2-real-in-1 Hermitian packing and parabolic sub-sample peak interpolation.
* **`src/dsp/warp.rs`**: Native Rust ADSync non-linear warp engine -- pre-computed 4 kHz 3-band signals, fingerprint-guided tight interior search, second-order Viterbi candidate lattice DP, micro-blip spur filtering, silence-nudged cut boundaries, monotone Fritsch-Carlson PCHIP cubic splines, and 30-second streaming Catmull-Rom cubic rendering (`< 35 MB` peak RAM).
* **`src/dsp/gap_fill.rs`**: Windowed Reference Master Audio gap splicer (`>= 0.25 s` voids, `-2.0 dB` RMS capped level matching, 50 ms equal-power cosine crossfades, `< 15 MB` RAM).

### Build-Time Asset Scripts (`scripts/`)
* `scripts/render_icon.py`: Rasterizes `assets/icon.svg` into `.ico` and `.icns`.
* `scripts/render_banner.py`: Generates `assets/banner.png` from `assets/icon.svg`.

---

## 2. Build, Test & CI Release Parity Rules

The GitHub Actions workflow (`.github/workflows/release.yml`) builds Windows (`x86_64`), Linux (`x86_64`, glibc 2.28+ via `cargo-zigbuild`), and macOS (`Universal arm64 + x86_64 .app`) with `--locked`.

Before pushing any commit or tag, ALWAYS verify all three checks locally with **0 warnings**:
```powershell
cargo test --release --locked
cargo build --release --locked --bins
cargo check --release --locked --no-default-features --bin admaster-cli
```
* **Lockfile Rule**: Whenever bumping `version` in `Cargo.toml` or updating dependencies, run `cargo test --release` first to sync `Cargo.lock`, then commit both `Cargo.toml` and `Cargo.lock` so `--locked` CI builds pass.
* **Cross-Platform GUI/CLI Rule**: Keep GUI crates (`eframe`, `egui`, `resvg`, `rfd`) strictly behind `#[cfg(feature = "gui")]` so headless `admaster-cli --no-default-features` always compiles cleanly on Windows, Linux, and macOS.

---

## 3. Critical Alignment, DSP & FFmpeg Invariants

### 1. Never Force PAL Slowdown From Filename Tags
* In `PalMode::AutoDetect`, **ALWAYS** decode the AD track at native `1.0x` first and run `detect_speed_pal` (landmark constellation prober).
* Never assume `[UK]` or shorter total duration implies a 25.0 fps PAL speedup; native-speed UK releases and shorter Theatrical Cuts run at `1.0x` (23.976 fps). Only apply PAL slowdown when `detect_speed_pal` confirms a true ~1.0427x speed ratio or the user explicitly sets `--pal`.

### 2. Offset Sign Convention (`correlation` vs `warp`)
* `correlation.rs` defines `winner_offset = t_ad - t_vid`:
  * `offset > 0` (AD starts earlier / has extra intro) -> trim start (`-ss <offset>` after `-i`).
  * `offset < 0` (AD starts later) -> prepend silence (`adelay=<ms>|<ms>`).
* `warp.rs` defines lattice offsets as `t_vid - t_ad`, so `pipeline.rs` MUST pass `-winner_offset` as `offset_hint` into `warp.rs`. Inverting this sign causes false cut hallucinations and stalls.

### 3. Sample-Accurate PAL -> Film Conversion (`asetrate` vs `atempo`)
* Always use dynamic sample-rate clock restoration rather than `atempo`:
  `asetrate = input_sr * (23.976023976 / 25.0)`, followed by `aresample=48000`.
* Always probe the input sample rate first (`48000 Hz -> 46033.966`, `44100 Hz -> 42293.706`). Never hardcode `46033.966`.

### 4. Raw Bitstream Seeking & Subprocess Safety
* **Never repeatedly seek into raw bitstreams** (`.dts`, `.ac3`, `.eac3`, `.thd`, `.dtshd`) with FFmpeg `-ss`. Always pre-decode once to an 8 kHz mono analysis WAV (atomically via `_tmp.wav`) and slice in memory.
* **Always pass `-nostdin`** to FFmpeg so it never hangs waiting on terminal stdin.
* **Always pass `-rf64 auto`** when extracting 48 kHz WAV intermediates to avoid the 4 GB RIFF WAV truncation bug on movies longer than 3h06m.
* **Always use `run_cancellable_command`** (polling `try_wait` + `child.kill()` on cancel) so clicking **Cancel Sync** terminates active FFmpeg/mkvmerge jobs immediately and deletes partial outputs.

### 5. Strict 2.0 Stereo Downmixing & Loudnorm
* Whenever generating Stereo or Night Mode deliverables from a surround source (`channels >= 6`), ALWAYS apply the ITU-R BS.775 pan matrix and explicit `-ac 2 -ar 48000`:
  * Stereo AAC: `-af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,alimiter=level_in=1:level_out=0.98:limit=0.98" -ac 2 -c:a aac -b:a 256k -ar 48000`
  * Night Mode AAC: `-af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -ac 2 -c:a aac -b:a 256k -ar 48000`
* Remember `loudnorm` upsamples internally to 192 kHz -> 96 kHz unless `-ar 48000` is explicitly set after the filter chain.

### 6. Streaming Memory Discipline (`< 55 MB` RAM)
* Never allocate full-movie `f64` stereo waveform arrays for 3-hour features.
* Keep `warp.rs` rendering in 30-second streaming chunks (Catmull-Rom cubic interpolation) and `gap_fill.rs` reading reference audio via windowed random-access WAV I/O.

### 7. MKVPropEdit Track Selector Syntax & Chapters
* `--edit track:=<uid>` selects by 64-bit Matroska Track UID (`=`).
* `--edit track:@<n>` selects by 1-based Track Number (`@`). Never pair `@` with a Track UID.
```powershell
mkvextract "movie.mkv" chapters "chapters.xml"
mkvpropedit "movie.mkv" --chapters "chapters.xml"
```

---

## 4. Pure ASCII Enforcement Rule
* **All Rust source files, Python asset scripts, YAML workflows, JSON/TOML configs, markdown documentation, comments, and console outputs MUST be 100% pure ASCII.**
* Never use Unicode checkmarks (use `[OK]`), cross marks (use `[X]`), em-dashes (use `--`), en-dashes (use `-`), Unicode bullets (use `*`), Unicode arrows (use `->` or `<-`), curly quotes (use `'` or `"`), mathematical symbols (use `+/-`, `*`, `pi`, `^2`), or box-drawing characters.

<img width="1672" height="500" alt="ADMaster" src="assets/banner.png" />

# ADMaster -- Pure-Rust Audio Description Synchronization & Mastering Studio

**ADMaster** (`v2.0.4`) is a high-performance, low-memory pure-Rust desktop studio (`admaster`) and standalone command-line interface (`admaster-cli`) for synchronizing, time-warping, downmixing, loudness-normalizing, and multiplexing Audio Description (AD) and secondary audio tracks for movies and television series.

It combines **100-anchor 3-band narration-suppressed SIMD FFT cross-correlation** with a native Rust implementation of the **[ADSync](https://github.com/JohnnyTheCoder1/ADSync)** non-linear alignment architecture (STFT acoustic landmark constellation PAL speed probing, second-order Viterbi candidate lattice dynamic programming, monotone Fritsch-Carlson PCHIP cubic spline warping, micro-blip spur filtering, and silence-nudged cut boundary refinement), plus a **streaming 30-second Catmull-Rom cubic warp renderer** and **windowed Master Audio gap splicer** that keep peak memory usage under **55 MB RAM** even on 3.5-hour feature films.

---

## Binaries & Workspace Architecture

| Binary / Path | Purpose |
| :--- | :--- |
| **`admaster` / `admaster.exe`** | **Desktop GUI Studio (`egui` / `eframe`)** -- Vertical Step 1 / Step 2 drag-and-drop workspace with live `ffprobe` audio stream inspector (`Match Against`), live output filename previews, 4-stage live progress dashboard with dual Overall/Current Task meters and clickable `Elapsed <-> ETA` timers, Layout C Friendly Report Card (plain-English Sync Summary, 4-card checklist, Movie Structure Map filmstrip, interactive alignment curve, Output Files Ready), instant FFmpeg/mkvmerge cancellation, and full-height two-pane Settings studio. |
| **`admaster-cli` / `admaster-cli.exe`** | **Headless Command-Line Runner** -- Zero-GUI batch automation for movie folders, multi-movie queue directories (`--batch`), and explicit `--ref` / `--ad` file pairs with real-time progress and `--json` machine-readable reporting. |
| **`assets/icon.svg`** | Minimal vector master icon (Fuchsia & Amethyst interlocking PCHIP spline waves) compiled into `.ico` / `.icns` and rasterized at native DPI in-app via `resvg`. |
| **`scripts/render_icon.py` & `render_banner.py`** | Build-time vector rasterizers that compile `assets/icon.svg` into multi-size `.ico`, `.icns`, and the README hero banner (`assets/banner.png`). |

---

## Quick Start

### 1. Desktop GUI (`admaster`)
Launch `admaster` (or `admaster.exe` on Windows, `ADMaster.app` on macOS) and drag-and-drop media directly into the window:
* **Step 1 -- Reference Movie / Master**: Drop a video or surround master (`.mkv`, `.mp4`, `.m2ts`, `.dts`, `.dtshd`, `.thd`). ADMaster probes embedded audio streams asynchronously via `ffprobe` so you can pick the exact **Match Against** track (showing language, channel layout, codec, and default/commentary flags).
* **Step 2 -- Tracks to Sync**: Drop one or more Audio Description or secondary audio files (`.mp3`, `.m4a`, `.flac`, `.aac`, `.ogg`, `.opus`, `.wav`, `.w64`, `.ac3`, `.eac3`, `.mka`) with live output filename previews.
* **Movie Project Folder Drop**: Drop a folder containing a movie and audio tracks (or `source/` and `tosync/`) to automatically stage them in Step 1 and Step 2 so you can review settings before clicking **Start Sync** (enable immediate auto-start on drop in **Settings -> Appearance & Logs** if desired).
* **Live Progress & Instant Cancel**: Watch per-track status, Overall + Current Task progress bars driven by live FFmpeg and `mkvmerge` progress streams, switch between **Elapsed** and **ETA** live (with configurable default in **Settings -> Appearance & Logs**), or click **Cancel Sync** to terminate running subprocesses immediately and clean up partial outputs.
* **Output Format & Bitrate**: Defaults to `AAC (.m4a)` at `256 kbps`, with one-click switching (top bar or **Settings -> Encoding**) to `WAV (.wav)`, `Wave64 (.w64)`, `FLAC (.flac)`, `MP3 (.mp3)`, `AC3 (.ac3)`, `EAC3 (.eac3)`, and `Opus (.opus)`, plus configurable Stereo and 5.1 Surround AC3 bitrates.
* **Session Logs**: Saved alongside the source MKV folder by default. Customize the log folder and token-based filename pattern (`{title}`, `{mkv}`, `{date}`, `{time}`, `{datetime}`, `{tracks}`, `{method}`) in **Settings -> Appearance & Logs**.

### 2. Command-Line Interface (`admaster-cli`)

```powershell
# 1. Auto-discover source/ and tosync/ inside a movie folder and also convert Main Audio:
admaster-cli "F:\uploads\queue\Movie1" --convert-main

# 2. Batch-process all movie subdirectories inside a parent queue directory:
admaster-cli "F:\uploads\queue" --batch --convert-main

# 3. Sync explicit reference and AD track files directly and mux a final MKV:
admaster-cli --ref "F:\Master.mkv" --ad "F:\Track_UK_AD.m4a" --mux

# 4. Select a specific reference audio stream and force Time-Warp alignment:
admaster-cli --ref "F:\Master.mkv" --ref-stream 0:a:1 --ad "F:\Track_AD.mp3" --force-warp --no-pal

# 5. Fast uncompressed 48kHz stereo WAV test pass (skip Night Mode loudnorm):
admaster-cli "F:\uploads\queue\Movie1" --wav --no-night

# 6. Render a 180-second test preview MKV sample with custom log filename template:
admaster-cli "F:\uploads\queue\Movie1" --preview 180 -m --log-pattern "{mkv}_sync_{date}.log"
```

Run `admaster-cli --help` for the complete list of flags (`--format`, `--bitrate`, `--surround-bitrate`, `--anchors`, `--min-confidence`, `--offset-only`, `--pal`, `--no-pal`, `--no-speed-detect`, `--no-multiband`, `--no-gap-fill`, `--stereo-only`, `--target-i`, `--target-tp`, `--threads`, `--clean-scratch`, `--keep-scratch`, `--log-dir`, `--log-pattern`, `--no-log-to-file`, `--no-render`, `--json`).

---

## Core DSP & Mastering Pipeline

1. **3-Band Narration Speech Suppression (`src/dsp/filters.rs`)**:
   4th-order zero-phase Butterworth SOS `filtfilt` cascade splits audio into Low (`< 250 Hz`), Mid/Speech (`250 - 1200 Hz`), and High (`> 1200 Hz`) bands, downweighting the narrator speech band to `0.2` so alignment locks onto background score and sound effects rather than voiceover narration.
2. **Automatic Landmark Constellation PAL Speed Prober (`src/dsp/fingerprint.rs`)**:
   Always decodes at native `1.0x` first (never blindly assuming PAL from `[UK]` filename tags), extracts STFT spectral peak pairs with 70th-percentile peak flooring, and runs single-pass `O(n_matches)` bucket histogram matching at native `1.0x` (`23.976 fps`) and PAL `1.042709x` (`25.0 fps`). When a true PAL transfer is confirmed, applies dynamic sample-rate clock restoration (`asetrate = input_sr * 23.976024 / 25.0`, `aresample=48000`).
3. **100-Anchor Packed SIMD FFT Cross-Correlation (`src/dsp/correlation.rs`)**:
   Evaluates 100 anchors across `5%..88%` of the timeline in parallel via `rayon` using thread-local cached 5-smooth `f32` SIMD FFTs with 2-real-in-1 Hermitian packing and parabolic sub-sample peak interpolation.
4. **Second-Order Viterbi Lattice & Monotone PCHIP Warping (`src/dsp/warp.rs`)**:
   When alternate cuts, reel splices, or censorship trims are detected (or `--force-warp` is enabled), pre-computes 4 kHz 3-band signals, runs fingerprint-guided tight interior search + second-order Viterbi dynamic programming, strips spurious micro-blips, nudges structural cut boundaries into narrator silence windows, fits monotone Fritsch-Carlson PCHIP cubic splines, and renders the warped stereo track in 30-second streaming chunks via 4-point Catmull-Rom cubic interpolation (`< 35 MB` peak RAM).
5. **Windowed Reference Master Audio Gap Filling (`src/dsp/gap_fill.rs`)**:
   Detects excised scenes / timeline voids (`>= 0.25 s`) using spline-endpoint offset analysis and splices level-matched (`-2.0 dB` RMS capped) reference soundtrack audio across the gap with `50 ms` equal-power cosine crossfades via windowed WAV reads (`< 15 MB` RAM).
6. **ITU-R BS.775 True 2.0 Stereo & EBU R128 Night Mode (`src/pipeline.rs`)**:
   * **ITU-R BS.775 Downmix Matrix**: `c0 = L + 0.7071*C + 0.7071*Ls`, `c1 = R + 0.7071*C + 0.7071*Rs` with explicit `-ac 2` and `-ar 48000`.
   * **Stereo Deliverable**: BS.775 downmix + `alimiter=level_in=1:level_out=0.98:limit=0.98`.
   * **Night Mode Deliverable**: BS.775 downmix + EBU R128 loudness normalization (`loudnorm=I=-16:TP=-1.5:LRA=11`).

---

## Credits, Acknowledgments & Open-Source Software

ADMaster stands on the shoulders of outstanding open-source audio, video, and Rust projects. Full credit and gratitude go to their authors and maintainers:

### Core Alignment & Media Engines
* **[ADSync](https://github.com/JohnnyTheCoder1/ADSync)** by **Anilo ([JohnnyTheCoder1](https://github.com/JohnnyTheCoder1))** (*MIT License -- Copyright (c) 2026 Anilo*) -- The foundational non-linear Audio Description alignment architecture from which ADMaster's Rust warping pipeline (`src/dsp/`) is adapted, including 3-band narration speech suppression, STFT acoustic landmark constellation speed probing, second-order Viterbi candidate lattice dynamic programming, monotone Fritsch-Carlson PCHIP spline fitting, and silence-aware cut boundary refinement.
* **[FFmpeg](https://github.com/FFmpeg/FFmpeg)** by **The FFmpeg Developers** (*LGPLv2.1+ / GPLv2+*) -- Universal multimedia framework invoked as an external subprocess for container probing (`ffprobe`), stream decoding, dynamic `asetrate` PAL clock conversion, ITU-R BS.775 surround downmixing, EBU R128 `loudnorm` loudness normalization, and AAC / AC3 encoding.
* **[MKVToolNix](https://github.com/mbunkus/mkvtoolnix)** by **[Moritz Bunkus](https://github.com/mbunkus)** (*GPLv2*) -- Matroska multimedia manipulation suite (`mkvmerge`, `mkvpropedit`, `mkvextract`) invoked as an external subprocess for final MKV multiplexing, visual-impaired (`flag-visual-impaired`) and commentary (`flag-commentary`) track flagging, and chapter extraction/injection.

### Rust GUI, Vector & DSP Ecosystem
* **[egui & eframe](https://github.com/emilk/egui)** by **[Emil Ernerfeldt (emilk)](https://github.com/emilk)** (*MIT / Apache-2.0*) -- Immediate-mode pure-Rust GUI library powering the ADMaster desktop studio.
* **[resvg](https://github.com/linebender/resvg)** & **[tiny-skia](https://github.com/linebender/tiny-skia)** by **[Yevhenii Reizner](https://github.com/RazrFalcon) & the Linebender team** (*MIT / Apache-2.0*) -- Pure-Rust SVG parsing and high-precision 2D vector rasterization for crisp HiDPI icons.
* **[RustFFT](https://github.com/ejmahler/RustFFT)** by **[Elliott Mahler (ejmahler)](https://github.com/ejmahler)** (*MIT / Apache-2.0*) -- SIMD-accelerated pure-Rust Fast Fourier Transform library used for packed 5-smooth `f32` anchor cross-correlation and STFT spectral landmark extraction.
* **[Rayon](https://github.com/rayon-rs/rayon)** by **[Niko Matsakis](https://github.com/nikomatsakis) & [Josh Stone](https://github.com/cuviper)** (*MIT / Apache-2.0*) -- Data-parallel work-stealing concurrency library accelerating multi-core anchor correlation and Viterbi lattice scoring.
* **[rfd (Rusty File Dialog)](https://github.com/PolyMeilex/rfd)** by **[PolyMeilex](https://github.com/PolyMeilex)** (*MIT*) -- Native cross-platform file and folder picker dialogs.
* **[Crossbeam](https://github.com/crossbeam-rs/crossbeam)** by **The Crossbeam Project Developers** (*MIT / Apache-2.0*) -- Lock-free channels powering real-time pipeline event streaming between background DSP worker threads and the GUI.
* **[Serde](https://github.com/serde-rs/serde)** & **[toml-rs](https://github.com/toml-rs/toml)** (*MIT / Apache-2.0*) -- Configuration serialization (`config.toml` and `sync_params.json` scratch caching).
* **[Chrono](https://github.com/chronotope/chrono)** (*MIT / Apache-2.0*) -- Timestamp formatting for session logs and reports.
* **[directories-rs](https://github.com/dirs-dev/directories-rs)** (*MIT / Apache-2.0*) -- Standard platform configuration directory resolution.
* **[winresource](https://github.com/mxre/winresource)** (*MIT*) -- Build-time Windows PE executable icon and manifest embedding.

---

## Building from Source

```powershell
# Run unit tests (PCHIP monotonicity, packed FFT correlation, Viterbi warp, log tokens):
cargo test --release --locked

# Build optimized release binaries (admaster + admaster-cli):
cargo build --release --locked --bins

# Verify headless CLI build without GUI dependencies:
cargo check --release --locked --no-default-features --bin admaster-cli
```

---

## License & Third-Party Notices

**ADMaster** is Copyright (c) 2026 **torrq** and licensed under the [MIT License](LICENSE).

Portions of the non-linear audio alignment pipeline in `src/dsp/` are adapted from **[ADSync](https://github.com/JohnnyTheCoder1/ADSync)**, Copyright (c) 2026 **Anilo ([JohnnyTheCoder1](https://github.com/JohnnyTheCoder1))**, licensed under the [MIT License](LICENSE). Full copyright and permission notices for ADSync and all third-party dependencies are included in [`LICENSE`](LICENSE) and inside the application's **About** screen.

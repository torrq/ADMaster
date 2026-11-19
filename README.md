<img width="1672" height="500" alt="ADMaster" src="assets/banner.png" />

# ADMaster -- Pure-Rust Audio Description Synchronization & Mastering Studio

**ADMaster** (`v2.0`) is a high-performance, low-memory pure-Rust desktop studio (`admaster.exe`) and standalone command-line interface (`admaster-cli.exe`) for synchronizing, time-warping, downmixing, loudness-normalizing, and multiplexing Audio Description (AD) and secondary audio tracks for movies and television series.

It combines **100-anchor 3-band narration-suppressed FFT cross-correlation** with a native Rust implementation of the **[ADSync](https://github.com/JohnnyTheCoder1/ADSync)** non-linear alignment architecture (Shazam-style acoustic landmark constellation PAL speed probing, second-order Viterbi candidate lattice dynamic programming, monotone Fritsch-Carlson PCHIP cubic spline warping, and silence-nudged cut boundary refinement), plus a **streaming 30-second Catmull-Rom cubic warp renderer** and **windowed Master Audio gap splicer** that keep peak memory usage under **55 MB RAM** even on 3.5-hour feature films.

---

## Binaries & Workspace Architecture

| Binary / Path | Purpose |
| :--- | :--- |
| **`admaster` / `admaster.exe`** | **Desktop GUI Studio (`egui` / `eframe`)** -- Two-zone color-coded drag-and-drop staging (`Reference Master` vs. `Tracks to Sync`), interactive PCHIP warp curve & 100-anchor confidence inspector, full-height two-pane Settings studio, and live progress console. |
| **`admaster-cli` / `admaster-cli.exe`** | **Headless Command-Line Runner** -- Zero-GUI batch automation for movie folders, multi-movie queue directories (`--batch`), and explicit `--ref` / `--ad` file pairs with `--json` machine-readable reporting. |
| **`assets/icon.svg`** | Minimal vector master icon (Fuchsia & Amethyst interlocking PCHIP spline waves) compiled into `.ico` / `.icns` and rasterized at native DPI in-app via `resvg`. |
| **`scripts/render_icon.py` & `render_banner.py`** | Build-time vector rasterizers that compile `assets/icon.svg` into multi-size `.ico`, `.icns`, and the README hero banner (`assets/banner.png`). |

---

## Quick Start

### 1. Desktop GUI (`admaster.exe`)
Launch `admaster.exe` and drag-and-drop media directly into the window:
* **Drop a Video / Surround Master** (`.mkv`, `.mp4`, `.m2ts`, `.dts`, `.dtshd`, `.thd`) -> automatically assigned to **Zone 1: Reference Master (Sync To)**.
* **Drop Syncable Audio Tracks** (`.mp3`, `.m4a`, `.flac`, `.aac`, `.ogg`, `.opus`, `.wav`, `.w64`, `.ac3`, `.eac3`, `.mka`) -> automatically added to **Zone 2: Audio Tracks to Sync**.
* **Drop a Movie Project Folder** (containing `source/` and `tosync/`) -> automatically pairs the reference file with all candidate AD tracks.

### 2. Command-Line Interface (`admaster-cli.exe`)

```powershell
# 1. Auto-discover source/ and tosync/ inside a movie folder and also convert Main Audio:
admaster-cli.exe "F:\uploads\queue\Movie1" --convert-main

# 2. Batch-process all movie subdirectories inside a parent queue directory:
admaster-cli.exe "F:\uploads\queue" --batch --convert-main

# 3. Sync explicit reference and AD track files directly:
admaster-cli.exe --ref "F:\Master.mkv" --ad "F:\Track_UK_AD.m4a" --mux

# 4. Force non-linear Viterbi + PCHIP spline warp alignment (bypass global anchor consensus):
admaster-cli.exe "F:\uploads\queue\Movie2" --force-warp --no-pal

# 5. Fast uncompressed 48kHz stereo WAV test pass (skip Night Mode loudnorm):
admaster-cli.exe "F:\uploads\queue\Movie1" --wav --no-night

# 6. Render a 180-second test preview MKV sample:
admaster-cli.exe "F:\uploads\queue\Movie1" --preview 180 -m
```

---

## Core DSP & Mastering Pipeline

1. **3-Band Narration Speech Suppression (`src/dsp/filters.rs`)**:
   4th-order zero-phase Butterworth SOS `filtfilt` cascade splits audio into Low (`< 250 Hz`), Mid/Speech (`250 - 1200 Hz`), and High (`> 1200 Hz`) bands, downweighting the narrator speech band to `0.2` so cross-correlation locks onto background score and sound effects rather than voiceover narration.
2. **Automatic 2-Second Landmark PAL Speed Prober (`src/dsp/fingerprint.rs`)**:
   Extracts Shazam-style STFT spectral peak pairs in a streaming ring buffer and evaluates monotone bucket chains at native `1.0x` (`23.976 fps`) and PAL `1.042709x` (`25.0 fps`) without requiring `[UK]` or `[PAL]` filename tags.
3. **100-Anchor Parallel FFT Cross-Correlation (`src/dsp/correlation.rs`)**:
   Evaluates 100 anchors across `5%..88%` of the timeline in parallel via `rayon` and `rustfft` with parabolic sub-sample peak interpolation.
4. **Second-Order Viterbi Lattice & Monotone PCHIP Warping (`src/dsp/warp.rs`)**:
   When alternate cuts, reel splices, or censorship trims are detected, builds a 4 kHz 3-band candidate lattice, solves the globally optimal path via second-order Viterbi dynamic programming (penalizing offset jumps and slope curvature), nudges structural cut boundaries into narrator silence windows (`_nudge_to_silence`), fits monotone Fritsch-Carlson PCHIP cubic splines, and renders the warped stereo track in 30-second streaming chunks via 4-point Catmull-Rom cubic interpolation (`< 35 MB` peak RAM).
5. **Windowed Reference Master Audio Gap Filling (`src/dsp/gap_fill.rs`)**:
   Detects excised scenes / timeline voids (`>= 0.25 s`) and splices level-matched (`-2.0 dB` RMS capped) reference Blu-ray soundtrack audio across the gap with `50 ms` equal-power cosine crossfades using random-access windowed WAV reads (`< 15 MB` RAM).
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
* **[RustFFT](https://github.com/ejmahler/RustFFT)** by **[Elliott Mahler (ejmahler)](https://github.com/ejmahler)** (*MIT / Apache-2.0*) -- SIMD-accelerated pure-Rust Fast Fourier Transform library used for 100-anchor sliding cross-correlation and STFT spectral landmark extraction.
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
# Run unit tests (PCHIP monotonicity, 3-band correlation, streaming WAV & gap fill):
cargo test --release --locked

# Build optimized release binaries (target/release/admaster.exe and admaster-cli.exe):
cargo build --release --locked --bins
```

---

## License & Third-Party Notices

**ADMaster** is Copyright (c) 2026 **torrq** and licensed under the [MIT License](LICENSE).

Portions of the non-linear audio alignment pipeline in `src/dsp/` are adapted from **[ADSync](https://github.com/JohnnyTheCoder1/ADSync)**, Copyright (c) 2026 **Anilo ([JohnnyTheCoder1](https://github.com/JohnnyTheCoder1))**, licensed under the [MIT License](LICENSE). Full copyright and permission notices for ADSync and all third-party dependencies are included in [`LICENSE`](LICENSE) and inside the application's **About** screen.

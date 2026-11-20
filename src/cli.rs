use crate::config::{AppConfig, OutputFormat, PalMode, SyncMode};
use crate::pipeline::{spawn_pipeline_job, JobTarget, MovieJobReport, PipelineEvent};
use crossbeam_channel::unbounded;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum CliAction {
    LaunchGui(Option<JobTarget>),
    RunHeadless {
        target: JobTarget,
        config: AppConfig,
        no_render: bool,
        json_output: bool,
        show_progress: bool,
    },
    PrintAndExit(i32, String),
}

pub fn attach_parent_console_if_windows() {
    #[cfg(target_os = "windows")]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn AttachConsole(dwProcessId: u32) -> i32;
        }
        const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

pub fn parse_cli_args(args: &[String], base_cfg: &AppConfig) -> CliAction {
    parse_cli_args_with_mode(args, base_cfg, false)
}

pub fn parse_cli_args_headless(args: &[String], base_cfg: &AppConfig) -> CliAction {
    parse_cli_args_with_mode(args, base_cfg, true)
}

fn take_opt_val<'a>(
    flag_name: &str,
    inline_val: Option<&'a str>,
    args: &'a [String],
    idx: &mut usize,
) -> Result<&'a str, String> {
    if let Some(v) = inline_val {
        return Ok(v);
    }
    *idx += 1;
    args.get(*idx)
        .map(|s| s.as_str())
        .ok_or_else(|| format!("Missing value after {}", flag_name))
}

fn parse_cli_args_with_mode(
    args: &[String],
    base_cfg: &AppConfig,
    force_headless: bool,
) -> CliAction {
    if args.is_empty() {
        return if force_headless {
            CliAction::PrintAndExit(0, help_text(true))
        } else {
            CliAction::LaunchGui(None)
        };
    }

    let mut headless = force_headless;
    let mut config = base_cfg.clone();
    let mut batch_mode = false;
    let mut no_render = false;
    let mut json_output = false;
    let mut show_progress = true;
    let mut print_config = false;

    let mut movie_dir: Option<PathBuf> = None;
    let mut ref_path: Option<PathBuf> = None;
    let mut ad_items: Vec<(String, PathBuf)> = Vec::new();
    let mut title_opt: Option<String> = None;
    let mut out_dir_opt: Option<PathBuf> = None;
    let mut scratch_dir_opt: Option<PathBuf> = None;
    let mut mux_video_opt: Option<PathBuf> = None;

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        let (flag, inline_val): (&str, Option<&str>) = if arg.starts_with("--") {
            if let Some((k, v)) = arg.split_once('=') {
                (k, Some(v))
            } else {
                (arg, None)
            }
        } else {
            (arg, None)
        };

        macro_rules! need_val {
            () => {
                match take_opt_val(flag, inline_val, args, &mut i) {
                    Ok(v) => v,
                    Err(e) => return CliAction::PrintAndExit(2, e),
                }
            };
        }

        match flag {
            "-h" | "--help" => {
                return CliAction::PrintAndExit(0, help_text(force_headless));
            }
            "-V" | "--version" => {
                return CliAction::PrintAndExit(
                    0,
                    format!(
                        "ADMaster v{} (MIT License -- Copyright (c) 2026 torrq; ADSync DSP Copyright (c) 2026 Anilo)",
                        env!("CARGO_PKG_VERSION")
                    ),
                );
            }
            "--license" | "--credits" => {
                return CliAction::PrintAndExit(
                    0,
                    include_str!("../LICENSE").to_string(),
                );
            }
            "--cli" => headless = true,
            "--json" => {
                headless = true;
                json_output = true;
            }
            "-q" | "--quiet" | "--no-progress" => {
                headless = true;
                show_progress = false;
            }
            "--print-config" => {
                headless = true;
                print_config = true;
            }
            "--config" => {
                headless = true;
                let p = need_val!();
                match AppConfig::load_from_path(Path::new(p)) {
                    Ok(loaded) => config = loaded,
                    Err(e) => return CliAction::PrintAndExit(2, e),
                }
            }
            "-d" | "--dir" => {
                let v = need_val!();
                movie_dir = Some(PathBuf::from(v));
            }
            "-b" | "--batch" => {
                headless = true;
                batch_mode = true;
            }
            "-r" | "--ref" => {
                headless = true;
                ref_path = Some(PathBuf::from(need_val!()));
            }
            "-a" | "--ad" => {
                headless = true;
                if let Some(v) = inline_val {
                    ad_items.push(parse_ad_spec(v));
                } else {
                    // Consume one or more non-flag arguments after --ad
                    while i + 1 < args.len() && !args[i + 1].starts_with('-') {
                        i += 1;
                        ad_items.push(parse_ad_spec(&args[i]));
                    }
                    if ad_items.is_empty() {
                        return CliAction::PrintAndExit(
                            2,
                            "Missing file path(s) after --ad".to_string(),
                        );
                    }
                }
            }
            "--ref-stream" => {
                headless = true;
                config.ref_stream = need_val!().to_string();
            }
            "-m" | "--mux-video" => {
                headless = true;
                mux_video_opt = Some(PathBuf::from(need_val!()));
            }
            "-o" | "--out" | "--outdir" => {
                headless = true;
                out_dir_opt = Some(PathBuf::from(need_val!()));
            }
            "-t" | "--title" => {
                headless = true;
                title_opt = Some(need_val!().to_string());
            }
            "--preview" => {
                headless = true;
                let v = need_val!();
                match v.parse::<f64>() {
                    Ok(sec) if sec > 0.0 => config.preview_sec = Some(sec),
                    _ => {
                        return CliAction::PrintAndExit(
                            2,
                            format!("Invalid --preview duration: '{}'", v),
                        );
                    }
                }
            }
            "-f" | "--overwrite" | "--force-overwrite" => {
                headless = true;
                config.overwrite = true;
            }
            "--clean-scratch" | "--wipe-scratch" => {
                headless = true;
                config.clean_scratch = true;
            }
            "--keep-scratch" => {
                headless = true;
                config.keep_scratch = true;
            }
            "--scratch-dir" => {
                headless = true;
                scratch_dir_opt = Some(PathBuf::from(need_val!()));
            }
            "--wav" => {
                headless = true;
                config.set_output_format(OutputFormat::Wav);
            }
            "--format" => {
                headless = true;
                let v = need_val!();
                match OutputFormat::from_str_loose(v) {
                    Some(fmt) => config.set_output_format(fmt),
                    None => {
                        return CliAction::PrintAndExit(
                            2,
                            format!(
                                "Unsupported --format '{}'. Supported: m4a, wav, w64, flac, mp3, ac3, eac3, opus",
                                v
                            ),
                        );
                    }
                }
            }
            "--bitrate" => {
                headless = true;
                let v = need_val!();
                let clean = v.trim().trim_end_matches(|c: char| c == 'k' || c == 'K');
                match clean.parse::<u32>() {
                    Ok(kbps) if kbps > 0 => config.audio_bitrate_kbps = kbps,
                    _ => {
                        return CliAction::PrintAndExit(
                            2,
                            format!("Invalid --bitrate '{}'. Example: --bitrate 256 or --bitrate 320k", v),
                        );
                    }
                }
            }
            "--surround-bitrate" => {
                headless = true;
                let v = need_val!();
                let clean = v.trim().trim_end_matches(|c: char| c == 'k' || c == 'K');
                match clean.parse::<u32>() {
                    Ok(kbps) if kbps > 0 => config.surround_bitrate_kbps = kbps,
                    _ => {
                        return CliAction::PrintAndExit(
                            2,
                            format!("Invalid --surround-bitrate '{}'. Supported: 384, 448, 640", v),
                        );
                    }
                }
            }
            "--convert-main" => {
                headless = true;
                config.convert_main = true;
            }
            "--stereo-only" => {
                headless = true;
                config.stereo_only = true;
                config.convert_main = true;
            }
            "--mux" => {
                headless = true;
                config.mux_final_mkv = true;
            }
            "--mkv-dir" => {
                headless = true;
                config.custom_mkv_dir = need_val!().to_string();
            }
            "--no-night" => {
                headless = true;
                config.generate_night_mode = false;
            }
            "--no-render" => {
                headless = true;
                no_render = true;
            }
            "--anchors" => {
                headless = true;
                let v = need_val!();
                match v.parse::<usize>() {
                    Ok(n) if n >= 5 => config.anchors = n,
                    _ => {
                        return CliAction::PrintAndExit(
                            2,
                            format!("Invalid --anchors count: '{}'", v),
                        );
                    }
                }
            }
            "--pal" => {
                headless = true;
                config.pal_mode = PalMode::ForcePal;
            }
            "--no-pal" => {
                headless = true;
                config.pal_mode = PalMode::ForceNative;
            }
            "--min-confidence" => {
                headless = true;
                let v = need_val!();
                match v.parse::<f64>() {
                    Ok(c) if (0.01..=1.0).contains(&c) => config.min_confidence = c,
                    _ => {
                        return CliAction::PrintAndExit(
                            2,
                            format!("Invalid --min-confidence: '{}'", v),
                        );
                    }
                }
            }
            "--force-warp" | "--force-dtw" => {
                headless = true;
                config.sync_mode = SyncMode::ForceWarp;
            }
            "--no-warp-fallback" | "--no-dtw-fallback" | "--offset-only" => {
                headless = true;
                config.sync_mode = SyncMode::OffsetOnly;
            }
            "--warp-engine" => {
                // Consume value for backwards CLI compatibility (always uses Native Rust ADSync engine)
                headless = true;
                let _ = need_val!();
            }
            "--no-speed-detect" => {
                headless = true;
                config.speed_detect = false;
            }
            "--no-multiband" => {
                headless = true;
                config.multiband = false;
            }
            "--no-gap-fill" => {
                headless = true;
                config.gap_fill = false;
            }
            "--target-i" => {
                headless = true;
                let v = need_val!();
                if let Ok(val) = v.parse::<f64>() {
                    config.target_i = val;
                }
            }
            "--target-tp" => {
                headless = true;
                let v = need_val!();
                if let Ok(val) = v.parse::<f64>() {
                    config.target_tp = val;
                }
            }
            "-j" | "--threads" => {
                headless = true;
                let v = need_val!();
                if let Ok(n) = v.parse::<usize>() {
                    config.threads = n.max(1);
                }
            }
            "--ffmpeg" => {
                headless = true;
                config.ffmpeg_path = need_val!().to_string();
            }
            "--debug" => {
                headless = true;
                config.debug_mode = true;
            }
            "--log-to-file" => {
                headless = true;
                config.log_to_file = true;
            }
            "--no-log-to-file" => {
                headless = true;
                config.log_to_file = false;
            }
            "--log-dir" => {
                headless = true;
                config.log_dir = need_val!().to_string();
            }
            "--log-pattern" => {
                headless = true;
                config.log_filename_pattern = need_val!().to_string();
            }
            "--output-pattern" => {
                headless = true;
                config.ad_filename_pattern = need_val!().to_string();
            }
            "--no-stretch" | "--force-stretch" => {
                headless = true;
            }
            other if other.starts_with('-') => {
                return CliAction::PrintAndExit(
                    2,
                    format!("Unknown flag '{}'. Run with --help for usage.", other),
                );
            }
            pos_arg => {
                movie_dir = Some(PathBuf::from(pos_arg));
            }
        }
        i += 1;
    }

    config.sanitize();

    if print_config {
        let toml_out = toml::to_string_pretty(&config)
            .unwrap_or_else(|e| format!("# Serialization error: {}", e));
        return CliAction::PrintAndExit(0, toml_out);
    }

    let target = if let Some(rp) = ref_path {
        Some(JobTarget::Explicit {
            ref_path: rp,
            ad_paths: ad_items,
            title: title_opt,
            out_dir: out_dir_opt,
            scratch_dir: scratch_dir_opt,
            mux_video_path: mux_video_opt,
        })
    } else if let Some(mdir) = movie_dir {
        if batch_mode {
            Some(JobTarget::BatchQueue(mdir))
        } else {
            Some(JobTarget::Directory(mdir))
        }
    } else {
        None
    };

    if !headless {
        return CliAction::LaunchGui(target);
    }

    let Some(target) = target else {
        return CliAction::PrintAndExit(
            2,
            "No target movie directory or --ref file specified.\n\n".to_string()
                + &help_text(force_headless),
        );
    };

    CliAction::RunHeadless {
        target,
        config,
        no_render,
        json_output,
        show_progress,
    }
}

fn parse_ad_spec(spec: &str) -> (String, PathBuf) {
    if let Some((label, path_str)) = spec.split_once('=') {
        let p = PathBuf::from(path_str.trim());
        if p.exists() {
            return (label.trim().to_string(), p);
        }
    }
    let p = PathBuf::from(spec.trim());
    let label = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("AD")
        .to_string();
    (label, p)
}

pub fn execute_headless(
    target: JobTarget,
    config: AppConfig,
    no_render: bool,
    json_output: bool,
    show_progress: bool,
) -> Result<(), String> {
    let (tx, rx) = unbounded();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let is_tty = std::io::stderr().is_terminal();

    spawn_pipeline_job(target, config, no_render, cancel_flag, tx);

    let mut reports: Vec<MovieJobReport> = Vec::new();
    let mut last_had_progress = false;

    while let Ok(ev) = rx.recv() {
        match ev {
            PipelineEvent::Started { .. }
            | PipelineEvent::JobMeta { .. }
            | PipelineEvent::TrackProgress { .. } => {}
            PipelineEvent::LogLine(line) => {
                if !json_output {
                    if last_had_progress && is_tty {
                        eprint!("\r{: <108}\r", "");
                        let _ = std::io::stderr().flush();
                        last_had_progress = false;
                    }
                    println!("{}", line);
                }
            }
            PipelineEvent::Progress {
                overall_fraction,
                step_label,
                completed,
                total,
                ..
            } => {
                if show_progress && is_tty && !json_output {
                    let ov_pct = ((overall_fraction * 100.0).round() as u32).min(100);
                    let pct = ((completed * 100) / total.max(1)).min(100);
                    eprint!("\r  [All {:3}% | Task {:3}%] {: <78}", ov_pct, pct, step_label);
                    let _ = std::io::stderr().flush();
                    last_had_progress = true;
                }
            }
            PipelineEvent::MovieCompleted(rep) => {
                reports.push(rep);
            }
            PipelineEvent::BatchCompleted { .. } => {
                if last_had_progress && is_tty {
                    eprint!("\r{: <100}\r", "");
                    let _ = std::io::stderr().flush();
                }
                break;
            }
            PipelineEvent::Cancelled => {
                return Err("Job cancelled".to_string());
            }
            PipelineEvent::Error(e) => {
                if last_had_progress && is_tty {
                    eprint!("\r{: <100}\r", "");
                    let _ = std::io::stderr().flush();
                }
                return Err(e);
            }
        }
    }

    if json_output {
        let json = serde_json::to_string_pretty(&reports)
            .map_err(|e| format!("Failed to serialize JSON output: {}", e))?;
        println!("{}", json);
    }

    Ok(())
}

pub fn help_text(standalone_cli: bool) -> String {
    let bin = if standalone_cli {
        "admaster-cli"
    } else {
        "admaster"
    };
    format!(
        "ADMaster v{} -- Pure-Rust Audio Description Sync, Viterbi+PCHIP Warp & Encoding Toolkit\n\
================================================================================\n\
Usage:\n\
  {bin} [OPTIONS] [MOVIE_DIR]\n\
  {bin} -r <REF_FILE> --ad <AD_FILE...> [OPTIONS]\n\n\
Core Modes & Discovery:\n\
  <MOVIE_DIR>               Directory with 'source/' & 'tosync/' subfolders (or queue parent with --batch)\n\
  -b, --batch               Process all movie subdirectories inside MOVIE_DIR sequentially\n\
  -r, --ref <PATH>          Explicit reference audio/video file (MKV, DTS, AC3, FLAC, WAV)\n\
  -a, --ad <PATH...>        Explicit AD audio file(s) (or LABEL=PATH)\n\
  --ref-stream <SELECTOR>   Reference audio stream selector (e.g. '0:a:0', '0:a:1', '1')\n\
  --cli                     Run headlessly in terminal without opening the GUI window\n\n\
Alignment & Warping Engine (Native Rust ADSync):\n\
  --anchors <N>             Number of 3-band FFT correlation anchors across timeline (Default: 100)\n\
  --min-confidence <0..1>   Minimum consensus confidence before Viterbi+PCHIP warp fallback (Default: 0.15)\n\
  --force-warp              Force non-linear Viterbi lattice DP + Monotone PCHIP warp (--force-dtw alias)\n\
  --offset-only             Enforce single global offset only (--no-dtw-fallback alias)\n\
  --pal / --no-pal          Force PAL 25->23.976fps sample-rate slowdown or enforce native 1.0x speed\n\
  --no-speed-detect         Disable automatic acoustic landmark PAL speed probing\n\
  --no-multiband            Disable 3-band narration speech suppression (250-1200 Hz downweighting)\n\
  --no-gap-fill             Disable windowed Reference Master Audio splicing into deleted scene voids\n\n\
Deliverables & Encoding:\n\
  --convert-main            Convert & downmix reference main audio (Surround AC3 + Stereo AAC + Night Mode AAC)\n\
  --stereo-only             Downmix reference main audio to True 2.0 Stereo only (skip Surround AC3)\n\
  --no-night                Disable Night Mode (EBU R128 loudnorm) deliverables\n\
  --format <FMT>            Output audio format: m4a (default), wav, w64, flac, mp3, ac3, eac3, opus\n\
  --bitrate <KBPS>          Output stereo bitrate for lossy formats (e.g. 128, 192, 256, 320)\n\
  --surround-bitrate <KBPS> 5.1 Surround AC3 bitrate when converting main audio (384, 448, 640)\n\
  --wav                     Shorthand for --format wav (16-bit 48kHz PCM WAV)\n\
  --mux                     Multiplex final MKV (video + stereo audio + English subtitles) via mkvmerge\n\
  --mkv-dir <DIR>           Destination directory for final multiplexed MKV\n\
  -m, --mux-video <PATH>    Multiplex a preview video sample MKV\n\
  --preview <SECONDS>       Render only the first N seconds for rapid sync verification\n\
  --target-i <LUFS>         Target EBU R128 integrated loudness for Night Mode (Default: -16.0)\n\
  --target-tp <dBTP>        Target True Peak ceiling for Night Mode (Default: -1.5)\n\
  -j, --threads <N>         FFmpeg encoding thread count (Default: 2)\n\n\
File & Scratch Management:\n\
  -o, --outdir <DIR>        Custom output directory (Defaults to <MOVIE_DIR>/output)\n\
  -t, --title <TITLE>       Movie title prefix for output filenames\n\
  -f, --overwrite           Overwrite existing deliverables in output/\n\
  --clean-scratch           Wipe scratch directory before running for a 100% clean run\n\
  --keep-scratch            Keep temporary analysis & intermediate WAVs in scratch/\n\
  --log-dir <DIR>           Custom session log directory (Defaults to the MKV's folder)\n\
  --output-pattern <PAT>    AD output filename template (tokens: {{title}}, {{mkv}}, {{ad}}, {{ad_tag}}, {{variant}}, {{codec}}, {{bitrate}}, {{method}}, {{speed}}, {{date}})\n\
  --log-pattern <PATTERN>   Log filename template (tokens: {{title}}, {{mkv}}, {{date}}, {{time}}, {{datetime}}, {{tracks}}, {{method}})\n\
  --no-log-to-file          Disable writing session log files to disk\n\
  --no-render               Compute & cache alignment parameters only without rendering audio\n\
  --json                    Emit structured JSON job report to stdout\n\
  --print-config            Print effective TOML configuration and exit\n\
  -h, --help                Show this help message\n\
  -V, --version             Show version",
        env!("CARGO_PKG_VERSION")
    )
}

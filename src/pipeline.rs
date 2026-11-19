use crate::config::{AppConfig, PalMode, SyncMode};
use crate::dsp::correlation::{
    evaluate_anchor_correlations, find_offset_candidates, AnchorMeasurement, OffsetCluster,
};
use crate::dsp::fingerprint::{detect_speed_pal, fingerprint_align};
use crate::dsp::gap_fill::{fill_master_audio_gaps_windowed, FilledGapReport};
use crate::dsp::warp::{
    build_candidate_lattice, decode_warp_path, fit_and_refine_warp, render_warped_wav_streaming,
    WarpCut, WarpPlan, WarpPoint,
};
use crate::media::{
    apply_no_window, format_stream_selector, get_audio_info, mux_final_mkv, mux_video_sample,
    predecode_to_mono_wav, read_wav_mono_f32, ToolPaths, PAN_STEREO_BS775,
};
use chrono::Local;
use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone)]
pub enum JobTarget {
    Directory(PathBuf),
    BatchQueue(PathBuf),
    Explicit {
        ref_path: PathBuf,
        ad_paths: Vec<(String, PathBuf)>,
        title: Option<String>,
        out_dir: Option<PathBuf>,
        scratch_dir: Option<PathBuf>,
        mux_video_path: Option<PathBuf>,
    },
}

impl JobTarget {
    pub fn display_label(&self) -> String {
        match self {
            JobTarget::Directory(p) | JobTarget::BatchQueue(p) => p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Movie Folder")
                .to_string(),
            JobTarget::Explicit {
                ref_path, title, ..
            } => title.clone().unwrap_or_else(|| {
                ref_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Explicit Job")
                    .to_string()
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackSyncResult {
    pub name: String,
    pub source_path: PathBuf,
    pub method: String, // "cross_correlation" or "warp"
    pub offset: f64,
    pub atempo: f64,
    pub confidence: f64,
    pub is_pal: bool,
    pub speed_stretch: Option<f64>,
    pub winner_anchors: usize,
    pub total_anchors: usize,
    pub runner_ups: Vec<OffsetCluster>,
    pub anchor_measurements: Vec<AnchorMeasurement>,
    pub warp_points: Vec<WarpPoint>,
    pub warp_cuts: Vec<WarpCut>,
    pub filled_gaps: Vec<FilledGapReport>,
    pub deliverables: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovieJobReport {
    pub base_title: String,
    pub ref_path: PathBuf,
    pub ref_stream_map: String,
    pub ref_channels: u32,
    pub ref_sample_rate: u32,
    pub target_duration: f64,
    pub project_dir: PathBuf,
    pub out_dir: PathBuf,
    pub scratch_dir: PathBuf,
    pub main_deliverables: Vec<PathBuf>,
    pub ad_results: Vec<TrackSyncResult>,
    pub final_mkv: Option<PathBuf>,
    pub elapsed_secs: f64,
    pub skipped: bool,
    pub log_text: String,
}

#[derive(Debug, Clone)]
pub enum PipelineEvent {
    Started {
        total_movies: usize,
        label: String,
    },
    LogLine(String),
    Progress {
        step_label: String,
        completed: usize,
        total: usize,
    },
    MovieCompleted(MovieJobReport),
    BatchCompleted {
        reports: Vec<MovieJobReport>,
        elapsed_secs: f64,
    },
    Cancelled,
    Error(String),
}

pub fn format_time_code(seconds: f64) -> String {
    let sign = if seconds < 0.0 { "-" } else { "" };
    let abs_s = seconds.abs();
    let total_m = (abs_s / 60.0).floor() as u64;
    let s = abs_s - (total_m as f64) * 60.0;
    let h = total_m / 60;
    let m = total_m % 60;
    if h > 0 {
        format!("{}{:02}:{:02}:{:06.3}", sign, h, m, s)
    } else {
        format!("{}00:{:02}:{:06.3}", sign, m, s)
    }
}

/// Auto-discovers source reference MKV/audio in `source/` (or top-level) and AD tracks in `tosync/` (or top-level).
pub fn auto_discover_inputs(directory: &Path) -> (Option<PathBuf>, Vec<(String, PathBuf)>) {
    let source_dir = directory.join("source");
    let search_dirs: Vec<PathBuf> = if source_dir.is_dir() {
        vec![source_dir, directory.to_path_buf()]
    } else {
        vec![directory.to_path_buf()]
    };

    let ref_exts = [
        "mkv", "dts", "dtshd", "ac3", "eac3", "thd", "mka", "flac", "wav", "mp4",
    ];
    let mut ref_file: Option<PathBuf> = None;

    'ref_loop: for s_dir in &search_dirs {
        let Ok(entries) = fs::read_dir(s_dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|d| d.path()))
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        for ext in &ref_exts {
            for f in &files {
                if f.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case(ext))
                    .unwrap_or(false)
                {
                    ref_file = Some(f.clone());
                    break 'ref_loop;
                }
            }
        }
    }

    let tosync_dir = directory.join("tosync");
    let ad_search_dirs: Vec<PathBuf> = if tosync_dir.is_dir() {
        vec![tosync_dir, directory.to_path_buf()]
    } else {
        vec![directory.to_path_buf()]
    };

    let ad_exts = [
        "mp3", "m4a", "aac", "ogg", "flac", "wav", "ac3", "eac3", "thd", "dts", "dtshd", "mka",
    ];
    let mut ad_files: Vec<(String, PathBuf)> = Vec::new();

    for a_dir in &ad_search_dirs {
        let Ok(entries) = fs::read_dir(a_dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|d| d.path()))
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        for ext in &ad_exts {
            for f in &files {
                if Some(f) == ref_file.as_ref() {
                    continue;
                }
                let fname = f.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if fname.ends_with("_mono8k.wav")
                    || fname.ends_with("_mono16k.wav")
                    || fname.ends_with("_temp.wav")
                    || fname.ends_with("_tmp.wav")
                {
                    continue;
                }
                if f.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case(ext))
                    .unwrap_or(false)
                    && !ad_files.iter().any(|(_, existing)| existing == f)
                {
                    let label = f
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("AD")
                        .to_string();
                    ad_files.push((label, f.clone()));
                }
            }
        }
    }

    (ref_file, ad_files)
}

pub fn discover_batch_queue_dirs(parent_dir: &Path, convert_main: bool) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(parent_dir) else {
        return Vec::new();
    };
    let mut subdirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|d| d.path()))
        .filter(|p| {
            if !p.is_dir() {
                return false;
            }
            let name = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            !matches!(
                name.as_str(),
                "scratch" | "output" | "bin" | "source" | "tosync" | "legacy"
            )
        })
        .collect();
    subdirs.sort();

    let mut movie_dirs = Vec::new();
    for d in subdirs {
        if d.join("source").is_dir() || d.join("tosync").is_dir() {
            movie_dirs.push(d);
        } else {
            let (r, ads) = auto_discover_inputs(&d);
            if r.is_some() && (!ads.is_empty() || convert_main) {
                movie_dirs.push(d);
            }
        }
    }
    movie_dirs
}

pub fn spawn_pipeline_job(
    target: JobTarget,
    config: AppConfig,
    no_render: bool,
    cancel_flag: Arc<AtomicBool>,
    tx: Sender<PipelineEvent>,
) {
    std::thread::spawn(move || {
        let start_all = Instant::now();
        let tools = ToolPaths::discover(&config);

        match target {
            JobTarget::BatchQueue(queue_dir) => {
                let movie_dirs = discover_batch_queue_dirs(
                    &queue_dir,
                    config.convert_main || config.stereo_only,
                );
                if movie_dirs.is_empty() {
                    // Fall back to processing queue_dir as a single movie directory
                    let _ = tx.send(PipelineEvent::Started {
                        total_movies: 1,
                        label: queue_dir.display().to_string(),
                    });
                    match process_single_movie_dir(
                        &queue_dir,
                        &config,
                        &tools,
                        no_render,
                        &cancel_flag,
                        &tx,
                    ) {
                        Ok(rep) => {
                            let _ = tx.send(PipelineEvent::MovieCompleted(rep.clone()));
                            let _ = tx.send(PipelineEvent::BatchCompleted {
                                reports: vec![rep],
                                elapsed_secs: start_all.elapsed().as_secs_f64(),
                            });
                        }
                        Err(e) => {
                            if cancel_flag.load(Ordering::Relaxed) {
                                let _ = tx.send(PipelineEvent::Cancelled);
                            } else {
                                let _ = tx.send(PipelineEvent::Error(e));
                            }
                        }
                    }
                    return;
                }

                let total = movie_dirs.len();
                let _ = tx.send(PipelineEvent::Started {
                    total_movies: total,
                    label: format!("Batch Queue ({} folders)", total),
                });
                let _ = tx.send(PipelineEvent::LogLine(format!(
                    "=== BATCH QUEUE MODE: Found {} movie folders in '{}' ===",
                    total,
                    queue_dir.display()
                )));

                let mut reports = Vec::with_capacity(total);
                for (i, m_dir) in movie_dirs.iter().enumerate() {
                    if cancel_flag.load(Ordering::Relaxed) {
                        let _ = tx.send(PipelineEvent::Cancelled);
                        return;
                    }
                    let _ = tx.send(PipelineEvent::LogLine(format!(
                        "\n[{}/{}] Processing folder: {}",
                        i + 1,
                        total,
                        m_dir.display()
                    )));
                    match process_single_movie_dir(
                        m_dir,
                        &config,
                        &tools,
                        no_render,
                        &cancel_flag,
                        &tx,
                    ) {
                        Ok(rep) => {
                            let _ = tx.send(PipelineEvent::MovieCompleted(rep.clone()));
                            reports.push(rep);
                        }
                        Err(e) => {
                            let _ = tx.send(PipelineEvent::LogLine(format!(
                                "  [ERROR] Failed processing '{}': {}",
                                m_dir.display(),
                                e
                            )));
                        }
                    }
                }
                let _ = tx.send(PipelineEvent::BatchCompleted {
                    reports,
                    elapsed_secs: start_all.elapsed().as_secs_f64(),
                });
            }
            JobTarget::Directory(dir) => {
                // Auto-check if `dir` is actually a parent batch queue folder
                if !dir.join("source").is_dir() && !dir.join("tosync").is_dir() {
                    let (r, _) = auto_discover_inputs(&dir);
                    if r.is_none() {
                        let sub_movies = discover_batch_queue_dirs(
                            &dir,
                            config.convert_main || config.stereo_only,
                        );
                        if !sub_movies.is_empty() {
                            spawn_pipeline_job(
                                JobTarget::BatchQueue(dir),
                                config,
                                no_render,
                                cancel_flag,
                                tx,
                            );
                            return;
                        }
                    }
                }

                let _ = tx.send(PipelineEvent::Started {
                    total_movies: 1,
                    label: dir.display().to_string(),
                });
                match process_single_movie_dir(&dir, &config, &tools, no_render, &cancel_flag, &tx)
                {
                    Ok(rep) => {
                        let _ = tx.send(PipelineEvent::MovieCompleted(rep.clone()));
                        let _ = tx.send(PipelineEvent::BatchCompleted {
                            reports: vec![rep],
                            elapsed_secs: start_all.elapsed().as_secs_f64(),
                        });
                    }
                    Err(e) => {
                        if cancel_flag.load(Ordering::Relaxed) {
                            let _ = tx.send(PipelineEvent::Cancelled);
                        } else {
                            let _ = tx.send(PipelineEvent::Error(e));
                        }
                    }
                }
            }
            JobTarget::Explicit {
                ref_path,
                ad_paths,
                title,
                out_dir,
                scratch_dir,
                mux_video_path,
            } => {
                let _ = tx.send(PipelineEvent::Started {
                    total_movies: 1,
                    label: ref_path.display().to_string(),
                });
                match process_explicit_inputs(
                    &ref_path,
                    &ad_paths,
                    title.as_deref(),
                    out_dir.as_deref(),
                    scratch_dir.as_deref(),
                    mux_video_path.as_deref(),
                    None,
                    &config,
                    &tools,
                    no_render,
                    &cancel_flag,
                    &tx,
                ) {
                    Ok(rep) => {
                        let _ = tx.send(PipelineEvent::MovieCompleted(rep.clone()));
                        let _ = tx.send(PipelineEvent::BatchCompleted {
                            reports: vec![rep],
                            elapsed_secs: start_all.elapsed().as_secs_f64(),
                        });
                    }
                    Err(e) => {
                        if cancel_flag.load(Ordering::Relaxed) {
                            let _ = tx.send(PipelineEvent::Cancelled);
                        } else {
                            let _ = tx.send(PipelineEvent::Error(e));
                        }
                    }
                }
            }
        }
    });
}

pub fn process_single_movie_dir(
    target_dir: &Path,
    config: &AppConfig,
    tools: &ToolPaths,
    no_render: bool,
    cancel_flag: &AtomicBool,
    tx: &Sender<PipelineEvent>,
) -> Result<MovieJobReport, String> {
    let (auto_ref, auto_ads) = auto_discover_inputs(target_dir);
    let Some(ref_path) = auto_ref else {
        return Err(format!(
            "Reference file not found in '{}'",
            target_dir.display()
        ));
    };
    process_explicit_inputs(
        &ref_path,
        &auto_ads,
        None,
        None,
        None,
        None,
        Some(target_dir),
        config,
        tools,
        no_render,
        cancel_flag,
        tx,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedTrackSync {
    offset: f64,
    atempo: f64,
    confidence: f64,
    method: String,
    is_pal: bool,
    #[serde(default)]
    speed_stretch: Option<f64>,
    #[serde(default)]
    warp_plan: Option<WarpPlan>,
}

#[allow(clippy::too_many_arguments)]
pub fn process_explicit_inputs(
    ref_path: &Path,
    ad_inputs: &[(String, PathBuf)],
    explicit_title: Option<&str>,
    explicit_out_dir: Option<&Path>,
    explicit_scratch_dir: Option<&Path>,
    mux_video_path: Option<&Path>,
    target_dir: Option<&Path>,
    config: &AppConfig,
    tools: &ToolPaths,
    no_render: bool,
    cancel_flag: &AtomicBool,
    tx: &Sender<PipelineEvent>,
) -> Result<MovieJobReport, String> {
    let job_start = Instant::now();
    if !ref_path.exists() {
        return Err(format!("Reference file not found: '{}'", ref_path.display()));
    }

    let convert_main = config.convert_main || config.stereo_only;
    if ad_inputs.is_empty() && !convert_main {
        return Err(format!(
            "No Audio Description tracks provided or found in '{}'",
            target_dir.unwrap_or(ref_path).display()
        ));
    }

    let ref_abs = fs::canonicalize(ref_path).unwrap_or_else(|_| ref_path.to_path_buf());
    let ref_parent = ref_abs.parent().unwrap_or_else(|| Path::new("."));
    let parent_name = ref_parent
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let project_dir = if matches!(
        parent_name.as_str(),
        "source" | "tosync" | "output" | "scratch"
    ) {
        ref_parent
            .parent()
            .unwrap_or(ref_parent)
            .to_path_buf()
    } else if let Some(td) = target_dir {
        td.to_path_buf()
    } else {
        ref_parent.to_path_buf()
    };

    let out_dir = if let Some(od) = explicit_out_dir {
        od.to_path_buf()
    } else if !config.custom_output_dir.trim().is_empty() {
        PathBuf::from(config.custom_output_dir.trim())
    } else {
        project_dir.join("output")
    };

    let scratch_dir = if let Some(sd) = explicit_scratch_dir {
        sd.to_path_buf()
    } else {
        project_dir.join("scratch")
    };

    let mut log_lines: Vec<String> = Vec::new();
    let mut emit_log = |msg: String| {
        let _ = tx.send(PipelineEvent::LogLine(msg.clone()));
        log_lines.push(msg);
    };

    if config.clean_scratch && scratch_dir.exists() {
        emit_log(format!(
            "  [CLEAN] Wiping scratch directory for clean run: {}",
            scratch_dir.display()
        ));
        let _ = fs::remove_dir_all(&scratch_dir);
    }

    fs::create_dir_all(&out_dir)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;
    fs::create_dir_all(&scratch_dir)
        .map_err(|e| format!("Failed to create scratch directory: {}", e))?;

    let base_title = if let Some(t) = explicit_title {
        t.to_string()
    } else {
        let p_name = project_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if !p_name.is_empty() && !p_name.ends_with(':') {
            p_name.to_string()
        } else {
            ref_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Movie")
                .to_string()
        }
    };

    let (select_stream_probe, ref_stream_map) = format_stream_selector(&config.ref_stream);
    let ref_info = get_audio_info(&tools.ffprobe, ref_path, &select_stream_probe)?;
    let target_duration = ref_info.duration;
    let channels = ref_info.channels;
    let ref_sr = ref_info.sample_rate;

    emit_log("================================================================================".to_string());
    emit_log(format!("Reference Audio : {}", ref_path.display()));
    emit_log(format!(
        "Stream Selection: {} ({} Channels @ {} Hz | Duration: {:.2}s)",
        ref_stream_map, channels, ref_sr, target_duration
    ));
    emit_log(format!("Movie Title     : {}", base_title));
    emit_log(format!("Project Dir     : {}", project_dir.display()));
    emit_log(format!("Scratch Dir     : {}", scratch_dir.display()));
    emit_log(format!("Output Dir      : {}", out_dir.display()));
    emit_log(format!("AD Tracks Count : {}", ad_inputs.len()));
    for (name, p) in ad_inputs {
        emit_log(format!(
            "  - [{}]: {}",
            name,
            p.file_name().and_then(|s| s.to_str()).unwrap_or("")
        ));
    }
    if config.stereo_only {
        emit_log("Stereo Only     : [OK] YES (Downmix to True 2.0 Stereo AAC, no surround output)".to_string());
    } else if config.convert_main {
        emit_log("Convert Main    : [OK] YES (Surround AC3 + True 2.0 Stereo AAC + Night Mode AAC)".to_string());
    }
    if config.mux_final_mkv {
        emit_log("Mux Final MKV   : [OK] YES (Video + Stereo Audio + English Subtitles)".to_string());
    }
    emit_log(format!("Sync Mode       : {}", config.sync_mode.label()));
    emit_log(format!("PAL Mode        : {}", config.pal_mode.label()));
    if config.wav_output {
        emit_log("Audio Format    : 16-bit 48kHz PCM WAV (--wav)".to_string());
    }
    if let Some(psec) = config.preview_sec {
        emit_log(format!("Preview Mode    : ON ({:.0} seconds)", psec));
    }
    emit_log(format!(
        "Overwrite Mode  : {}",
        if config.overwrite {
            "ENABLED (--overwrite)"
        } else {
            "[OK] DISABLED (preserve existing files)"
        }
    ));
    emit_log(format!("FFmpeg Threads  : {}", config.threads));
    emit_log("================================================================================".to_string());

    let render_duration = config
        .preview_sec
        .map(|p| p.min(target_duration))
        .unwrap_or(target_duration);
    let suffix = if config.preview_sec.is_some() {
        format!(" (Preview {}s)", render_duration as u64)
    } else {
        String::new()
    };
    let ext = if config.wav_output { ".wav" } else { ".m4a" };

    // Pre-check if all deliverables already exist
    let mut expected_outputs = Vec::new();
    if convert_main {
        if channels >= 6 && !config.stereo_only {
            expected_outputs.push(out_dir.join(format!("{} (Surround){}.ac3", base_title, suffix)));
        }
        expected_outputs.push(out_dir.join(format!("{} (Stereo){}{}", base_title, suffix, ext)));
        if config.generate_night_mode && !config.stereo_only {
            expected_outputs
                .push(out_dir.join(format!("{} (Night Mode){}{}", base_title, suffix, ext)));
        }
    }
    for (name, _) in ad_inputs {
        let label_prefix = if ad_inputs.len() == 1 {
            format!("{}-AD", base_title)
        } else {
            format!("{}-AD [{}]", base_title, name)
        };
        expected_outputs
            .push(out_dir.join(format!("{} (Stereo){}{}", label_prefix, suffix, ext)));
        if config.generate_night_mode && !config.stereo_only {
            expected_outputs
                .push(out_dir.join(format!("{} (Night Mode){}{}", label_prefix, suffix, ext)));
        }
    }

    let all_exist = !expected_outputs.is_empty()
        && expected_outputs.iter().all(|p| {
            p.exists() && fs::metadata(p).map(|m| m.len()).unwrap_or(0) > 1000
        });
    if all_exist && !config.overwrite && !no_render {
        emit_log(format!(
            "\n  [SKIP] All {} requested deliverables already exist in output/:",
            expected_outputs.len()
        ));
        for exp in &expected_outputs {
            let size_mb =
                (fs::metadata(exp).map(|m| m.len()).unwrap_or(0) as f64) / (1024.0 * 1024.0);
            emit_log(format!(
                "    [OK] {} ({:.1} MB)",
                exp.file_name().and_then(|s| s.to_str()).unwrap_or(""),
                size_mb
            ));
        }
        emit_log("  Skipping correlation analysis and rendering (use --overwrite to force).".to_string());
        emit_log("================================================================================\n".to_string());

        let log_text = log_lines.join("\n");
        maybe_save_log_file(config, &project_dir, &base_title, &log_text);
        return Ok(MovieJobReport {
            base_title,
            ref_path: ref_path.to_path_buf(),
            ref_stream_map,
            ref_channels: channels,
            ref_sample_rate: ref_sr,
            target_duration,
            project_dir,
            out_dir,
            scratch_dir,
            main_deliverables: expected_outputs,
            ad_results: Vec::new(),
            final_mkv: None,
            elapsed_secs: job_start.elapsed().as_secs_f64(),
            skipped: true,
            log_text,
        });
    }

    // 1. Acoustic Cross-Correlation & Native ADSync Viterbi+PCHIP Warp Alignment
    let sync_cache_path = scratch_dir.join("sync_params.json");
    let mut cached_map: BTreeMap<String, CachedTrackSync> = if sync_cache_path.exists() {
        fs::read_to_string(&sync_cache_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        BTreeMap::new()
    };

    let mut ad_results = Vec::with_capacity(ad_inputs.len());
    let mut base_8k_samples: Option<Vec<f32>> = None;

    if !ad_inputs.is_empty() {
        emit_log("\n=== COMPUTING ACOUSTIC ALIGNMENT PARAMETERS ===".to_string());
    }

    for (name, ad_path) in ad_inputs {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err("Cancelled".to_string());
        }

        let ad_info = get_audio_info(&tools.ffprobe, ad_path, "a:0")?;
        let ad_sr = ad_info.sample_rate;
        let ad_dur = ad_info.duration;

        // Check scratch cache first
        if let Some(c_entry) = cached_map.get(name) {
            let want_warp = config.sync_mode == SyncMode::ForceWarp;
            if (!want_warp || c_entry.method == "warp") && !config.clean_scratch {
                emit_log(format!(
                    "  [CACHE] Reusing cached sync parameters from scratch for [{}]: offset={:+.3}s | conf={:.1}% | method={}",
                    name,
                    c_entry.offset,
                    c_entry.confidence * 100.0,
                    c_entry.method.to_ascii_uppercase()
                ));
                let cuts = c_entry
                    .warp_plan
                    .as_ref()
                    .map(|p| p.cuts.clone())
                    .unwrap_or_default();
                let pts = c_entry
                    .warp_plan
                    .as_ref()
                    .map(|p| p.points.clone())
                    .unwrap_or_default();
                ad_results.push(TrackSyncResult {
                    name: name.clone(),
                    source_path: ad_path.clone(),
                    method: c_entry.method.clone(),
                    offset: c_entry.offset,
                    atempo: c_entry.atempo,
                    confidence: c_entry.confidence,
                    is_pal: c_entry.is_pal,
                    speed_stretch: c_entry.speed_stretch,
                    winner_anchors: config.anchors,
                    total_anchors: config.anchors,
                    runner_ups: Vec::new(),
                    anchor_measurements: Vec::new(),
                    warp_points: pts,
                    warp_cuts: cuts,
                    filled_gaps: Vec::new(),
                    deliverables: Vec::new(),
                });
                continue;
            }
        }

        // Lazy decode Reference 8kHz mono WAV once
        if base_8k_samples.is_none() {
            let _ = tx.send(PipelineEvent::Progress {
                step_label: "Pre-decoding Reference 8kHz analysis audio...".to_string(),
                completed: 0,
                total: 100,
            });
            emit_log(format!(
                "  [REF] Pre-decoding stream to 8kHz mono analysis WAV: {}...",
                ref_path.file_name().and_then(|s| s.to_str()).unwrap_or("")
            ));
            let ref_wav = predecode_to_mono_wav(
                &tools.ffmpeg,
                ref_path,
                &scratch_dir,
                "REF",
                8000,
                Some(&ref_stream_map),
                false,
                ref_sr,
                config.threads,
            )?;
            let (samples, _) = read_wav_mono_f32(&ref_wav)?;
            base_8k_samples = Some(samples);
        }
        let ref_samples = base_8k_samples.as_ref().unwrap();

        let lname = name.to_ascii_lowercase();
        let mut is_pal_track = match config.pal_mode {
            PalMode::ForcePal => true,
            PalMode::ForceNative => false,
            PalMode::AutoDetect => {
                lname.contains("uk") || lname.contains("pal") || lname.contains("25fps")
            }
        };
        let mut detected_stretch: Option<f64> = None;
        let mut baseline_fp = None;

        // Automatic Acoustic Landmark Speed / PAL Probing
        if !is_pal_track
            && config.pal_mode == PalMode::AutoDetect
            && config.speed_detect
        {
            let _ = tx.send(PipelineEvent::Progress {
                step_label: format!("[{}] Probing acoustic landmarks for PAL speed shift...", name),
                completed: 10,
                total: 100,
            });
            let probe_wav = predecode_to_mono_wav(
                &tools.ffmpeg,
                ad_path,
                &scratch_dir,
                &format!("{}_rawprobe", name),
                8000,
                None,
                false,
                ad_sr,
                config.threads,
            )?;
            let (probe_samples, _) = read_wav_mono_f32(&probe_wav)?;
            let (det_pal, ratio, matches, fp_res) =
                detect_speed_pal(ref_samples, &probe_samples, 8000);
            baseline_fp = Some(fp_res);
            if det_pal {
                emit_log(format!(
                    "  [SPEED PROBE] Acoustic landmark probe detected PAL speed transfer (stretch x{:.4}, {} matches) for [{}]!",
                    ratio, matches, name
                ));
                is_pal_track = true;
                detected_stretch = Some(ratio);
            }
        }

        // Decode 8kHz mono AD analysis WAV (with PAL clock correction if PAL)
        emit_log(format!(
            "\nAnalyzing acoustic correlation for [{}] (PAL={} @ {} Hz)...",
            name, is_pal_track, ad_sr
        ));
        let ad_8k_wav = predecode_to_mono_wav(
            &tools.ffmpeg,
            ad_path,
            &scratch_dir,
            name,
            8000,
            None,
            is_pal_track,
            ad_sr,
            config.threads,
        )?;
        let (ad_samples, _) = read_wav_mono_f32(&ad_8k_wav)?;

        let mut method = "cross_correlation".to_string();
        let mut winner_offset = 0.0_f64;
        let mut winner_peak = 1.0_f64;
        let mut winner_count = 0usize;
        let mut measurements = Vec::new();
        let mut candidates = Vec::new();

        if config.sync_mode == SyncMode::ForceWarp {
            emit_log(format!(
                "  [{}] Forced Viterbi+PCHIP Warp mode enabled (bypassing 100-anchor global offset search).",
                name
            ));
            method = "warp".to_string();
        } else {
            measurements = evaluate_anchor_correlations(
                ref_samples,
                &ad_samples,
                8000,
                target_duration,
                config.anchors,
                config.multiband,
                |done, tot, _| {
                    let _ = tx.send(PipelineEvent::Progress {
                        step_label: format!(
                            "[{}] Evaluating 3-Band FFT Correlation Anchors ({}/{})",
                            name, done, tot
                        ),
                        completed: done,
                        total: tot.max(1),
                    });
                },
            );

            candidates = find_offset_candidates(&measurements, config.min_confidence, 0.03);
            let top = candidates[0];
            winner_offset = if top.median_offset.abs() < 0.008 {
                0.0
            } else {
                top.median_offset
            };
            winner_peak = top.max_peak;
            winner_count = top.count;

            let min_quorum = ((measurements.len() as f64) * 0.05).floor() as usize;
            let min_quorum = min_quorum.max(5);
            let min_quorum_hi = (((measurements.len() as f64) * 0.08).floor() as usize).max(8);

            let has_insufficient_quorum = (winner_count < min_quorum && winner_peak < 0.40)
                || (winner_count < min_quorum_hi && winner_peak < 0.25);

            let mut has_drifting_clusters = false;
            for r in candidates.iter().skip(1).take(4) {
                if r.max_peak >= config.min_confidence
                    && (r.median_offset - winner_offset).abs() > 0.20
                    && (r.count >= 2 || r.max_peak >= 0.25)
                {
                    has_drifting_clusters = true;
                    break;
                }
            }

            if (winner_peak < config.min_confidence
                || has_insufficient_quorum
                || has_drifting_clusters)
                && config.sync_mode != SyncMode::OffsetOnly
            {
                if has_drifting_clusters {
                    emit_log("  [WARNING] Multiple distinct offset clusters detected across timeline (drifting offsets / reel changes / structural cuts).".to_string());
                } else if has_insufficient_quorum {
                    emit_log(format!(
                        "  [WARNING] Insufficient anchor consensus: Only {}/{} anchors agreed on winner offset with {:.1}% peak confidence (< {} anchors required).",
                        winner_count,
                        measurements.len(),
                        winner_peak * 100.0,
                        min_quorum
                    ));
                } else {
                    emit_log(format!(
                        "  [WARNING] Max correlation confidence is only {:.1}% (< {:.1}%).",
                        winner_peak * 100.0,
                        config.min_confidence * 100.0
                    ));
                }
                emit_log(format!(
                    "  [FALLBACK] Engaging Native ADSync Viterbi Lattice DP + Monotone PCHIP Warp for [{}]...",
                    name
                ));
                method = "warp".to_string();
            }

            emit_log(format!(
                "  => [OK] CONSENSUS OFFSET for [{}]: {:+.3}s (Max Confidence: {:.1}% | {}/{} anchors | Method: {})",
                name,
                winner_offset,
                winner_peak * 100.0,
                winner_count,
                measurements.len(),
                method.to_ascii_uppercase()
            ));

            let mut r_idx = 1;
            for r in candidates.iter().skip(1) {
                if (r.median_offset - winner_offset).abs() > 0.01 {
                    emit_log(format!(
                        "     Runner-up #{:<6}: {:+.3}s (Max Confidence: {:.1}% | {}/{} anchors)",
                        r_idx,
                        r.median_offset,
                        r.max_peak * 100.0,
                        r.count,
                        measurements.len()
                    ));
                    r_idx += 1;
                    if r_idx > 2 {
                        break;
                    }
                }
            }
        }

        let mut warp_plan: Option<WarpPlan> = None;
        if method == "warp" {
            let _ = tx.send(PipelineEvent::Progress {
                step_label: format!("[{}] Building Landmark Fingerprint & Candidate Lattice...", name),
                completed: 0,
                total: 100,
            });
            let fp = if is_pal_track || baseline_fp.is_none() {
                fingerprint_align(ref_samples, &ad_samples, 8000)
            } else {
                baseline_fp.unwrap()
            };

            let search_radius = if fp.is_strong() && fp.spans.len() >= 2 {
                let mut max_jump = 0.0_f64;
                for pair in fp.spans.windows(2) {
                    max_jump = max_jump.max((pair[1].offset - pair[0].offset).abs());
                }
                (max_jump + 15.0).clamp(30.0, 90.0)
            } else {
                let dur_gap = (target_duration - ad_dur).abs();
                (dur_gap + 45.0).clamp(30.0, 180.0)
            };

            emit_log(format!(
                "  [WARP] Building candidate lattice (search radius +/-{:.0}s, {} fingerprint spans)...",
                search_radius,
                fp.spans.len()
            ));

            let lattice = build_candidate_lattice(
                ref_samples,
                &ad_samples,
                8000,
                8.0,
                2.0,
                search_radius,
                winner_offset,
                Some(&fp),
                5,
                config.multiband,
                |done, tot| {
                    let _ = tx.send(PipelineEvent::Progress {
                        step_label: format!(
                            "[{}] Scanning Candidate Lattice ({}/{})",
                            name, done, tot
                        ),
                        completed: done,
                        total: tot.max(1),
                    });
                },
            );

            let (raw_pts, _cost) = decode_warp_path(&lattice, winner_offset, Some(&fp));
            let eff_ad_dur = (ad_samples.len() as f64) / 8000.0;
            let plan = fit_and_refine_warp(
                raw_pts,
                eff_ad_dur,
                target_duration,
                &ad_samples,
                8000,
                config.discontinuity_threshold,
                0.60,
            );
            winner_offset = plan.global_offset;
            winner_peak = plan.confidence;
            emit_log(format!(
                "  [WARP] Fitted {} continuous PCHIP segment(s) across {} warp points (Mean Confidence: {:.1}%)",
                plan.splines.len(),
                plan.points.len(),
                plan.confidence * 100.0
            ));
            warp_plan = Some(plan);
        }

        cached_map.insert(
            name.clone(),
            CachedTrackSync {
                offset: winner_offset,
                atempo: 1.0,
                confidence: winner_peak,
                method: method.clone(),
                is_pal: is_pal_track,
                speed_stretch: detected_stretch,
                warp_plan: warp_plan.clone(),
            },
        );

        let warp_points = warp_plan
            .as_ref()
            .map(|p| p.points.clone())
            .unwrap_or_default();
        let warp_cuts = warp_plan
            .as_ref()
            .map(|p| p.cuts.clone())
            .unwrap_or_default();

        ad_results.push(TrackSyncResult {
            name: name.clone(),
            source_path: ad_path.clone(),
            method,
            offset: winner_offset,
            atempo: 1.0,
            confidence: winner_peak,
            is_pal: is_pal_track,
            speed_stretch: detected_stretch,
            winner_anchors: winner_count,
            total_anchors: measurements.len(),
            runner_ups: candidates.into_iter().skip(1).take(3).collect(),
            anchor_measurements: measurements,
            warp_points,
            warp_cuts,
            filled_gaps: Vec::new(),
            deliverables: Vec::new(),
        });
    }

    // Free 8kHz reference buffer before HQ 48kHz rendering
    drop(base_8k_samples);

    // Save sync_params.json cache
    if let Ok(json_str) = serde_json::to_string_pretty(&cached_map) {
        let _ = fs::write(&sync_cache_path, json_str);
    }

    let mut main_deliverables = Vec::new();
    let mut final_mkv_out = None;

    if !no_render {
        // 2. Render Main Audio Tracks if requested
        if convert_main {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            let rendered_main = render_main_tracks(
                &tools.ffmpeg,
                ref_path,
                &base_title,
                &out_dir,
                &scratch_dir,
                channels,
                Some(&ref_stream_map),
                config,
                &mut emit_log,
                tx,
            )?;
            main_deliverables = rendered_main;
        }

        // 3. Render Synced AD Tracks
        if !ad_results.is_empty() {
            emit_log("\n=== RENDERING AUDIO DESCRIPTION TRACKS ===".to_string());
            let total_ads = ad_results.len();
            for res in &mut ad_results {
                if cancel_flag.load(Ordering::Relaxed) {
                    return Err("Cancelled".to_string());
                }
                let plan_opt = cached_map
                    .get(&res.name)
                    .and_then(|c| c.warp_plan.as_ref());
                render_single_ad_track(
                    &tools.ffmpeg,
                    &tools.ffprobe,
                    ref_path,
                    &base_title,
                    res,
                    plan_opt,
                    total_ads,
                    target_duration,
                    channels,
                    &out_dir,
                    &scratch_dir,
                    Some(&ref_stream_map),
                    mux_video_path,
                    config,
                    &mut emit_log,
                    tx,
                )?;
            }
        }

        // 4. Multiplex Final MKV if requested
        if config.mux_final_mkv
            && ref_path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("mkv"))
                .unwrap_or(false)
        {
            let main_stereo = out_dir.join(format!("{} (Stereo){}{}", base_title, suffix, ext));
            let mut ad_stereos = Vec::new();
            for res in &ad_results {
                let label_prefix = if ad_results.len() == 1 {
                    format!("{}-AD", base_title)
                } else {
                    format!("{}-AD [{}]", base_title, res.name)
                };
                let ad_s = out_dir.join(format!("{} (Stereo){}{}", label_prefix, suffix, ext));
                if ad_s.exists() {
                    ad_stereos.push(ad_s);
                }
            }

            if main_stereo.exists() && !ad_stereos.is_empty() {
                let mkv_dest_dir = if !config.custom_mkv_dir.trim().is_empty() {
                    PathBuf::from(config.custom_mkv_dir.trim())
                } else {
                    out_dir.clone()
                };
                let same_dir = fs::canonicalize(&mkv_dest_dir).ok()
                    == ref_path.parent().and_then(|p| fs::canonicalize(p).ok());
                let suffix_mkv = if same_dir { " (Synced).mkv" } else { ".mkv" };
                let target_mkv = mkv_dest_dir.join(format!("{}{}", base_title, suffix_mkv));

                if target_mkv.exists()
                    && fs::metadata(&target_mkv).map(|m| m.len()).unwrap_or(0) > 1000
                    && !config.overwrite
                {
                    emit_log(format!(
                        "  [SKIP] Final MKV already exists: {}",
                        target_mkv.display()
                    ));
                    final_mkv_out = Some(target_mkv);
                } else if let Some(ref mkvmerge_bin) = tools.mkvmerge {
                    emit_log("\n=== MULTIPLEXING FINAL MKV ===".to_string());
                    emit_log(format!("  Output: {}", target_mkv.display()));
                    mux_final_mkv(mkvmerge_bin, ref_path, &main_stereo, &ad_stereos, &target_mkv)?;
                    let sz_mb = (fs::metadata(&target_mkv).map(|m| m.len()).unwrap_or(0) as f64)
                        / (1024.0 * 1024.0);
                    emit_log(format!(
                        "  [SUCCESS] Saved Final MKV: {} ({:.1} MB)",
                        target_mkv.display(),
                        sz_mb
                    ));
                    final_mkv_out = Some(target_mkv);
                } else {
                    emit_log("  [ERROR] mkvmerge binary not found for multiplexing!".to_string());
                }
            }
        }
    }

    // Cleanup temporary scratch WAVs unless keep_scratch is enabled
    if !config.keep_scratch {
        if let Ok(entries) = fs::read_dir(&scratch_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if fname.ends_with("_mono8k.wav")
                    || fname.ends_with("_temp.wav")
                    || fname.ends_with("_raw.wav")
                    || fname == "ref_master_stereo_48k.wav"
                {
                    let _ = fs::remove_file(p);
                }
            }
        }
    }

    // 5. Final Sync Summary
    emit_log("\n================================================================================".to_string());
    emit_log(format!(
        "[SUCCESS]: Processing completed for: {}",
        base_title
    ));
    emit_log(format!("Deliverables located in: {}", out_dir.display()));
    emit_log("--------------------------------------------------------------------------------".to_string());
    emit_log("FINAL SYNC SUMMARY:".to_string());

    if convert_main {
        if config.stereo_only {
            emit_log("  Main Audio     : [OK] Processed (Stereo)".to_string());
        } else {
            emit_log("  Main Audio     : [OK] Processed (Surround / Stereo / Night Mode)".to_string());
        }
    }

    for res in &ad_results {
        emit_log(format!("\n  AD Track [{}]", res.name));
        let m_str = if res.method == "cross_correlation" {
            "CROSS_CORRELATION"
        } else {
            "VITERBI + PCHIP WARP"
        };
        emit_log(format!("    Sync Method  : {}", m_str));
        emit_log(format!("    Global Offset: {:+.3}s", res.offset));
        emit_log(format!(
            "    PAL Slowdown : {}",
            if res.is_pal { "Yes (25fps -> 23.976fps)" } else { "No (1.0x Native)" }
        ));
        emit_log(format!("    Confidence   : {:.1}%", res.confidence * 100.0));

        if !res.filled_gaps.is_empty() {
            let total_gap_sec: f64 = res.filled_gaps.iter().map(|g| g.duration_sec).sum();
            emit_log(format!(
                "    [GAP FILL]   : Spliced Reference Master Audio into {} excised scene void(s) ({:.1}s total)",
                res.filled_gaps.len(),
                total_gap_sec
            ));
        }

        if !res.warp_cuts.is_empty() {
            emit_log(format!(
                "    Warp Segments: {} continuous blocks",
                res.warp_cuts.len()
            ));
            let sig_cuts: Vec<&WarpCut> = res
                .warp_cuts
                .iter()
                .filter(|c| c.is_initial || c.jump.abs() > 0.03)
                .collect();
            if sig_cuts.len() > 1 {
                emit_log("    Double-Check Cut Times (where AD track shifts):".to_string());
                for c in sig_cuts.iter().take(25) {
                    let t_str = format_time_code(c.vid_start);
                    if c.is_initial {
                        emit_log(format!(
                            "      - Seg 1 starts at {} (initial offset: {:+.3}s)",
                            t_str, c.offset
                        ));
                    } else {
                        emit_log(format!(
                            "      - Seg {} jumps at {} (offset: {:+.3}s, shift: {:+.3}s)",
                            c.index, t_str, c.offset, c.jump
                        ));
                    }
                }
                if sig_cuts.len() > 25 {
                    emit_log(format!(
                        "      ... and {} more cut transition(s)",
                        sig_cuts.len() - 25
                    ));
                }
            }
        }
    }
    emit_log("================================================================================\n".to_string());

    let log_text = log_lines.join("\n");
    maybe_save_log_file(config, &project_dir, &base_title, &log_text);

    Ok(MovieJobReport {
        base_title,
        ref_path: ref_path.to_path_buf(),
        ref_stream_map,
        ref_channels: channels,
        ref_sample_rate: ref_sr,
        target_duration,
        project_dir,
        out_dir,
        scratch_dir,
        main_deliverables,
        ad_results,
        final_mkv: final_mkv_out,
        elapsed_secs: job_start.elapsed().as_secs_f64(),
        skipped: false,
        log_text,
    })
}

fn maybe_save_log_file(config: &AppConfig, project_dir: &Path, base_title: &str, log_text: &str) {
    if !config.log_to_file {
        return;
    }
    let log_dir = if !config.log_dir.trim().is_empty() {
        PathBuf::from(config.log_dir.trim())
    } else {
        project_dir.to_path_buf()
    };
    let _ = fs::create_dir_all(&log_dir);
    let clean_title: String = base_title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(35)
        .collect();
    let ts = Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("{}_{}.log", ts, clean_title.trim_matches('_'));
    let _ = fs::write(log_dir.join(filename), log_text);
}

#[allow(clippy::too_many_arguments)]
fn render_main_tracks<F>(
    ffmpeg_bin: &Path,
    ref_path: &Path,
    base_title: &str,
    out_dir: &Path,
    scratch_dir: &Path,
    channels: u32,
    stream_map: Option<&str>,
    config: &AppConfig,
    emit_log: &mut F,
    tx: &Sender<PipelineEvent>,
) -> Result<Vec<PathBuf>, String>
where
    F: FnMut(String),
{
    emit_log("\n=== RENDERING MAIN AUDIO TRACKS ===".to_string());
    let suffix = if let Some(psec) = config.preview_sec {
        format!(" (Preview {}s)", psec as u64)
    } else {
        String::new()
    };

    let mut saved = Vec::new();

    // 1. Surround 5.1 AC3 (if 6+ channels and not stereo_only)
    if channels >= 6 && !config.stereo_only {
        let out_surround = out_dir.join(format!("{} (Surround){}.ac3", base_title, suffix));
        if out_surround.exists()
            && fs::metadata(&out_surround).map(|m| m.len()).unwrap_or(0) > 1000
            && !config.overwrite
        {
            emit_log(format!(
                "  [SKIP] 5.1 Surround AC3 already exists: {}",
                out_surround.file_name().and_then(|s| s.to_str()).unwrap_or("")
            ));
            saved.push(out_surround);
        } else {
            let _ = tx.send(PipelineEvent::Progress {
                step_label: "Encoding Main 5.1 Surround AC3 (640k)...".to_string(),
                completed: 25,
                total: 100,
            });
            emit_log("  [1/3] Encoding 5.1 Surround AC3 (640k)...".to_string());
            let mut cmd = Command::new(ffmpeg_bin);
            apply_no_window(&mut cmd);
            cmd.args([
                "-y",
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "fatal",
                "-threads",
                &config.threads.to_string(),
                "-i",
            ])
            .arg(ref_path);
            if let Some(smap) = stream_map {
                cmd.args(["-map", smap]);
            }
            if let Some(psec) = config.preview_sec {
                cmd.args(["-t", &format!("{:.3}", psec)]);
            }
            cmd.args(["-c:a", "ac3", "-b:a", "640k", "-ar", "48000"])
                .arg(&out_surround)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            let res = cmd.output().map_err(|e| e.to_string())?;
            if res.status.success() {
                emit_log(format!("  [SUCCESS] Saved: {}", out_surround.display()));
                saved.push(out_surround);
            } else {
                emit_log(format!(
                    "  [ERROR] Surround AC3 failed: {}",
                    String::from_utf8_lossy(&res.stderr).trim()
                ));
            }
        }
    }

    let ext = if config.wav_output { ".wav" } else { ".m4a" };
    let codec_label = if config.wav_output {
        "WAV PCM"
    } else {
        "AAC 2.0"
    };
    let out_stereo = out_dir.join(format!("{} (Stereo){}{}", base_title, suffix, ext));
    let out_night = out_dir.join(format!("{} (Night Mode){}{}", base_title, suffix, ext));

    let stereo_exists = out_stereo.exists()
        && fs::metadata(&out_stereo).map(|m| m.len()).unwrap_or(0) > 1000;
    let night_exists = !config.generate_night_mode
        || (out_night.exists()
            && fs::metadata(&out_night).map(|m| m.len()).unwrap_or(0) > 1000);

    if stereo_exists && night_exists && !config.overwrite {
        emit_log(format!(
            "  [SKIP] Main Stereo & Night Mode {} already exist in output/.",
            codec_label
        ));
        saved.push(out_stereo);
        if config.generate_night_mode {
            saved.push(out_night);
        }
        return Ok(saved);
    }

    // 2. Extract intermediate stereo WAV in scratch (ITU-R BS.775 downmix)
    let stereo_wav = scratch_dir.join("main_stereo_temp.wav");
    emit_log("  Extracting intermediate True 2.0 Stereo WAV (ITU-R BS.775 downmix)...".to_string());
    let af_str = if channels >= 6 {
        format!("asetpts=PTS-STARTPTS,{},aresample=48000", PAN_STEREO_BS775)
    } else {
        "asetpts=PTS-STARTPTS,aresample=48000".to_string()
    };

    let mut cmd_wav = Command::new(ffmpeg_bin);
    apply_no_window(&mut cmd_wav);
    cmd_wav
        .args([
            "-y",
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "fatal",
            "-threads",
            &config.threads.to_string(),
            "-fflags",
            "+genpts",
            "-i",
        ])
        .arg(ref_path);
    if let Some(smap) = stream_map {
        cmd_wav.args(["-map", smap]);
    }
    if let Some(psec) = config.preview_sec {
        cmd_wav.args(["-t", &format!("{:.3}", psec)]);
    }
    cmd_wav
        .args([
            "-af",
            &af_str,
            "-ac",
            "2",
            "-c:a",
            "pcm_s16le",
            "-rf64",
            "auto",
            "-f",
            "wav",
        ])
        .arg(&stereo_wav)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let res_wav = cmd_wav.output().map_err(|e| e.to_string())?;
    if !res_wav.status.success() {
        return Err(format!(
            "Error extracting intermediate stereo WAV: {}",
            String::from_utf8_lossy(&res_wav.stderr).trim()
        ));
    }

    // 3. Standard Stereo from intermediate WAV
    if stereo_exists && !config.overwrite {
        emit_log(format!(
            "  [SKIP] Main Stereo {} already exists.",
            codec_label
        ));
        saved.push(out_stereo);
    } else {
        let _ = tx.send(PipelineEvent::Progress {
            step_label: format!("Encoding Main Stereo {} (alimiter)...", codec_label),
            completed: 55,
            total: 100,
        });
        emit_log(format!(
            "  Encoding Main Stereo {} (alimiter, full dynamic range)...",
            codec_label
        ));
        encode_deliverable_from_wav(
            ffmpeg_bin,
            &stereo_wav,
            &out_stereo,
            "alimiter=level_in=1:level_out=0.98:limit=0.98",
            config.wav_output,
            config.threads,
        )?;
        emit_log(format!("  [SUCCESS] Saved: {}", out_stereo.display()));
        saved.push(out_stereo);
    }

    // 4. Night Mode from intermediate WAV
    if config.generate_night_mode {
        if night_exists && !config.overwrite {
            emit_log(format!(
                "  [SKIP] Main Night Mode {} already exists.",
                codec_label
            ));
            saved.push(out_night);
        } else {
            let _ = tx.send(PipelineEvent::Progress {
                step_label: format!(
                    "Encoding Main Night Mode {} (loudnorm {:.1} LUFS)...",
                    codec_label, config.target_i
                ),
                completed: 80,
                total: 100,
            });
            emit_log(format!(
                "  Encoding Main Night Mode {} (loudnorm {:.1} LUFS)...",
                codec_label, config.target_i
            ));
            let ln_filter = format!(
                "loudnorm=I={:.1}:TP={:.1}:LRA=11",
                config.target_i, config.target_tp
            );
            encode_deliverable_from_wav(
                ffmpeg_bin,
                &stereo_wav,
                &out_night,
                &ln_filter,
                config.wav_output,
                config.threads,
            )?;
            emit_log(format!("  [SUCCESS] Saved: {}", out_night.display()));
            saved.push(out_night);
        }
    }

    if !config.keep_scratch {
        let _ = fs::remove_file(&stereo_wav);
    }

    Ok(saved)
}

#[allow(clippy::too_many_arguments)]
fn render_single_ad_track<F>(
    ffmpeg_bin: &Path,
    ffprobe_bin: &Path,
    ref_path: &Path,
    base_title: &str,
    res: &mut TrackSyncResult,
    warp_plan: Option<&WarpPlan>,
    total_ad_count: usize,
    target_duration: f64,
    ref_channels: u32,
    out_dir: &Path,
    scratch_dir: &Path,
    ref_stream_map: Option<&str>,
    mux_video_path: Option<&Path>,
    config: &AppConfig,
    emit_log: &mut F,
    tx: &Sender<PipelineEvent>,
) -> Result<(), String>
where
    F: FnMut(String),
{
    let render_duration = config
        .preview_sec
        .map(|p| p.min(target_duration))
        .unwrap_or(target_duration);
    let suffix = if config.preview_sec.is_some() {
        format!(" (Preview {}s)", render_duration as u64)
    } else {
        String::new()
    };
    let ext = if config.wav_output { ".wav" } else { ".m4a" };
    let codec_name = if config.wav_output {
        "WAV PCM"
    } else {
        "AAC 2.0"
    };

    let label_prefix = if total_ad_count == 1 {
        format!("{}-AD", base_title)
    } else {
        format!("{}-AD [{}]", base_title, res.name)
    };
    let out_stereo = out_dir.join(format!("{} (Stereo){}{}", label_prefix, suffix, ext));
    let out_night = out_dir.join(format!("{} (Night Mode){}{}", label_prefix, suffix, ext));

    let stereo_exists = out_stereo.exists()
        && fs::metadata(&out_stereo).map(|m| m.len()).unwrap_or(0) > 1000;
    let night_exists = !config.generate_night_mode
        || (out_night.exists()
            && fs::metadata(&out_night).map(|m| m.len()).unwrap_or(0) > 1000);

    if stereo_exists && night_exists && !config.overwrite {
        emit_log(format!(
            "\n  [SKIP] All AD deliverables for [{}] already exist in output/.",
            res.name
        ));
        res.deliverables.push(out_stereo);
        if config.generate_night_mode {
            res.deliverables.push(out_night);
        }
        return Ok(());
    }

    let ad_info = get_audio_info(ffprobe_bin, &res.source_path, "a:0")?;
    let ad_sr = ad_info.sample_rate;
    let temp_wav = scratch_dir.join(format!("{}_synced_temp.wav", res.name));

    if res.method == "warp" {
        if let Some(plan) = warp_plan {
            emit_log(format!(
                "\n--- [WARP MODE: Native Rust Viterbi+PCHIP] Rendering [{}] ---",
                res.name
            ));
            let _ = tx.send(PipelineEvent::Progress {
                step_label: format!("[{}] Decoding 48kHz HQ source audio...", res.name),
                completed: 30,
                total: 100,
            });

            // 1. Decode HQ 48kHz stereo AD WAV (applying exact PAL sample-rate slowdown if PAL)
            let hq_src_wav = scratch_dir.join(format!("{}_hq_src_48k.wav", res.name));
            let mut filters = vec!["asetpts=PTS-STARTPTS".to_string()];
            if res.is_pal {
                let pal_rate = (ad_sr as f64) * (23.976023976 / 25.0);
                filters.push(format!("asetrate={:.8}", pal_rate));
            }
            filters.push("aresample=48000".to_string());

            let mut cmd_hq = Command::new(ffmpeg_bin);
            apply_no_window(&mut cmd_hq);
            cmd_hq
                .args([
                    "-y",
                    "-nostdin",
                    "-hide_banner",
                    "-loglevel",
                    "fatal",
                    "-threads",
                    &config.threads.to_string(),
                    "-fflags",
                    "+genpts",
                    "-i",
                ])
                .arg(&res.source_path)
                .args([
                    "-vn",
                    "-af",
                    &filters.join(","),
                    "-ac",
                    "2",
                    "-c:a",
                    "pcm_s16le",
                    "-rf64",
                    "auto",
                    "-f",
                    "wav",
                ])
                .arg(&hq_src_wav)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            let out_hq = cmd_hq.output().map_err(|e| e.to_string())?;
            if !out_hq.status.success() {
                return Err(format!(
                    "Failed decoding HQ AD WAV: {}",
                    String::from_utf8_lossy(&out_hq.stderr).trim()
                ));
            }

            // 2. Render warped 48kHz stereo WAV in streaming 30-second blocks (< 35 MB RAM!)
            let _ = tx.send(PipelineEvent::Progress {
                step_label: format!("[{}] Streaming Catmull-Rom PCHIP Warp Render...", res.name),
                completed: 50,
                total: 100,
            });
            render_warped_wav_streaming(
                &hq_src_wav,
                &temp_wav,
                plan,
                render_duration,
                config.crossfade_ms,
            )?;
            if !config.keep_scratch {
                let _ = fs::remove_file(&hq_src_wav);
            }

            // 3. Windowed Master Audio Gap Filling (only if explicit_gaps exist!)
            if config.gap_fill && !plan.explicit_gaps.is_empty() {
                let _ = tx.send(PipelineEvent::Progress {
                    step_label: format!(
                        "[{}] Splicing Master Audio into {} excised scene gap(s)...",
                        res.name,
                        plan.explicit_gaps.len()
                    ),
                    completed: 65,
                    total: 100,
                });
                let filled = fill_master_audio_gaps_windowed(
                    ffmpeg_bin,
                    &temp_wav,
                    ref_path,
                    ref_stream_map,
                    ref_channels,
                    scratch_dir,
                    config.threads,
                    &plan.explicit_gaps,
                    config.crossfade_ms,
                )?;
                if !filled.is_empty() {
                    emit_log(format!(
                        "  [GAP FILL] Spliced Level-Matched Reference Master Audio into {} missing scene void(s):",
                        filled.len()
                    ));
                    for g in &filled {
                        let db_s = 20.0 * g.gain_start.log10();
                        let db_e = 20.0 * g.gain_end.log10();
                        emit_log(format!(
                            "    - Excised Scene Void: {} -> {} ({:.1}s) | Matched Gain: {:.2}x ({:+.1}dB) -> {:.2}x ({:+.1}dB)",
                            format_time_code(g.start_sec),
                            format_time_code(g.end_sec),
                            g.duration_sec,
                            g.gain_start,
                            db_s,
                            g.gain_end,
                            db_e
                        ));
                    }
                    res.filled_gaps = filled;
                }
            }
        }
    } else {
        // Standard Cross-Correlation Single Offset Path
        emit_log(format!(
            "  Pre-rendering synced PCM WAV for [{}] (offset={:+.3}s)...",
            res.name, res.offset
        ));
        let mut wav_filters = vec!["asetpts=PTS-STARTPTS".to_string()];
        if res.is_pal {
            let pal_rate = (ad_sr as f64) * (23.976023976 / 25.0);
            wav_filters.push(format!("asetrate={:.8}", pal_rate));
            wav_filters.push("aresample=48000".to_string());
        } else if (res.atempo - 1.0).abs() > 0.00001 {
            wav_filters.push(format!("atempo={:.8}", res.atempo));
        }

        if res.offset > 0.0 {
            wav_filters.push(format!("atrim=start={:.3}", res.offset));
            wav_filters.push("asetpts=PTS-STARTPTS".to_string());
        } else if res.offset < 0.0 {
            let delay_ms = (res.offset.abs() * 1000.0).round() as u64;
            wav_filters.push(format!("adelay={}:all=1", delay_ms));
        }
        wav_filters.push("aresample=48000".to_string());

        let mut cmd_wav = Command::new(ffmpeg_bin);
        apply_no_window(&mut cmd_wav);
        cmd_wav
            .args([
                "-y",
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "fatal",
                "-threads",
                &config.threads.to_string(),
                "-fflags",
                "+genpts",
                "-i",
            ])
            .arg(&res.source_path);
        if let Some(psec) = config.preview_sec {
            cmd_wav.args(["-t", &format!("{:.3}", psec)]);
        } else {
            cmd_wav.args(["-t", &format!("{:.3}", render_duration)]);
        }
        cmd_wav
            .args([
                "-vn",
                "-af",
                &wav_filters.join(","),
                "-ac",
                "2",
                "-c:a",
                "pcm_s16le",
                "-rf64",
                "auto",
                "-f",
                "wav",
            ])
            .arg(&temp_wav)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let out_w = cmd_wav.output().map_err(|e| e.to_string())?;
        if !out_w.status.success() {
            return Err(format!(
                "Error pre-rendering AD WAV: {}",
                String::from_utf8_lossy(&out_w.stderr).trim()
            ));
        }

        // Fill intro delay or end credits gap if present
        if config.gap_fill && temp_wav.exists() {
            let mut single_gaps = Vec::new();
            if let Ok(ad_w_info) = get_audio_info(ffprobe_bin, &temp_wav, "a:0") {
                if res.offset < -0.20 {
                    let intro_gap = res.offset.abs();
                    single_gaps.push((0.0, intro_gap, intro_gap));
                }
                if ad_w_info.duration > 0.0 && ad_w_info.duration < render_duration - 1.0 {
                    single_gaps.push((
                        ad_w_info.duration,
                        render_duration,
                        render_duration - ad_w_info.duration,
                    ));
                }
            }
            if !single_gaps.is_empty() {
                let filled = fill_master_audio_gaps_windowed(
                    ffmpeg_bin,
                    &temp_wav,
                    ref_path,
                    ref_stream_map,
                    ref_channels,
                    scratch_dir,
                    config.threads,
                    &single_gaps,
                    config.crossfade_ms,
                )?;
                res.filled_gaps = filled;
            }
        }
    }

    // Encode Standard Stereo deliverable from temp_wav
    let _ = tx.send(PipelineEvent::Progress {
        step_label: format!("[{}] Encoding Synced AD Stereo {}...", res.name, codec_name),
        completed: 75,
        total: 100,
    });
    emit_log(format!(
        "  Encoding Synced AD Stereo {} (alimiter)...",
        codec_name
    ));
    encode_deliverable_from_wav(
        ffmpeg_bin,
        &temp_wav,
        &out_stereo,
        "alimiter=level_in=1:level_out=0.98:limit=0.98",
        config.wav_output,
        config.threads,
    )?;
    emit_log(format!("  [SUCCESS] Saved: {}", out_stereo.display()));
    res.deliverables.push(out_stereo.clone());

    // Encode Night Mode deliverable from temp_wav
    if config.generate_night_mode {
        let _ = tx.send(PipelineEvent::Progress {
            step_label: format!(
                "[{}] Encoding Synced AD Night Mode {} (loudnorm {:.1} LUFS)...",
                res.name, codec_name, config.target_i
            ),
            completed: 90,
            total: 100,
        });
        emit_log(format!(
            "  Encoding Synced AD Night Mode {} (loudnorm {:.1} LUFS)...",
            codec_name, config.target_i
        ));
        let ln_filter = format!(
            "loudnorm=I={:.1}:TP={:.1}:LRA=11",
            config.target_i, config.target_tp
        );
        encode_deliverable_from_wav(
            ffmpeg_bin,
            &temp_wav,
            &out_night,
            &ln_filter,
            config.wav_output,
            config.threads,
        )?;
        emit_log(format!("  [SUCCESS] Saved: {}", out_night.display()));
        res.deliverables.push(out_night);
    }

    if !config.keep_scratch {
        let _ = fs::remove_file(&temp_wav);
    }

    if let Some(vid_src) = mux_video_path {
        if vid_src.exists() {
            let out_sample_mkv =
                out_dir.join(format!("{} (Synced){}.mkv", label_prefix, suffix));
            emit_log(format!(
                "  Multiplexing Video Sample MKV ({:.0}s)...",
                render_duration
            ));
            mux_video_sample(
                ffmpeg_bin,
                vid_src,
                &out_stereo,
                &out_sample_mkv,
                render_duration,
            )?;
            emit_log(format!(
                "  [SUCCESS] Saved MKV Video Sample: {}",
                out_sample_mkv.display()
            ));
            res.deliverables.push(out_sample_mkv);
        }
    }

    Ok(())
}

fn encode_deliverable_from_wav(
    ffmpeg_bin: &Path,
    in_wav: &Path,
    out_path: &Path,
    af_filter: &str,
    wav_output: bool,
    threads: usize,
) -> Result<(), String> {
    let mut cmd = Command::new(ffmpeg_bin);
    apply_no_window(&mut cmd);
    cmd.args([
        "-y",
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "fatal",
        "-threads",
        &threads.to_string(),
        "-i",
    ])
    .arg(in_wav)
    .args(["-af", af_filter, "-ac", "2", "-ar", "48000"]);

    if wav_output {
        cmd.args(["-c:a", "pcm_s16le", "-rf64", "auto"]);
    } else {
        cmd.args(["-c:a", "aac", "-b:a", "256k"]);
    }
    cmd.arg(out_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "FFmpeg failed encoding '{}': {}",
            out_path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

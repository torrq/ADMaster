#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub use admaster::{cli, config, dsp, media, pipeline};

pub mod app;
pub mod icon_svg;
pub mod visual_ui;

use app::ADMasterApp;
use cli::{attach_parent_console_if_windows, execute_headless, parse_cli_args, CliAction};

fn main() -> eframe::Result<()> {
    let mut initial_cfg = config::AppConfig::load();
    initial_cfg.sanitize_window_geometry();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let initial_target = match parse_cli_args(&args, &initial_cfg) {
        CliAction::LaunchGui(target) => target,
        CliAction::PrintAndExit(code, msg) => {
            attach_parent_console_if_windows();
            if code == 0 {
                println!("{}", msg);
            } else {
                eprintln!("{}", msg);
            }
            std::process::exit(code);
        }
        CliAction::RunHeadless {
            target,
            config,
            no_render,
            json_output,
            show_progress,
        } => {
            attach_parent_console_if_windows();
            match execute_headless(target, config, no_render, json_output, show_progress) {
                Ok(()) => std::process::exit(0),
                Err(err) => {
                    eprintln!("Error: {}", err);
                    std::process::exit(1);
                }
            }
        }
    };

    let has_saved_pos = initial_cfg.has_saved_window_position();
    let window_title = format!(
        "ADMaster -- Audio Description Sync & Mastering Engine v{}",
        env!("CARGO_PKG_VERSION")
    );
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(window_title)
        .with_inner_size([initial_cfg.window_width, initial_cfg.window_height])
        .with_min_inner_size([config::MIN_WINDOW_WIDTH, config::MIN_WINDOW_HEIGHT])
        .with_drag_and_drop(true);
    if let Ok(icon) = icon_svg::rasterize_icon_svg(include_bytes!("../assets/icon.svg"), 256) {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    if let (Some(x), Some(y)) = (initial_cfg.window_x, initial_cfg.window_y) {
        viewport = viewport.with_position([x, y]);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        centered: !has_saved_pos,
        ..Default::default()
    };

    eframe::run_native(
        "ADMaster",
        native_options,
        Box::new(move |cc| Ok(Box::new(ADMasterApp::new(cc, initial_cfg, initial_target)))),
    )
}

#[cfg(test)]
mod tests {
    use crate::config::AppConfig;
    use crate::dsp::correlation::{
        evaluate_anchor_correlations, find_offset_candidates, next_fast_even_len,
        normalized_sliding_correlation,
    };
    use std::path::Path;
    use crate::dsp::fingerprint::fingerprint_align;
    use crate::dsp::gap_fill::fill_master_audio_gaps_windowed;
    use crate::dsp::warp::{
        build_candidate_lattice, decode_warp_path, fit_and_refine_warp,
        render_warped_wav_streaming, PchipSpline,
    };
    use crate::media::{read_wav_mono_f32, WavStreamReader, WavStreamWriter};

    #[test]
    fn test_pchip_strict_monotonicity_and_inversion() {
        let xs = vec![0.0, 10.0, 25.0, 60.0, 120.0];
        let ys = vec![2.5, 12.5, 29.0, 64.0, 124.2];
        let sp = PchipSpline::new(xs, ys);
        let inv = sp.invert((0.0, 120.0));

        let mut prev_y = -1.0_f64;
        for i in 0..=240 {
            let t = (i as f64) * 0.5;
            let y = sp.eval(t);
            assert!(y > prev_y, "PCHIP must be strictly increasing");
            prev_y = y;

            let t_back = inv.eval(y);
            assert!(
                (t_back - t).abs() < 0.05,
                "Inverse PCHIP mismatch at t={}: got {}",
                t,
                t_back
            );
        }
    }

    #[test]
    fn test_multiband_cross_correlation_and_sub_sample_offset() {
        let sr = 8000u32;
        let dur_sec = 120usize;
        let n = (sr as usize) * dur_sec;

        // Generate synthetic reference broadband chirp/transient track
        let mut ref_sig = vec![0.0_f32; n];
        for i in 0..n {
            let t = (i as f64) / (sr as f64);
            let f1 = (2.0 * std::f64::consts::PI * (140.0 + 3.5 * t) * t).sin();
            let f2 = (2.0 * std::f64::consts::PI * (1650.0 + 11.0 * (t * 0.7).sin() * 200.0) * t)
                .sin();
            let env = ((t * 1.7).sin().abs() + 0.2) as f32;
            ref_sig[i] = ((0.5 * f1 + 0.5 * f2) as f32) * env * 0.4;
        }

        // Shift AD track by +2.375 seconds (AD has 2.375s of extra leading intro, so ad[t + 2.375] == ref[t])
        let shift_sec = 2.375_f64;
        let shift_samples = (shift_sec * (sr as f64)).round() as usize;
        let mut ad_sig = vec![0.0_f32; n + shift_samples];
        for i in 0..n {
            let t = (i as f64) / (sr as f64);
            // Add synthetic narrator voice in 450 Hz mid band on top of AD bed
            let narrator = if (i / 4000) % 3 == 0 {
                0.35 * (2.0 * std::f64::consts::PI * 450.0 * t).sin() as f32
            } else {
                0.0
            };
            ad_sig[i + shift_samples] = ref_sig[i] + narrator;
        }

        let cancel = std::sync::atomic::AtomicBool::new(false);
        let meas = evaluate_anchor_correlations(
            &ref_sig,
            &ad_sig,
            sr,
            dur_sec as f64,
            15,
            true,
            &cancel,
            |_, _, _| {},
        );
        let clusters = find_offset_candidates(&meas, 0.15, 0.03);
        assert!(!clusters.is_empty());
        let winner = clusters[0];
        assert!(
            (winner.median_offset - shift_sec).abs() < 0.015,
            "Expected offset ~+2.375s, got {:+.4}s (conf={:.2})",
            winner.median_offset,
            winner.max_peak
        );
        assert!(winner.max_peak > 0.45);
    }

    #[test]
    fn test_packed_fft_correlation_exact_match() {
        // Verify 2-real-in-1-complex packed 5-smooth FFT matches brute-force normalized correlation
        assert_eq!(next_fast_even_len(304_000), 307_200);
        let n = 750usize;
        let m = 211usize; // n + m - 1 = 960 = 2^6 * 3 * 5 (non-power-of-two 5-smooth)
        let mut v = vec![0.0_f32; n];
        let mut a = vec![0.0_f32; m];
        for i in 0..n {
            v[i] = ((i as f32) * 0.13).sin() + 0.4 * ((i as f32) * 0.47).cos();
        }
        for i in 0..m {
            a[i] = v[i + 173] * 1.25 + 0.15; // scaled & shifted copy at lag 173
        }
        let fast = normalized_sliding_correlation(&v, &a).expect("non-empty correlation");
        assert_eq!(fast.len(), n - m + 1);
        let (best_lag, &best_val) = fast
            .iter()
            .enumerate()
            .max_by(|x, y| x.1.partial_cmp(y.1).unwrap())
            .unwrap();
        assert_eq!(best_lag, 173, "Expected argmax at lag 173");
        assert!(
            best_val > 0.99,
            "Expected strong correlation peak at lag 173, got {}",
            best_val
        );

        let a_mean: f64 = a.iter().map(|&x| x as f64).sum::<f64>() / (m as f64);
        let a_e: f64 = a
            .iter()
            .map(|&x| {
                let c = (x as f64) - a_mean;
                c * c
            })
            .sum::<f64>()
            .sqrt();
        let v_mean: f64 = v.iter().map(|&x| x as f64).sum::<f64>() / (n as f64);
        let mut max_err = 0.0_f64;
        for k in 0..(n - m + 1) {
            let mut dot = 0.0_f64;
            let mut ve_sq = 0.0_f64;
            for j in 0..m {
                let vc = (v[k + j] as f64) - v_mean;
                let ac = (a[j] as f64) - a_mean;
                dot += vc * ac;
                ve_sq += vc * vc;
            }
            let exact = (dot / (a_e * ve_sq.sqrt())).clamp(-1.0, 1.0);
            max_err = max_err.max((fast[k] - exact).abs());
        }
        assert!(
            max_err < 1e-4,
            "Packed f32 FFT correlation diverged from brute-force f64 by {}",
            max_err
        );
    }

    #[test]
    fn test_log_filename_and_dir_tokens() {
        let mut cfg = AppConfig::default();
        assert!(cfg.log_dir.is_empty());
        let mkv = Path::new("C:/Movies/Od/The.Odyssey.2026.1080p.mkv");
        let resolved = cfg.resolve_log_dir(
            mkv,
            Path::new("C:/Movies/Od"),
            Path::new("C:/Movies/Od/output"),
            "The Odyssey",
        );
        assert_eq!(resolved, Path::new("C:/Movies/Od"));

        cfg.log_filename_pattern = "{mkv}_{method}_{tracks}t".to_string();
        let fname = cfg.format_log_filename("The Odyssey", "The.Odyssey.2026.1080p", 2, "warp");
        assert_eq!(fname, "The.Odyssey.2026.1080p_warp_2t.log");

        cfg.log_filename_pattern = "{title}_sync.txt".to_string();
        let fname2 = cfg.format_log_filename("The Odyssey: Cut", "movie", 1, "corr");
        assert_eq!(fname2, "The Odyssey_ Cut_sync.txt");
    }

    #[test]
    fn test_viterbi_warp_and_streaming_catmull_rom_render() {
        let sr = 8000u32;
        let dur_sec = 80usize;
        let n = (sr as usize) * dur_sec;

        let mut ref_sig = vec![0.0_f32; n];
        for i in 0..n {
            let t = (i as f64) / (sr as f64);
            let s1 = (2.0 * std::f64::consts::PI * (180.0 + 2.1 * t) * t).sin();
            let s2 = (2.0 * std::f64::consts::PI * (1800.0 + 180.0 * (t * 0.5).cos()) * t).sin();
            ref_sig[i] = ((0.5 * s1 + 0.5 * s2) as f32) * 0.4;
        }

        let cancel = std::sync::atomic::AtomicBool::new(false);
        let fp = fingerprint_align(&ref_sig, &ref_sig, sr);
        let lattice = build_candidate_lattice(
            &ref_sig,
            &ref_sig,
            sr,
            8.0,
            2.0,
            15.0,
            0.0,
            Some(&fp),
            5,
            true,
            &cancel,
            |_, _| {},
        );
        let (pts, _) = decode_warp_path(&lattice, 0.0, Some(&fp));
        let plan = fit_and_refine_warp(pts, dur_sec as f64, dur_sec as f64, &ref_sig, sr, 0.8, 0.6);

        let temp_dir = std::env::temp_dir().join("admaster_rust_tests");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let src_wav = temp_dir.join("src_stereo.wav");
        let out_wav = temp_dir.join("warped_stereo.wav");

        {
            let mut w = WavStreamWriter::create(&src_wav, sr, 2, n as u64).unwrap();
            let interleaved: Vec<f32> = ref_sig.iter().flat_map(|&s| [s, s]).collect();
            w.write_interleaved_f32(&interleaved).unwrap();
            w.finalize().unwrap();
        }

        render_warped_wav_streaming(&src_wav, &out_wav, &plan, dur_sec as f64, 50, &cancel, |_| {})
            .unwrap();
        let (out_mono, out_sr) = read_wav_mono_f32(&out_wav).unwrap();
        assert_eq!(out_sr, sr);
        assert_eq!(out_mono.len(), n);

        // Verify windowed WavStreamReader random-access seek accuracy
        let mut reader = WavStreamReader::open(&out_wav).unwrap();
        let mut win_buf = Vec::new();
        let n_read = reader
            .read_frames_stereo_f32(1000, 500, &mut win_buf)
            .unwrap();
        assert_eq!(n_read, 500);
        assert_eq!(win_buf.len(), 1000);

        // Verify empty explicit_gaps short-circuits in 0ms without spawning FFmpeg
        let dummy_ffmpeg = temp_dir.join("nonexistent_ffmpeg.exe");
        let gaps = fill_master_audio_gaps_windowed(
            &dummy_ffmpeg,
            &out_wav,
            &src_wav,
            None,
            2,
            &temp_dir,
            2,
            &[],
            50,
            &cancel,
        )
        .unwrap();
        assert!(gaps.is_empty());

        assert_eq!(crate::pipeline::format_human_duration(42.2), "42s");
        assert_eq!(crate::pipeline::format_human_duration(80.0), "1m 20s");
        assert_eq!(crate::pipeline::format_human_duration(3852.0), "1h 4m 12s");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

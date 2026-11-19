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
    use crate::dsp::correlation::{evaluate_anchor_correlations, find_offset_candidates};
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

        let meas = evaluate_anchor_correlations(
            &ref_sig,
            &ad_sig,
            sr,
            dur_sec as f64,
            15,
            true,
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

        render_warped_wav_streaming(&src_wav, &out_wav, &plan, dur_sec as f64, 50).unwrap();
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
        )
        .unwrap();
        assert!(gaps.is_empty());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

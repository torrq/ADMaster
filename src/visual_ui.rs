use crate::pipeline::{format_human_duration, format_time_code, MovieJobReport, TrackSyncResult};
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, FontId, Frame, Layout, Margin, Pos2, Rect, RichText,
    Rounding, Sense, Stroke, Vec2,
};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResultsTab {
    #[default]
    VisualInspector,
    ConsoleLog,
}

#[derive(Debug, Clone, Default)]
pub struct VisualState {
    pub tab: ResultsTab,
    pub selected_movie_idx: usize,
    pub selected_ad_idx: usize,
}

#[derive(Debug, Clone)]
pub enum VisualAction {
    None,
    OpenFolder(PathBuf),
    RevealFile(PathBuf),
}

#[derive(Debug, Clone, Copy)]
pub struct VisualColors {
    pub dark: bool,
    pub surface: Color32,
    pub surface_alt: Color32,
    pub well: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub text_primary: Color32,
    pub text_muted: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
}

fn conf_tier_color(conf: f64, pal: &VisualColors) -> Color32 {
    if conf >= 0.60 {
        pal.success
    } else if conf >= 0.35 {
        pal.warning
    } else {
        pal.error
    }
}

fn anchor_dot_color(raw_conf: f64, pal: &VisualColors) -> Color32 {
    if raw_conf >= 0.45 {
        pal.success
    } else if raw_conf >= 0.18 {
        pal.warning
    } else {
        pal.error
    }
}

fn conf_verdict_label(conf: f64) -> &'static str {
    if conf >= 0.80 {
        "Strong Match"
    } else if conf >= 0.60 {
        "Good Match"
    } else if conf >= 0.35 {
        "Fair Match"
    } else {
        "Needs Review"
    }
}

fn format_file_size(path: &Path) -> String {
    let bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0) as f64;
    if bytes >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", bytes / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024.0 * 1024.0 {
        format!("{:.1} MB", bytes / (1024.0 * 1024.0))
    } else if bytes >= 1024.0 {
        format!("{:.1} KB", bytes / 1024.0)
    } else if bytes > 0.0 {
        format!("{:.0} B", bytes)
    } else {
        "--".to_string()
    }
}

fn format_hms_short(sec: f64) -> String {
    let s = sec.max(0.0).round() as u64;
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let r = s % 60;
    format!("{:02}:{:02}:{:02}", h, m, r)
}

fn describe_offset_plain(offset: f64) -> String {
    let abs_ms = (offset.abs() * 1000.0).round();
    if abs_ms < 5.0 {
        "Timing lined up right on the mark (0 ms shift) with no trim or padding needed."
            .to_string()
    } else if offset > 0.0 {
        if offset < 1.0 {
            format!(
                "Trimmed {:.0} ms ({:+.3}s) from the start so it opens right on frame.",
                abs_ms, offset
            )
        } else {
            format!(
                "Trimmed {:.2}s ({:+.3}s) of extra intro from the beginning.",
                offset, offset
            )
        }
    } else if offset.abs() < 1.0 {
        format!(
            "Added {:.0} ms ({:+.3}s) of lead-in padding so it enters right on frame.",
            abs_ms, offset
        )
    } else {
        format!(
            "Added {:.2}s ({:+.3}s) of lead-in alignment at the start.",
            offset.abs(),
            offset
        )
    }
}

struct TrackStory {
    headline: String,
    lead: String,
    detail: String,
    badge: String,
}

fn describe_shift_short(offset: f64) -> String {
    let abs_ms = (offset.abs() * 1000.0).round();
    if abs_ms < 5.0 {
        format!("virtually zero timing drift ({:+.3}s)", offset)
    } else if abs_ms < 100.0 {
        format!("a tiny {:.0} ms timing adjustment ({:+.3}s)", abs_ms, offset)
    } else if abs_ms < 1000.0 {
        format!("a {:.0} ms timing adjustment ({:+.3}s)", abs_ms, offset)
    } else {
        format!(
            "a {:.2}s timing adjustment ({:+.3}s)",
            offset.abs(),
            offset
        )
    }
}

fn build_track_story(ad: &TrackSyncResult) -> TrackStory {
    let pct = (ad.confidence * 100.0).round();
    let badge = format!("{}  {:.0}%", conf_verdict_label(ad.confidence), pct);

    let speed_clause = if ad.is_pal {
        if let Some(st) = ad.speed_stretch {
            format!(
                "Detected a x{:.4} speed transfer and restored exact movie speed and pitch",
                st
            )
        } else {
            "Detected a PAL 25fps transfer and restored exact 23.976fps movie speed and pitch"
                .to_string()
        }
    } else {
        "Playback stayed at native 1.0x speed (no pitch or tempo stretching)".to_string()
    };

    let low_conf = ad.confidence < 0.35;

    if ad.method == "warp" {
        let segs = ad.warp_cuts.len().max(1);
        let gaps = ad.filled_gaps.len();
        let total_gap: f64 = ad.filled_gaps.iter().map(|g| g.duration_sec).sum();
        let seg_word = if segs == 1 { "segment" } else { "segments" };
        let gap_word = if gaps == 1 { "scene" } else { "scenes" };

        if gaps > 0 {
            let headline = if low_conf {
                "Synced -- quick listen check recommended".to_string()
            } else if ad.is_pal {
                format!(
                    "Synced with PAL speed fix and {} missing {} filled",
                    gaps, gap_word
                )
            } else {
                format!("Synced cleanly -- {} missing {} filled", gaps, gap_word)
            };
            let lead = format!(
                "Time-Warp matched this track across {} continuous {} and seamlessly filled {} missing {} ({:.1}s total) using volume-matched movie audio.",
                segs, seg_word, gaps, gap_word, total_gap
            );
            let detail = format!(
                "{}, with {}.",
                speed_clause,
                describe_shift_short(ad.offset)
            );
            TrackStory {
                headline,
                lead,
                detail,
                badge,
            }
        } else if segs > 1 {
            let headline = if low_conf {
                "Synced -- quick listen check recommended".to_string()
            } else if ad.is_pal {
                format!(
                    "Synced with PAL speed fix across {} scene segments",
                    segs
                )
            } else {
                format!("Synced cleanly across {} scene segments", segs)
            };
            let lead = format!(
                "Time-Warp continuously aligned this track across {} scene segments so it stays locked to the movie from start to finish.",
                segs
            );
            let detail = format!(
                "{}, aligned with {}.",
                speed_clause,
                describe_shift_short(ad.offset)
            );
            TrackStory {
                headline,
                lead,
                detail,
                badge,
            }
        } else {
            let headline = if low_conf {
                "Synced -- quick listen check recommended".to_string()
            } else if ad.is_pal {
                "Synced cleanly -- PAL speed restored".to_string()
            } else {
                "Synced cleanly -- no cuts or speed changes needed".to_string()
            };
            let lead = "This track matched the full movie as one smooth, uninterrupted timeline with no missing scenes.".to_string();
            let detail = format!(
                "{}, aligned by Time-Warp with {}.",
                speed_clause,
                describe_shift_short(ad.offset)
            );
            TrackStory {
                headline,
                lead,
                detail,
                badge,
            }
        }
    } else {
        let headline = if low_conf {
            "Synced -- quick listen check recommended".to_string()
        } else if ad.is_pal {
            "Synced cleanly -- PAL speed restored".to_string()
        } else {
            "Synced cleanly -- direct frame-accurate lock".to_string()
        };
        let lead = if ad.total_anchors > 0 {
            format!(
                "This track locked directly onto the movie across {} of {} checkpoints with no structural edits needed.",
                ad.winner_anchors, ad.total_anchors
            )
        } else {
            "This track locked directly onto the movie with no structural edits needed.".to_string()
        };
        let detail = format!("{}. {}", speed_clause, describe_offset_plain(ad.offset));
        TrackStory {
            headline,
            lead,
            detail,
            badge,
        }
    }
}

fn paint_tag_pill(ui: &mut egui::Ui, text: &str, bg: Color32, fg: Color32) {
    let font = FontId::proportional(11.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, fg);
    let pad = Vec2::new(7.0, 2.5);
    let size = Vec2::new(galley.size().x + pad.x * 2.0, galley.size().y + pad.y * 2.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, Rounding::same(4.0), bg);
    ui.painter()
        .galley(Pos2::new(rect.left() + pad.x, rect.top() + pad.y), galley, fg);
}

fn row_lr(
    ui: &mut egui::Ui,
    height: f32,
    left: impl FnOnce(&mut egui::Ui),
    right: impl FnOnce(&mut egui::Ui),
) {
    let w = ui.available_width().max(120.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, height), Sense::hover());
    let mut row_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    row_ui.set_clip_rect(ui.clip_rect().intersect(rect.expand(1.0)));
    left(&mut row_ui);
    let rem = row_ui.available_rect_before_wrap();
    if rem.width() > 2.0 {
        let mut r_ui = row_ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rem)
                .layout(Layout::right_to_left(Align::Center)),
        );
        right(&mut r_ui);
    }
}

pub fn render_visual_results(
    ui: &mut egui::Ui,
    reports: &[MovieJobReport],
    state: &mut VisualState,
    pal: &VisualColors,
) -> VisualAction {
    let mut action = VisualAction::None;

    if reports.is_empty() {
        ui.label(
            RichText::new("No sync report available yet.")
                .size(13.0)
                .color(pal.text_muted),
        );
        return action;
    }

    if state.selected_movie_idx >= reports.len() {
        state.selected_movie_idx = 0;
    }

    // Batch movie switcher when > 1 movie
    if reports.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Movies:")
                    .size(12.0)
                    .strong()
                    .color(pal.text_muted),
            );
            for (idx, r) in reports.iter().enumerate() {
                let active = idx == state.selected_movie_idx;
                let fill = if active { pal.accent } else { pal.surface_alt };
                let text_col = if active { Color32::WHITE } else { pal.text_primary };
                if ui
                    .add(
                        egui::Button::new(RichText::new(&r.base_title).size(12.0).color(text_col))
                            .fill(fill)
                            .rounding(Rounding::same(6.0)),
                    )
                    .clicked()
                {
                    state.selected_movie_idx = idx;
                    state.selected_ad_idx = 0;
                }
            }
        });
        ui.add_space(8.0);
    }

    let rep = &reports[state.selected_movie_idx];
    if state.selected_ad_idx >= rep.ad_results.len() {
        state.selected_ad_idx = 0;
    }

    let soft_bg = |col: Color32| {
        Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), if pal.dark { 36 } else { 24 })
    };

    egui::ScrollArea::vertical()
        .id_salt("admaster_visual_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let avail_w = ui.available_width().max(320.0);
            ui.set_width(avail_w);

            // =================================================================
            // CARD 1: LAYOUT C FRIENDLY REPORT CARD + CHECKLIST + FILMSTRIP MAP
            // =================================================================
            Frame::none()
                .fill(pal.surface)
                .rounding(Rounding::same(10.0))
                .stroke(Stroke::new(1.0_f32, pal.border))
                .inner_margin(Margin::symmetric(16.0, 14.0))
                .show(ui, |ui| {
                    let inner_w = (avail_w - 34.0).max(260.0);
                    ui.set_width(inner_w);

                    // Top Movie Header + Clickable Track Pills
                    let mkv_name = rep
                        .ref_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Movie");
                    let subtitle = format!(
                        "{}  --  {}ch @ {:.1} kHz  |  Finished in {}",
                        mkv_name,
                        rep.ref_channels,
                        (rep.ref_sample_rate as f64) / 1000.0,
                        format_human_duration(rep.elapsed_secs)
                    );

                    ui.horizontal_wrapped(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(&rep.base_title)
                                    .size(16.0)
                                    .strong()
                                    .color(pal.text_primary),
                            );
                            ui.label(RichText::new(&subtitle).size(12.0).color(pal.text_muted));
                        });
                    });

                    if !rep.ad_results.is_empty() {
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                            for (idx, ad) in rep.ad_results.iter().enumerate() {
                                let sel = idx == state.selected_ad_idx;
                                let fill = if sel { pal.accent } else { pal.surface_alt };
                                let fg = if sel { Color32::WHITE } else { pal.text_primary };
                                let btn_lbl = format!(
                                    "Track #{}: {}  ({:.0}%)",
                                    idx + 1,
                                    ad.name,
                                    ad.confidence * 100.0
                                );
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(btn_lbl).size(12.5).strong().color(fg),
                                        )
                                        .fill(fill)
                                        .stroke(Stroke::new(
                                            1.0_f32,
                                            if sel { pal.accent } else { pal.border },
                                        ))
                                        .rounding(Rounding::same(7.0))
                                        .min_size(Vec2::new(0.0, 28.0)),
                                    )
                                    .on_hover_cursor(CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    state.selected_ad_idx = idx;
                                }
                            }
                        });
                    }

                    ui.add_space(10.0);

                    if let Some(ad) = rep.ad_results.get(state.selected_ad_idx) {
                        // Plain-English Sync Summary Card with Left Accent Bar
                        let story = build_track_story(ad);
                        let story_accent = conf_tier_color(ad.confidence, pal);
                        let blend = |base: Color32, tint: Color32, a: f32| -> Color32 {
                            let inv = 1.0 - a;
                            Color32::from_rgb(
                                (base.r() as f32 * inv + tint.r() as f32 * a).round() as u8,
                                (base.g() as f32 * inv + tint.g() as f32 * a).round() as u8,
                                (base.b() as f32 * inv + tint.b() as f32 * a).round() as u8,
                            )
                        };
                        let card_fill = blend(
                            pal.well,
                            story_accent,
                            if pal.dark { 0.08 } else { 0.05 },
                        );
                        let card_stroke = blend(
                            pal.border,
                            story_accent,
                            if pal.dark { 0.35 } else { 0.28 },
                        );

                        let story_resp = Frame::none()
                            .fill(card_fill)
                            .rounding(Rounding::same(10.0))
                            .stroke(Stroke::new(1.0_f32, card_stroke))
                            .inner_margin(Margin {
                                left: 20.0,
                                right: 18.0,
                                top: 14.0,
                                bottom: 14.0,
                            })
                            .show(ui, |ui| {
                                let story_w = (inner_w - 38.0).max(160.0);
                                ui.set_width(story_w);

                                // Eyebrow row: SYNC SUMMARY | Track Name   [ Verdict Badge ]
                                // Pre-measure the right badge so long filenames truncate cleanly
                                // without ever pushing or clipping the badge.
                                let badge_font = FontId::proportional(12.0);
                                let badge_galley = ui.painter().layout_no_wrap(
                                    story.badge.clone(),
                                    badge_font,
                                    story_accent,
                                );
                                let badge_pad = Vec2::new(9.0, 3.5);
                                let badge_size = Vec2::new(
                                    badge_galley.size().x + badge_pad.x * 2.0,
                                    badge_galley.size().y + badge_pad.y * 2.0,
                                );
                                let row_h = badge_size.y.max(22.0);
                                let (row_rect, _) =
                                    ui.allocate_exact_size(Vec2::new(story_w, row_h), Sense::hover());

                                let badge_rect = Rect::from_min_size(
                                    Pos2::new(
                                        row_rect.right() - badge_size.x,
                                        row_rect.center().y - badge_size.y * 0.5,
                                    ),
                                    badge_size,
                                );
                                ui.painter().rect(
                                    badge_rect,
                                    Rounding::same(5.0),
                                    soft_bg(story_accent),
                                    Stroke::new(1.0_f32, card_stroke),
                                );
                                ui.painter().galley(
                                    Pos2::new(
                                        badge_rect.left() + badge_pad.x,
                                        badge_rect.top() + badge_pad.y,
                                    ),
                                    badge_galley,
                                    story_accent,
                                );

                                let left_rect = Rect::from_min_max(
                                    row_rect.min,
                                    Pos2::new(
                                        (badge_rect.left() - 10.0).max(row_rect.left() + 40.0),
                                        row_rect.bottom(),
                                    ),
                                );
                                let mut left_ui = ui.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(left_rect)
                                        .layout(Layout::left_to_right(Align::Center)),
                                );
                                left_ui.set_clip_rect(ui.clip_rect().intersect(left_rect));
                                left_ui.label(
                                    RichText::new("SYNC SUMMARY")
                                        .size(11.5)
                                        .strong()
                                        .color(story_accent),
                                );
                                left_ui.label(
                                    RichText::new("|")
                                        .size(12.0)
                                        .color(pal.border),
                                );
                                left_ui.add(
                                    egui::Label::new(
                                        RichText::new(&ad.name)
                                            .size(12.5)
                                            .color(pal.text_muted),
                                    )
                                    .truncate(),
                                );

                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(&story.headline)
                                        .size(16.5)
                                        .strong()
                                        .color(pal.text_primary),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(&story.lead)
                                        .size(14.5)
                                        .color(pal.text_primary),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(&story.detail)
                                        .size(13.5)
                                        .color(pal.text_muted),
                                );
                            });
                        let sr = story_resp.response.rect;
                        let stripe = Rect::from_min_max(
                            Pos2::new(sr.left() + 1.0, sr.top() + 5.0),
                            Pos2::new(sr.left() + 5.5, sr.bottom() - 5.0),
                        );
                        ui.painter()
                            .rect_filled(stripe, Rounding::same(2.5), story_accent);

                        ui.add_space(12.0);

                        // 4-Column Plain-English Checklist Cards
                        let speed_badge = if ad.is_pal { "Fixed" } else { "Passed" };
                        let speed_col = if ad.is_pal { pal.warning } else { pal.success };
                        let speed_val = if ad.is_pal {
                            "PAL -> 23.976 fps".to_string()
                        } else {
                            "Normal Speed (1.0x)".to_string()
                        };

                        let lock_badge = format!("{:.0}%", ad.confidence * 100.0);
                        let lock_col = conf_tier_color(ad.confidence, pal);
                        let lock_val = if ad.total_anchors > 0 && ad.method != "warp" {
                            format!(
                                "{} ({}/{})",
                                conf_verdict_label(ad.confidence),
                                ad.winner_anchors,
                                ad.total_anchors
                            )
                        } else {
                            conf_verdict_label(ad.confidence).to_string()
                        };

                        let is_warped = ad.method == "warp";
                        let edits_badge = if is_warped { "Warped" } else { "Exact" };
                        let edits_col = if ad.filled_gaps.is_empty() && ad.warp_cuts.len() <= 1 {
                            pal.success
                        } else {
                            pal.warning
                        };
                        let edits_val = if !ad.filled_gaps.is_empty() {
                            format!(
                                "{} segs, {} gap(s) filled",
                                ad.warp_cuts.len().max(1),
                                ad.filled_gaps.len()
                            )
                        } else if ad.warp_cuts.len() > 1 {
                            format!("{} scene segments", ad.warp_cuts.len())
                        } else {
                            format!("Same cut ({:+.3}s)", ad.offset)
                        };

                        let (log_badge, log_col, log_val) = match &rep.saved_log_path {
                            Some(lp) => (
                                "Saved",
                                pal.success,
                                lp.file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("sync.log")
                                    .to_string(),
                            ),
                            None => ("Off", pal.text_muted, "Logging disabled".to_string()),
                        };

                        let cards = [
                            ("Speed Check", speed_badge, speed_col, speed_val),
                            ("Timing Lock", lock_badge.as_str(), lock_col, lock_val),
                            ("Scene Edits", edits_badge, edits_col, edits_val),
                            ("Session Log", log_badge, log_col, log_val),
                        ];

                        let gap = 8.0_f32;
                        let cols: usize = if inner_w >= 640.0 { 4 } else { 2 };
                        let rows: usize = 4 / cols;
                        let card_h = 58.0_f32;
                        let slot_w =
                            ((inner_w - gap * ((cols - 1) as f32)) / (cols as f32)).max(110.0);
                        let grid_h = card_h * (rows as f32) + gap * ((rows - 1) as f32);
                        let (grid_rect, _) =
                            ui.allocate_exact_size(Vec2::new(inner_w, grid_h), Sense::hover());

                        for (idx, (lbl, bdg, col, val)) in cards.iter().enumerate() {
                            let r = idx / cols;
                            let c = idx % cols;
                            let x = grid_rect.left() + (c as f32) * (slot_w + gap);
                            let y = grid_rect.top() + (r as f32) * (card_h + gap);
                            let w = if c + 1 == cols {
                                (grid_rect.right() - x).max(80.0)
                            } else {
                                slot_w
                            };
                            let slot_rect =
                                Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, card_h));
                            render_checklist_card(
                                ui,
                                slot_rect,
                                lbl,
                                bdg,
                                *col,
                                val,
                                pal,
                                soft_bg(*col),
                            );
                        }

                        ui.add_space(12.0);

                        // Movie Structure Map (Visual Filmstrip Timeline)
                        render_filmstrip_map(ui, ad, rep.target_duration, inner_w, pal);
                    } else {
                        // Main-audio-only or skipped job summary box
                        let msg = if rep.skipped {
                            "All requested output files already existed in output/ and were preserved (enable Overwrite to regenerate)."
                        } else {
                            "Main movie audio conversion completed (no Audio Description tracks were staged in this run)."
                        };
                        Frame::none()
                            .fill(pal.well)
                            .rounding(Rounding::same(8.0))
                            .stroke(Stroke::new(1.0_f32, pal.border))
                            .inner_margin(Margin::symmetric(14.0, 10.0))
                            .show(ui, |ui| {
                                ui.set_width((inner_w - 30.0).max(160.0));
                                ui.label(RichText::new(msg).size(13.0).color(pal.text_primary));
                            });
                    }
                });

            ui.add_space(10.0);

            // =================================================================
            // CARD 2: INTERACTIVE ALIGNMENT TIMELINE CHART
            // =================================================================
            if let Some(ad) = rep.ad_results.get(state.selected_ad_idx) {
                render_alignment_chart_card(ui, ad, rep.target_duration, avail_w, pal);
                ui.add_space(10.0);
            }

            // =================================================================
            // CARD 3: OUTPUT FILES READY (WITH REVEAL & OPEN FOLDER ACTIONS)
            // =================================================================
            if let Some(act) = render_output_files_card(ui, rep, avail_w, pal, &soft_bg) {
                action = act;
            }
        });

    action
}

#[allow(clippy::too_many_arguments)]
fn render_checklist_card(
    ui: &mut egui::Ui,
    slot_rect: Rect,
    label: &str,
    badge: &str,
    badge_color: Color32,
    value: &str,
    pal: &VisualColors,
    badge_bg: Color32,
) {
    ui.painter().rect(
        slot_rect,
        Rounding::same(8.0),
        pal.well,
        Stroke::new(1.0_f32, pal.border),
    );
    let inner_rect = slot_rect.shrink2(Vec2::new(10.0, 8.0));
    let mut cell_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    cell_ui.set_clip_rect(ui.clip_rect().intersect(slot_rect.shrink(1.0)));
    row_lr(
        &mut cell_ui,
        20.0,
        |ui| {
            ui.label(RichText::new(label).size(11.5).color(pal.text_muted));
        },
        |ui| {
            paint_tag_pill(ui, badge, badge_bg, badge_color);
        },
    );
    cell_ui.add_space(4.0);
    cell_ui
        .add(
            egui::Label::new(
                RichText::new(value)
                    .size(12.5)
                    .strong()
                    .color(pal.text_primary),
            )
            .truncate(),
        )
        .on_hover_text(value);
}

fn render_filmstrip_map(
    ui: &mut egui::Ui,
    ad: &TrackSyncResult,
    target_duration: f64,
    width: f32,
    pal: &VisualColors,
) {
    let dur = target_duration.max(1.0);
    row_lr(
        ui,
        18.0,
        |ui| {
            ui.label(
                RichText::new("Movie Structure Map")
                    .size(11.5)
                    .strong()
                    .color(pal.text_muted),
            );
        },
        |ui| {
            ui.label(
                RichText::new(format!("00:00:00  ->  {}", format_hms_short(dur)))
                    .size(11.0)
                    .monospace()
                    .color(pal.text_muted),
            );
        },
    );
    ui.add_space(4.0);

    let bar_h = 38.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, bar_h), Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        Rounding::same(6.0),
        pal.well,
        Stroke::new(1.0_f32, pal.border),
    );

    let seg_fill = Color32::from_rgba_unmultiplied(
        pal.accent.r(),
        pal.accent.g(),
        pal.accent.b(),
        if pal.dark { 48 } else { 32 },
    );
    let gap_fill = Color32::from_rgba_unmultiplied(
        pal.warning.r(),
        pal.warning.g(),
        pal.warning.b(),
        if pal.dark { 58 } else { 40 },
    );

    let x_for = |t: f64| -> f32 {
        let f = (t / dur).clamp(0.0, 1.0) as f32;
        rect.left() + 2.0 + f * (rect.width() - 4.0)
    };

    let mut hover_tip: Option<String> = None;
    let hpos = resp.hover_pos();

    // Draw continuous synced segment blocks
    if ad.warp_cuts.is_empty() {
        let seg_r = rect.shrink(2.0);
        painter.rect(
            seg_r,
            Rounding::same(4.0),
            seg_fill,
            Stroke::new(1.0_f32, pal.accent),
        );
        let txt = format!(
            "Full Movie (1 Continuous Segment)  --  Offset {:+.3}s",
            ad.offset
        );
        painter.text(
            seg_r.center(),
            Align2::CENTER_CENTER,
            &txt,
            FontId::proportional(12.0),
            pal.text_primary,
        );
        if hpos.map(|p| seg_r.contains(p)).unwrap_or(false) {
            hover_tip = Some(format!(
                "Full Movie: 00:00:00 -> {}\nOffset: {:+.3}s | Confidence: {:.1}%",
                format_hms_short(dur),
                ad.offset,
                ad.confidence * 100.0
            ));
        }
    } else {
        for cut in &ad.warp_cuts {
            let x0 = x_for(cut.vid_start);
            let x1 = x_for(cut.vid_end).max(x0 + 6.0).min(rect.right() - 2.0);
            let seg_r = Rect::from_min_max(
                Pos2::new(x0, rect.top() + 2.0),
                Pos2::new(x1, rect.bottom() - 2.0),
            );
            painter.rect(
                seg_r,
                Rounding::same(4.0),
                seg_fill,
                Stroke::new(1.0_f32, pal.accent),
            );
            let sw = seg_r.width();
            if sw >= 74.0 {
                painter.text(
                    Pos2::new(seg_r.center().x, seg_r.center().y - 6.0),
                    Align2::CENTER_CENTER,
                    format!("Part {}", cut.index),
                    FontId::proportional(11.0),
                    pal.accent_hover,
                );
                painter.text(
                    Pos2::new(seg_r.center().x, seg_r.center().y + 7.0),
                    Align2::CENTER_CENTER,
                    format!("{:+.2}s", -cut.offset),
                    FontId::monospace(10.5),
                    pal.text_primary,
                );
            } else if sw >= 34.0 {
                painter.text(
                    seg_r.center(),
                    Align2::CENTER_CENTER,
                    format!("P{}", cut.index),
                    FontId::proportional(10.5),
                    pal.accent_hover,
                );
            }
            if hpos.map(|p| seg_r.contains(p)).unwrap_or(false) {
                hover_tip = Some(format!(
                    "Part {}: {} -> {}\nOffset: {:+.3}s{}",
                    cut.index,
                    format_hms_short(cut.vid_start),
                    format_hms_short(cut.vid_end),
                    -cut.offset,
                    if cut.is_initial {
                        String::new()
                    } else {
                        format!(" (shift {:+.2}s)", -cut.jump)
                    }
                ));
            }
        }
    }

    // Overlay filled scene gaps prominently in amber
    for (idx, g) in ad.filled_gaps.iter().enumerate() {
        let x0 = x_for(g.start_sec);
        let x1 = x_for(g.end_sec).max(x0 + 10.0).min(rect.right() - 2.0);
        let gr = Rect::from_min_max(
            Pos2::new(x0, rect.top() + 2.0),
            Pos2::new(x1, rect.bottom() - 2.0),
        );
        painter.rect(
            gr,
            Rounding::same(3.0),
            gap_fill,
            Stroke::new(1.5_f32, pal.warning),
        );
        if gr.width() >= 34.0 {
            painter.text(
                gr.center(),
                Align2::CENTER_CENTER,
                "GAP",
                FontId::proportional(10.0),
                pal.warning,
            );
        }
        if hpos.map(|p| gr.contains(p)).unwrap_or(false) {
            let db_s = 20.0 * g.gain_start.log10();
            hover_tip = Some(format!(
                "Filled Scene Gap #{}: {} -> {} ({:.1}s)\nMovie audio seamlessly spliced at {:+.1} dB",
                idx + 1,
                format_hms_short(g.start_sec),
                format_hms_short(g.end_sec),
                g.duration_sec,
                db_s
            ));
        }
    }

    if let Some(tip) = hover_tip {
        let _ = resp.on_hover_text(tip);
    }
}

fn render_alignment_chart_card(
    ui: &mut egui::Ui,
    ad: &TrackSyncResult,
    target_duration: f64,
    avail_w: f32,
    pal: &VisualColors,
) {
    let pad_x = 16.0_f32;
    Frame::none()
        .fill(pal.surface)
        .rounding(Rounding::same(10.0))
        .stroke(Stroke::new(1.0_f32, pal.border))
        .inner_margin(Margin::symmetric(pad_x, 12.0))
        .show(ui, |ui| {
            let inner_w = (avail_w - pad_x * 2.0 - 2.0).max(240.0);
            ui.set_width(inner_w);

            let explain = if ad.method == "warp" && (ad.warp_cuts.len() > 1 || !ad.filled_gaps.is_empty()) {
                "Steps show where scenes differed between the movie and AD track. Shaded amber regions were filled with level-matched movie audio."
            } else {
                "A flat line across the movie means your audio track stayed locked in sync from start to finish with zero drift."
            };

            ui.label(
                RichText::new(format!("Alignment Timeline -- {}", ad.name))
                    .size(13.5)
                    .strong()
                    .color(pal.text_primary),
            );
            ui.label(RichText::new(explain).size(11.5).color(pal.text_muted));
            ui.add_space(6.0);

            // Legend row
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(14.0, 4.0);
                paint_legend_item(ui, "Final Sync", pal.accent, false, pal);
                paint_legend_item(ui, "Strong Check (>=45%)", pal.success, true, pal);
                paint_legend_item(ui, "Fair Check", pal.warning, true, pal);
                if !ad.filled_gaps.is_empty() {
                    paint_legend_item(ui, "Filled Scene Gap", pal.warning, false, pal);
                }
            });
            ui.add_space(6.0);

            let hover_status =
                paint_offset_timeline_chart(ui, ad, target_duration, inner_w, 200.0, pal);

            ui.add_space(5.0);
            row_lr(
                ui,
                18.0,
                |ui| {
                    let (msg, col) = match hover_status {
                        Some((txt, c)) => (txt, c),
                        None => (
                            "Hover chart to inspect timing and match quality at any point in the movie"
                                .to_string(),
                            pal.text_muted,
                        ),
                    };
                    ui.label(RichText::new(msg).size(11.5).monospace().color(col));
                },
                |ui| {
                    ui.label(
                        RichText::new(format!(
                            "00:00:00 -> {}",
                            format_hms_short(target_duration)
                        ))
                        .size(11.0)
                        .monospace()
                        .color(pal.text_muted),
                    );
                },
            );

            // Clean chronological "Times to Spot-Check" checklist (merges overlapping gaps + cuts)
            let spot_items = build_spot_check_items(ad, target_duration);
            if !spot_items.is_empty() {
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);
                render_spot_check_list(ui, inner_w, &spot_items, pal);
            }
        });
}

struct SpotCheckItem {
    time_sec: f64,
    timestamp: String,
    badge: &'static str,
    badge_kind: u8, // 0 = filled scene (warning), 1 = intro/outro (muted), 2 = timing shift (accent)
    summary: String,
    tooltip: String,
}

fn build_spot_check_items(ad: &TrackSyncResult, target_duration: f64) -> Vec<SpotCheckItem> {
    let mut items: Vec<SpotCheckItem> = Vec::new();
    let mut gap_spans: Vec<(f64, f64, usize)> = Vec::new();

    for g in &ad.filled_gaps {
        let is_intro = g.start_sec <= 5.0;
        let is_outro = target_duration > 30.0 && g.end_sec >= target_duration - 15.0;
        let (badge, badge_kind, desc) = if is_intro {
            (
                "Intro Fill",
                1u8,
                format!(
                    "{:.1}s intro filled with movie audio ({} -> {})",
                    g.duration_sec,
                    format_hms_short(g.start_sec),
                    format_hms_short(g.end_sec)
                ),
            )
        } else if is_outro {
            (
                "Outro Fill",
                1u8,
                format!(
                    "{:.1}s end section filled with movie audio ({} -> {})",
                    g.duration_sec,
                    format_hms_short(g.start_sec),
                    format_hms_short(g.end_sec)
                ),
            )
        } else {
            (
                "Filled Scene",
                0u8,
                format!(
                    "{:.1}s missing scene filled with movie audio ({} -> {})",
                    g.duration_sec,
                    format_hms_short(g.start_sec),
                    format_hms_short(g.end_sec)
                ),
            )
        };
        let db_s = 20.0 * g.gain_start.log10();
        let tooltip = format!(
            "{} -> {} ({:.1}s) | Level match: {:+.1} dB",
            format_hms_short(g.start_sec),
            format_hms_short(g.end_sec),
            g.duration_sec,
            db_s
        );
        let idx = items.len();
        items.push(SpotCheckItem {
            time_sec: g.start_sec,
            timestamp: format_hms_short(g.start_sec),
            badge,
            badge_kind,
            summary: desc,
            tooltip,
        });
        gap_spans.push((g.start_sec, g.end_sec, idx));
    }

    // Merge nearby cuts into their matching gap row so each transition appears once;
    // list standalone timing shifts that didn't require gap filling as their own row.
    for cut in ad.warp_cuts.iter().skip(1) {
        let mut merged_into = None;
        for &(gs, ge, item_idx) in &gap_spans {
            if cut.vid_start >= gs - 4.0 && cut.vid_start <= ge + 6.0 {
                merged_into = Some(item_idx);
                break;
            }
        }
        if let Some(idx) = merged_into {
            items[idx].tooltip.push_str(&format!(
                " | Timing shift: {:+.2}s (offset {:+.2}s)",
                -cut.jump, -cut.offset
            ));
        } else if cut.jump.abs() >= 0.25 {
            items.push(SpotCheckItem {
                time_sec: cut.vid_start,
                timestamp: format_hms_short(cut.vid_start),
                badge: "Timing Shift",
                badge_kind: 2u8,
                summary: format!("Timing adjusted by {:+.2}s at scene transition", -cut.jump),
                tooltip: format!(
                    "Cut at {} | Shift {:+.2}s -> new offset {:+.2}s",
                    format_hms_short(cut.vid_start),
                    -cut.jump,
                    -cut.offset
                ),
            });
        }
    }

    items.sort_by(|a, b| {
        a.time_sec
            .partial_cmp(&b.time_sec)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items
}

fn render_spot_check_list(
    ui: &mut egui::Ui,
    inner_w: f32,
    items: &[SpotCheckItem],
    pal: &VisualColors,
) {
    row_lr(
        ui,
        24.0,
        |ui| {
            ui.label(
                RichText::new(format!("Times to Spot-Check ({})", items.len()))
                    .size(13.5)
                    .strong()
                    .color(pal.text_primary),
            );
            ui.label(
                RichText::new("-- click any timestamp to copy")
                    .size(12.0)
                    .color(pal.text_muted),
            );
        },
        |ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("Copy List").size(11.5))
                        .min_size(Vec2::new(76.0, 22.0)),
                )
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text("Copy all spot-check timestamps and notes to clipboard")
                .clicked()
            {
                let mut text = String::new();
                for item in items {
                    text.push_str(&format!(
                        "{}  [{}]  {}\n",
                        item.timestamp, item.badge, item.summary
                    ));
                }
                ui.ctx().copy_text(text);
            }
        },
    );

    ui.add_space(6.0);

    Frame::none()
        .fill(pal.well)
        .rounding(Rounding::same(8.0))
        .stroke(Stroke::new(1.0_f32, pal.border))
        .inner_margin(Margin::symmetric(10.0, 6.0))
        .show(ui, |ui| {
            ui.set_width((inner_w - 22.0).max(200.0));
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    ui.separator();
                }
                let (badge_bg, badge_fg) = match item.badge_kind {
                    0 => (
                        Color32::from_rgba_unmultiplied(
                            pal.warning.r(),
                            pal.warning.g(),
                            pal.warning.b(),
                            if pal.dark { 40 } else { 26 },
                        ),
                        pal.warning,
                    ),
                    2 => (
                        Color32::from_rgba_unmultiplied(
                            pal.accent.r(),
                            pal.accent.g(),
                            pal.accent.b(),
                            if pal.dark { 40 } else { 24 },
                        ),
                        pal.accent,
                    ),
                    _ => (pal.surface_alt, pal.text_muted),
                };

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.set_min_height(28.0);

                    // Row index
                    ui.add_sized(
                        Vec2::new(22.0, 22.0),
                        egui::Label::new(
                            RichText::new(format!("#{}", idx + 1))
                                .size(11.5)
                                .monospace()
                                .color(pal.text_muted),
                        ),
                    );

                    // High-contrast clickable timestamp button
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(&item.timestamp)
                                    .font(FontId::monospace(13.0))
                                    .strong()
                                    .color(pal.text_primary),
                            )
                            .fill(pal.surface)
                            .stroke(Stroke::new(1.0_f32, pal.border))
                            .rounding(Rounding::same(5.0))
                            .min_size(Vec2::new(82.0, 24.0)),
                        )
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(format!("Click to copy {} to clipboard", item.timestamp))
                        .clicked()
                    {
                        ui.ctx().copy_text(item.timestamp.clone());
                    }

                    // Fixed-width crisp type badge so all descriptions align vertically
                    let badge_size = Vec2::new(92.0, 22.0);
                    let (b_rect, _) = ui.allocate_exact_size(badge_size, Sense::hover());
                    ui.painter()
                        .rect_filled(b_rect, Rounding::same(5.0), badge_bg);
                    let b_galley = ui.painter().layout_no_wrap(
                        item.badge.to_string(),
                        FontId::proportional(11.5),
                        badge_fg,
                    );
                    ui.painter().galley(
                        Pos2::new(
                            b_rect.center().x - b_galley.size().x * 0.5,
                            b_rect.center().y - b_galley.size().y * 0.5,
                        ),
                        b_galley,
                        badge_fg,
                    );

                    // Clean plain-English description with full technical detail on hover
                    ui.add(
                        egui::Label::new(
                            RichText::new(&item.summary)
                                .size(13.0)
                                .color(pal.text_primary),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&item.tooltip);
                });
            }
        });
}

fn paint_legend_item(
    ui: &mut egui::Ui,
    label: &str,
    col: Color32,
    is_dot: bool,
    pal: &VisualColors,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (r, _) = ui.allocate_exact_size(Vec2::new(12.0, 12.0), Sense::hover());
        if is_dot {
            ui.painter().circle_filled(r.center(), 3.6, col);
        } else {
            ui.painter().line_segment(
                [Pos2::new(r.left(), r.center().y), Pos2::new(r.right(), r.center().y)],
                Stroke::new(2.5_f32, col),
            );
        }
        ui.label(RichText::new(label).size(11.5).color(pal.text_muted));
    });
}

fn paint_offset_timeline_chart(
    ui: &mut egui::Ui,
    ad: &TrackSyncResult,
    target_duration: f64,
    width: f32,
    height: f32,
    pal: &VisualColors,
) -> Option<(String, Color32)> {
    let w = width.max(220.0);
    let h = height.max(160.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());

    let painter = ui.painter();
    painter.rect(
        rect,
        Rounding::same(6.0),
        pal.well,
        Stroke::new(1.0_f32, pal.border),
    );

    let plot = Rect::from_min_max(
        Pos2::new(rect.left() + 60.0, rect.top() + 16.0),
        Pos2::new(rect.right() - 16.0, rect.bottom() - 26.0),
    );
    let total_t = target_duration.max(1.0) as f32;

    let mut min_off = ad.offset - 1.0;
    let mut max_off = ad.offset + 1.0;
    for m in &ad.anchor_measurements {
        if m.confidence >= 0.12 {
            min_off = min_off.min(m.offset_sec - 0.5);
            max_off = max_off.max(m.offset_sec + 0.5);
        }
    }
    for p in &ad.warp_points {
        let off = p.source_time - p.target_time;
        min_off = min_off.min(off - 0.5);
        max_off = max_off.max(off + 0.5);
    }
    if (max_off - min_off).abs() < 1.0 {
        min_off -= 0.5;
        max_off += 0.5;
    }

    let x_for_t = |sec: f64| -> f32 {
        let frac = ((sec as f32) / total_t).clamp(0.0, 1.0);
        plot.left() + frac * plot.width()
    };
    let y_for_off = |off: f64| -> f32 {
        let frac = ((off - min_off) / (max_off - min_off)).clamp(0.0, 1.0) as f32;
        plot.bottom() - frac * plot.height()
    };

    // Horizontal Y gridlines
    for step in 0..=4 {
        let frac = (step as f64) / 4.0;
        let off_v = min_off + frac * (max_off - min_off);
        let y = y_for_off(off_v);
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0_f32, pal.border),
        );
        painter.text(
            Pos2::new(plot.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            format!("{:+.2}s", off_v),
            FontId::monospace(10.5),
            pal.text_muted,
        );
    }

    // Bottom X time ticks
    for step in 0..=4 {
        let frac = (step as f64) / 4.0;
        let t_sec = frac * target_duration;
        let x = x_for_t(t_sec);
        let align = if step == 0 {
            Align2::LEFT_TOP
        } else if step == 4 {
            Align2::RIGHT_TOP
        } else {
            Align2::CENTER_TOP
        };
        painter.text(
            Pos2::new(x, plot.bottom() + 6.0),
            align,
            format_hms_short(t_sec),
            FontId::monospace(10.5),
            pal.text_muted,
        );
    }

    // Highlight filled master audio gaps
    for gap in &ad.filled_gaps {
        let x0 = x_for_t(gap.start_sec);
        let x1 = x_for_t(gap.end_sec).max(x0 + 3.0);
        let gap_rect = Rect::from_min_max(Pos2::new(x0, plot.top()), Pos2::new(x1, plot.bottom()));
        painter.rect_filled(
            gap_rect,
            Rounding::ZERO,
            Color32::from_rgba_unmultiplied(pal.warning.r(), pal.warning.g(), pal.warning.b(), 44),
        );
    }

    // Draw consensus baseline offset line (or warp path polyline)
    if ad.warp_points.len() >= 2 {
        let step_stride = (ad.warp_points.len() / 600).max(1);
        let pts: Vec<Pos2> = ad
            .warp_points
            .iter()
            .step_by(step_stride)
            .map(|p| {
                let off = p.source_time - p.target_time;
                Pos2::new(x_for_t(p.target_time), y_for_off(off))
            })
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(2.2_f32, pal.accent)));
    } else {
        let y_base = y_for_off(ad.offset);
        painter.line_segment(
            [
                Pos2::new(plot.left(), y_base),
                Pos2::new(plot.right(), y_base),
            ],
            Stroke::new(2.0_f32, pal.accent),
        );
    }

    // Draw correlation anchor dots colored by raw per-anchor correlation
    for m in &ad.anchor_measurements {
        let pos = Pos2::new(x_for_t(m.anchor_sec), y_for_off(m.offset_sec));
        if plot.contains(pos) {
            let col = anchor_dot_color(m.confidence, pal);
            painter.circle_filled(pos, 3.2, col);
        }
    }

    // Interactive Hover Crosshair + Detailed Readout
    if let Some(hpos) = resp.hover_pos() {
        if plot.contains(hpos) {
            let hover_t = (((hpos.x - plot.left()) / plot.width()) * total_t) as f64;
            painter.line_segment(
                [
                    Pos2::new(hpos.x, plot.top()),
                    Pos2::new(hpos.x, plot.bottom()),
                ],
                Stroke::new(1.0_f32, pal.text_muted),
            );

            // Check if hovering inside a filled gap
            if let Some(gap) = ad
                .filled_gaps
                .iter()
                .find(|g| hover_t >= g.start_sec && hover_t <= g.end_sec)
            {
                let db = 20.0 * gap.gain_start.log10();
                return Some((
                    format!(
                        "Time {}  |  Filled Scene Gap ({:.1}s, movie audio spliced at {:+.1} dB)",
                        format_time_code(hover_t),
                        gap.duration_sec,
                        db
                    ),
                    pal.warning,
                ));
            }

            // Nearest warp point or anchor measurement
            if !ad.warp_points.is_empty() {
                if let Some(wp) = ad.warp_points.iter().min_by(|a, b| {
                    (a.target_time - hover_t)
                        .abs()
                        .partial_cmp(&(b.target_time - hover_t).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                }) {
                    let off = wp.source_time - wp.target_time;
                    return Some((
                        format!(
                            "Time {}  |  Warp Offset: {:+.3}s  |  Local Confidence: {:.0}%",
                            format_time_code(hover_t),
                            off,
                            wp.confidence * 100.0
                        ),
                        pal.text_primary,
                    ));
                }
            }
            if !ad.anchor_measurements.is_empty() {
                if let Some(am) = ad.anchor_measurements.iter().min_by(|a, b| {
                    (a.anchor_sec - hover_t)
                        .abs()
                        .partial_cmp(&(b.anchor_sec - hover_t).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                }) {
                    return Some((
                        format!(
                            "Time {}  |  Measured Offset: {:+.3}s  |  Match Quality: {:.0}%",
                            format_time_code(hover_t),
                            am.offset_sec,
                            am.confidence * 100.0
                        ),
                        pal.text_primary,
                    ));
                }
            }

            return Some((
                format!(
                    "Time {}  |  Sync Offset: {:+.3}s",
                    format_time_code(hover_t),
                    ad.offset
                ),
                pal.text_primary,
            ));
        }
    }

    None
}

fn classify_output_tag(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.ends_with(".log") || name.ends_with(".txt") || name.ends_with(".md") {
        "Session Log"
    } else if name.ends_with(".mkv") {
        "Final MKV"
    } else if name.contains("night mode") {
        "Night Mode"
    } else if name.contains("surround") || name.ends_with(".ac3") {
        "5.1 Surround"
    } else if name.ends_with(".wav") {
        "Stereo WAV"
    } else {
        "Stereo AAC"
    }
}

fn render_output_files_card(
    ui: &mut egui::Ui,
    rep: &MovieJobReport,
    avail_w: f32,
    pal: &VisualColors,
    soft_bg: &impl Fn(Color32) -> Color32,
) -> Option<VisualAction> {
    let mut action: Option<VisualAction> = None;

    // Gather all created deliverables + final MKV + saved session log
    let mut items: Vec<(PathBuf, &'static str, Color32)> = Vec::new();
    for ad in &rep.ad_results {
        for d in &ad.deliverables {
            items.push((d.clone(), classify_output_tag(d), pal.accent_hover));
        }
    }
    for m in &rep.main_deliverables {
        items.push((m.clone(), classify_output_tag(m), pal.accent));
    }
    if let Some(ref mkv) = rep.final_mkv {
        items.push((mkv.clone(), "Final MKV", pal.success));
    }
    if let Some(ref log_p) = rep.saved_log_path {
        items.push((log_p.clone(), "Session Log", pal.success));
    }

    let pad_x = 16.0_f32;
    Frame::none()
        .fill(pal.surface)
        .rounding(Rounding::same(10.0))
        .stroke(Stroke::new(1.0_f32, pal.border))
        .inner_margin(Margin::symmetric(pad_x, 12.0))
        .show(ui, |ui| {
            let inner_w = (avail_w - pad_x * 2.0 - 2.0).max(240.0);
            ui.set_width(inner_w);

            row_lr(
                ui,
                28.0,
                |ui| {
                    ui.label(
                        RichText::new(format!("Output Files Ready ({})", items.len()))
                            .size(13.5)
                            .strong()
                            .color(pal.text_primary),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(rep.out_dir.display().to_string())
                                .size(11.5)
                                .monospace()
                                .color(pal.text_muted),
                        )
                        .truncate(),
                    )
                    .on_hover_text(rep.out_dir.display().to_string());
                },
                |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Open Output Folder").size(12.0))
                                .min_size(Vec2::new(0.0, 26.0)),
                        )
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .clicked()
                    {
                        action = Some(VisualAction::OpenFolder(rep.out_dir.clone()));
                    }
                },
            );

            ui.add_space(6.0);

            if items.is_empty() {
                ui.label(
                    RichText::new(
                        "No audio files were rendered in this pass (analysis / dry run only).",
                    )
                    .size(12.5)
                    .color(pal.text_muted),
                );
            } else {
                for (path, tag, tag_col) in &items {
                    let fname = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("file");
                    let size_txt = format_file_size(path);
                    let full_str = path.display().to_string();

                    Frame::none()
                        .fill(pal.well)
                        .rounding(Rounding::same(6.0))
                        .stroke(Stroke::new(1.0_f32, pal.border))
                        .inner_margin(Margin::symmetric(10.0, 6.0))
                        .show(ui, |ui| {
                            row_lr(
                                ui,
                                24.0,
                                |ui| {
                                    paint_tag_pill(ui, tag, soft_bg(*tag_col), *tag_col);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(fname)
                                                .size(12.5)
                                                .color(pal.text_primary),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&full_str);
                                },
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(RichText::new("Reveal").size(11.5))
                                                .min_size(Vec2::new(0.0, 22.0)),
                                        )
                                        .on_hover_text(format!("Locate {}", full_str))
                                        .on_hover_cursor(CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        action = Some(VisualAction::RevealFile(path.clone()));
                                    }
                                    ui.label(
                                        RichText::new(&size_txt)
                                            .size(11.5)
                                            .monospace()
                                            .color(pal.text_muted),
                                    );
                                },
                            );
                        });
                    ui.add_space(4.0);
                }
            }
        });

    action
}

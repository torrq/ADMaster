use crate::pipeline::{format_time_code, MovieJobReport, TrackSyncResult};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Frame, Margin, Pos2, Rect, RichText, Rounding,
    Sense, Stroke, Vec2,
};

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
    if conf >= 0.45 {
        pal.success
    } else if conf >= 0.15 {
        pal.warning
    } else {
        pal.error
    }
}

fn render_kpi_row(
    ui: &mut egui::Ui,
    items: &[(&str, String, Color32)],
    avail_w: f32,
    pal: &VisualColors,
) {
    if items.is_empty() {
        return;
    }
    let gap = 8.0_f32;
    let count = items.len() as f32;
    if avail_w >= 620.0 {
        let card_w = ((avail_w - gap * (count - 1.0)) / count).max(96.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (idx, (lbl, val, col)) in items.iter().enumerate() {
                let w = if idx + 1 == items.len() {
                    (avail_w - (card_w + gap) * ((items.len() - 1) as f32)).max(96.0)
                } else {
                    card_w
                };
                kpi_pill_sized(ui, lbl, val, *col, Some(w), pal);
            }
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
            for (lbl, val, col) in items {
                kpi_pill_sized(ui, lbl, val, *col, None, pal);
            }
        });
    }
}

fn kpi_pill_sized(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    val_color: Color32,
    exact_width: Option<f32>,
    pal: &VisualColors,
) {
    let inner_pad = Vec2::new(12.0, 8.0);
    let mut frame = Frame::none()
        .fill(pal.surface)
        .rounding(Rounding::same(8.0))
        .stroke(Stroke::new(1.0_f32, pal.border))
        .inner_margin(Margin::symmetric(inner_pad.x, inner_pad.y));
    if exact_width.is_some() {
        frame = frame.outer_margin(Margin::ZERO);
    }
    frame.show(ui, |ui| {
        if let Some(w) = exact_width {
            ui.set_width((w - inner_pad.x * 2.0).max(60.0));
        }
        ui.label(RichText::new(label).size(11.0).color(pal.text_muted));
        ui.label(RichText::new(value).size(15.0).strong().color(val_color));
    });
}

pub fn render_visual_results(
    ui: &mut egui::Ui,
    reports: &[MovieJobReport],
    state: &mut VisualState,
    pal: &VisualColors,
) {
    if reports.is_empty() {
        ui.label(RichText::new("No synchronization reports available yet.").color(pal.text_muted));
        return;
    }

    if state.selected_movie_idx >= reports.len() {
        state.selected_movie_idx = 0;
    }

    // If multiple movies in batch, show movie selector bar at the top
    if reports.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Batch Movies:")
                    .size(12.0)
                    .strong()
                    .color(pal.text_muted),
            );
            for (idx, r) in reports.iter().enumerate() {
                let active = idx == state.selected_movie_idx;
                let fill = if active { pal.accent } else { pal.surface_alt };
                let text_col = if active {
                    Color32::WHITE
                } else {
                    pal.text_primary
                };
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

    let viewport_h = ui.available_height();

    egui::ScrollArea::vertical()
        .id_salt("admaster_visual_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let content_top_y = ui.cursor().top();
            let avail_w = ui.available_width().max(260.0);
            ui.set_width(avail_w);

            // 1. Summary KPI Row
            let sel_ad = rep.ad_results.get(state.selected_ad_idx);
            let mut kpis: Vec<(&str, String, Color32)> = vec![
                ("Movie Title", rep.base_title.clone(), pal.text_primary),
                (
                    "Reference Master",
                    format!(
                        "{}ch @ {}Hz ({})",
                        rep.ref_channels,
                        rep.ref_sample_rate,
                        format_time_code(rep.target_duration)
                    ),
                    pal.text_primary,
                ),
            ];
            if let Some(ad) = sel_ad {
                let m_str = if ad.method == "cross_correlation" {
                    "Cross-Corr (1.0x)"
                } else {
                    "Viterbi + PCHIP Warp"
                };
                kpis.push(("Sync Engine", m_str.to_string(), pal.accent_hover));
                kpis.push((
                    "Global Offset",
                    format!("{:+.3} s", ad.offset),
                    pal.text_primary,
                ));
                kpis.push((
                    "Confidence",
                    format!("{:.1}%", ad.confidence * 100.0),
                    conf_tier_color(ad.confidence, pal),
                ));
                kpis.push((
                    "PAL / Speed",
                    if ad.is_pal {
                        "PAL 25->23.976".to_string()
                    } else {
                        "Native 1.0x".to_string()
                    },
                    if ad.is_pal { pal.warning } else { pal.success },
                ));
            } else {
                kpis.push((
                    "Status",
                    if rep.skipped {
                        "Skipped (Existing)".to_string()
                    } else {
                        "Main Audio Only".to_string()
                    },
                    pal.success,
                ));
            }

            render_kpi_row(ui, &kpis, avail_w, pal);
            ui.add_space(10.0);

            // 2. AD Tracks & Deliverables Table Card
            Frame::none()
                .fill(pal.surface)
                .rounding(Rounding::same(8.0))
                .stroke(Stroke::new(1.0_f32, pal.border))
                .inner_margin(Margin::same(12.0))
                .show(ui, |ui| {
                    ui.set_width((avail_w - 26.0).max(200.0));
                    ui.label(
                        RichText::new("Synchronized Audio Description Tracks & Deliverables")
                            .size(13.5)
                            .strong()
                            .color(pal.text_primary),
                    );
                    ui.add_space(6.0);

                    if rep.ad_results.is_empty() {
                        ui.label(
                            RichText::new(if rep.skipped {
                                "Deliverables already existed in output/ and were preserved (--overwrite disabled)."
                            } else {
                                "No AD tracks in this job (Main Audio conversion only)."
                            })
                            .color(pal.text_muted),
                        );
                    } else {
                        for (idx, ad) in rep.ad_results.iter().enumerate() {
                            let is_sel = idx == state.selected_ad_idx;
                            let row_fill = if is_sel {
                                Color32::from_rgba_unmultiplied(
                                    pal.accent.r(),
                                    pal.accent.g(),
                                    pal.accent.b(),
                                    if pal.dark { 40 } else { 25 },
                                )
                            } else {
                                pal.well
                            };
                            let stroke_col = if is_sel { pal.accent } else { pal.border };

                            let resp = Frame::none()
                                .fill(row_fill)
                                .rounding(Rounding::same(6.0))
                                .stroke(Stroke::new(1.0_f32, stroke_col))
                                .inner_margin(Margin::symmetric(10.0, 8.0))
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            RichText::new(format!("#{} [{}]", idx + 1, ad.name))
                                                .size(13.0)
                                                .strong()
                                                .color(pal.text_primary),
                                        );
                                        ui.separator();
                                        ui.label(
                                            RichText::new(format!(
                                                "Method: {}",
                                                ad.method.to_ascii_uppercase()
                                            ))
                                            .size(12.0)
                                            .monospace()
                                            .color(pal.accent_hover),
                                        );
                                        ui.separator();
                                        ui.label(
                                            RichText::new(format!("Offset: {:+.3}s", ad.offset))
                                                .size(12.0)
                                                .monospace()
                                                .color(pal.text_primary),
                                        );
                                        ui.separator();
                                        ui.label(
                                            RichText::new(format!(
                                                "Confidence: {:.1}%",
                                                ad.confidence * 100.0
                                            ))
                                            .size(12.0)
                                            .monospace()
                                            .color(conf_tier_color(ad.confidence, pal)),
                                        );
                                        if ad.is_pal {
                                            ui.separator();
                                            ui.label(
                                                RichText::new("PAL Slowdown")
                                                    .size(12.0)
                                                    .color(pal.warning),
                                            );
                                        }
                                        if !ad.warp_cuts.is_empty() {
                                            ui.separator();
                                            ui.label(
                                                RichText::new(format!(
                                                    "{} Warp Blocks",
                                                    ad.warp_cuts.len()
                                                ))
                                                .size(12.0)
                                                .color(pal.text_muted),
                                            );
                                        }
                                        if !ad.filled_gaps.is_empty() {
                                            ui.separator();
                                            ui.label(
                                                RichText::new(format!(
                                                    "{} Master Gaps Filled",
                                                    ad.filled_gaps.len()
                                                ))
                                                .size(12.0)
                                                .color(pal.success),
                                            );
                                        }
                                    });
                                })
                                .response
                                .interact(Sense::click())
                                .on_hover_cursor(CursorIcon::PointingHand);

                            if resp.clicked() {
                                state.selected_ad_idx = idx;
                            }
                            ui.add_space(4.0);
                        }
                    }
                });

            ui.add_space(10.0);

            // 3. Interactive Offset & Warp Spline Timeline Chart
            if let Some(ad) = rep.ad_results.get(state.selected_ad_idx) {
                render_alignment_chart_card(
                    ui,
                    ad,
                    rep.target_duration,
                    avail_w,
                    viewport_h,
                    content_top_y,
                    pal,
                );
            }
        });
}

fn render_alignment_chart_card(
    ui: &mut egui::Ui,
    ad: &TrackSyncResult,
    target_duration: f64,
    avail_w: f32,
    viewport_h: f32,
    content_top_y: f32,
    pal: &VisualColors,
) {
    let pad_x = 14.0_f32;
    Frame::none()
        .fill(pal.surface)
        .rounding(Rounding::same(8.0))
        .stroke(Stroke::new(1.0_f32, pal.border))
        .inner_margin(Margin::symmetric(pad_x, 12.0))
        .show(ui, |ui| {
            let inner_w = (avail_w - pad_x * 2.0 - 2.0).max(200.0);
            ui.set_width(inner_w);

            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!(
                        "Acoustic Alignment & Warp Offset Timeline -- [{}]",
                        ad.name
                    ))
                    .size(14.0)
                    .strong()
                    .color(pal.text_primary),
                );
            });
            ui.add_space(4.0);

            let used_above = ui.cursor().top() - content_top_y;
            let chart_h = (viewport_h - used_above - 28.0).clamp(180.0, 320.0);
            paint_offset_timeline_chart(ui, ad, target_duration, inner_w, chart_h, pal);

            // If structural cuts or filled gaps exist, list them concisely beneath the chart
            if ad.warp_cuts.len() > 1 || !ad.filled_gaps.is_empty() {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    for cut in ad.warp_cuts.iter().take(12) {
                        let txt = if cut.is_initial {
                            format!(
                                "Seg 1 @ {}: {:+.2}s",
                                format_time_code(cut.vid_start),
                                cut.offset
                            )
                        } else {
                            format!(
                                "Seg {} @ {}: {:+.2}s ({:+.2}s)",
                                cut.index,
                                format_time_code(cut.vid_start),
                                cut.offset,
                                cut.jump
                            )
                        };
                        Frame::none()
                            .fill(pal.well)
                            .rounding(Rounding::same(5.0))
                            .stroke(Stroke::new(1.0_f32, pal.border))
                            .inner_margin(Margin::symmetric(8.0, 4.0))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(txt)
                                        .size(11.5)
                                        .monospace()
                                        .color(pal.text_primary),
                                );
                            });
                    }
                });
            }
        });
}

fn paint_offset_timeline_chart(
    ui: &mut egui::Ui,
    ad: &TrackSyncResult,
    target_duration: f64,
    width: f32,
    height: f32,
    pal: &VisualColors,
) {
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
        Pos2::new(rect.left() + 56.0, rect.top() + 24.0),
        Pos2::new(rect.right() - 16.0, rect.bottom() - 24.0),
    );
    let total_t = target_duration.max(1.0) as f32;

    // Collect offset range from anchor measurements and warp points
    let mut min_off = ad.offset - 1.0;
    let mut max_off = ad.offset + 1.0;
    for m in &ad.anchor_measurements {
        if m.confidence >= 0.12 {
            min_off = min_off.min(m.offset_sec - 0.5);
            max_off = max_off.max(m.offset_sec + 0.5);
        }
    }
    for p in &ad.warp_points {
        let off = p.target_time - p.source_time;
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

    // Grid lines
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

    // Highlight filled master audio gaps
    for gap in &ad.filled_gaps {
        let x0 = x_for_t(gap.start_sec);
        let x1 = x_for_t(gap.end_sec).max(x0 + 2.0);
        let gap_rect = Rect::from_min_max(Pos2::new(x0, plot.top()), Pos2::new(x1, plot.bottom()));
        painter.rect_filled(
            gap_rect,
            Rounding::ZERO,
            Color32::from_rgba_unmultiplied(pal.warning.r(), pal.warning.g(), pal.warning.b(), 38),
        );
    }

    // Draw consensus baseline offset line
    let y_base = y_for_off(ad.offset);
    painter.line_segment(
        [
            Pos2::new(plot.left(), y_base),
            Pos2::new(plot.right(), y_base),
        ],
        Stroke::new(1.5_f32, pal.accent),
    );

    // Draw warp path polyline if present
    if ad.warp_points.len() >= 2 {
        let pts: Vec<Pos2> = ad
            .warp_points
            .iter()
            .map(|p| {
                let off = p.target_time - p.source_time;
                Pos2::new(x_for_t(p.target_time), y_for_off(off))
            })
            .collect();
        painter.add(egui::Shape::line(
            pts,
            Stroke::new(2.0_f32, pal.accent_hover),
        ));
    }

    // Draw correlation anchor dots colored by confidence
    for m in &ad.anchor_measurements {
        let pos = Pos2::new(x_for_t(m.anchor_sec), y_for_off(m.offset_sec));
        if plot.contains(pos) {
            let col = conf_tier_color(m.confidence, pal);
            painter.circle_filled(pos, 3.2, col);
        }
    }

    // Hover crosshair tooltip
    if let Some(hpos) = resp.hover_pos() {
        if plot.contains(hpos) {
            let hover_t = ((hpos.x - plot.left()) / plot.width()) * total_t;
            painter.line_segment(
                [
                    Pos2::new(hpos.x, plot.top()),
                    Pos2::new(hpos.x, plot.bottom()),
                ],
                Stroke::new(1.0_f32, pal.text_muted),
            );
            let _ = resp.on_hover_text(format!(
                "Timeline: {}\nConsensus Offset: {:+.3}s",
                format_time_code(hover_t as f64),
                ad.offset
            ));
        }
    }
}

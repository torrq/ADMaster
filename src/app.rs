use crate::config::{
    AppConfig, PalMode, SyncMode, ThemeMode, DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH,
    MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
};
use crate::icon_svg::SvgIconCache;
use crate::media::ToolPaths;
use crate::pipeline::{spawn_pipeline_job, JobTarget, MovieJobReport, PipelineEvent};
use crate::visual_ui::{render_visual_results, ResultsTab, VisualColors, VisualState};
use crossbeam_channel::{unbounded, Receiver};
use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, FontId, Frame, Key, Layout, Margin, Modifiers, Pos2,
    Rect, RichText, Rounding, Sense, Stroke, Vec2,
};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveView {
    Workspace,
    Settings,
    About,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AboutTab {
    Overview,
    CreditsAndLicenses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    Alignment,
    Deliverables,
    ToolPaths,
    Appearance,
}

impl SettingsSection {
    fn all() -> &'static [SettingsSection] {
        &[
            SettingsSection::Alignment,
            SettingsSection::Deliverables,
            SettingsSection::ToolPaths,
            SettingsSection::Appearance,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            SettingsSection::Alignment => "Alignment & Warping",
            SettingsSection::Deliverables => "Encoding & Deliverables",
            SettingsSection::ToolPaths => "Binaries & Tool Paths",
            SettingsSection::Appearance => "Appearance & Scratch",
        }
    }

    fn description(self) -> &'static str {
        match self {
            SettingsSection::Alignment => "Warp, anchors & PAL probe",
            SettingsSection::Deliverables => "Downmix, loudnorm & MKV",
            SettingsSection::ToolPaths => "FFmpeg & MKVToolNix paths",
            SettingsSection::Appearance => "Theme, threads & scratch",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub canvas: Color32,
    pub surface: Color32,
    pub surface_alt: Color32,
    pub well: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub cyan_track: Color32,
    pub text_primary: Color32,
    pub text_muted: Color32,
    pub pill_active_fill: Color32,
    pub pill_active_text: Color32,
    pub pill_inactive_fill: Color32,
    pub pill_inactive_text: Color32,
    pub pill_inactive_border: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
}

impl Palette {
    pub fn to_visual_colors(self) -> VisualColors {
        VisualColors {
            dark: self.dark,
            surface: self.surface,
            surface_alt: self.surface_alt,
            well: self.well,
            border: self.border,
            accent: self.accent,
            accent_hover: self.accent_hover,
            text_primary: self.text_primary,
            text_muted: self.text_muted,
            success: self.success,
            warning: self.warning,
            error: self.error,
        }
    }

    pub fn for_theme(dark: bool) -> Self {
        if dark {
            Self {
                dark: true,
                canvas: Color32::from_rgb(21, 21, 21),
                surface: Color32::from_rgb(32, 32, 32),
                surface_alt: Color32::from_rgb(43, 43, 43),
                well: Color32::from_rgb(16, 16, 16),
                border: Color32::from_rgb(58, 58, 58),
                accent: Color32::from_rgb(50, 142, 255),
                accent_hover: Color32::from_rgb(91, 175, 255),
                cyan_track: Color32::from_rgb(56, 204, 248),
                text_primary: Color32::from_rgb(236, 239, 246),
                text_muted: Color32::from_rgb(167, 167, 167),
                pill_active_fill: Color32::from_rgb(17, 107, 255),
                pill_active_text: Color32::WHITE,
                pill_inactive_fill: Color32::from_rgb(41, 41, 41),
                pill_inactive_text: Color32::from_rgb(190, 196, 210),
                pill_inactive_border: Color32::from_rgb(72, 72, 72),
                success: Color32::from_rgb(176, 232, 115),
                warning: Color32::from_rgb(245, 166, 35),
                error: Color32::from_rgb(248, 113, 113),
            }
        } else {
            Self {
                dark: false,
                canvas: Color32::from_rgb(226, 237, 251),
                surface: Color32::from_rgb(247, 251, 255),
                surface_alt: Color32::from_rgb(216, 232, 249),
                well: Color32::from_rgb(247, 251, 255),
                border: Color32::from_rgb(196, 214, 237),
                accent: Color32::from_rgb(17, 135, 255),
                accent_hover: Color32::from_rgb(0, 109, 245),
                cyan_track: Color32::from_rgb(8, 145, 178),
                text_primary: Color32::from_rgb(22, 26, 36),
                text_muted: Color32::from_rgb(83, 105, 132),
                pill_active_fill: Color32::from_rgb(17, 135, 255),
                pill_active_text: Color32::WHITE,
                pill_inactive_fill: Color32::from_rgb(220, 233, 249),
                pill_inactive_text: Color32::from_rgb(55, 65, 81),
                pill_inactive_border: Color32::from_rgb(185, 209, 231),
                success: Color32::from_rgb(21, 128, 61),
                warning: Color32::from_rgb(180, 110, 0),
                error: Color32::from_rgb(185, 28, 28),
            }
        }
    }
}

fn to_egui_theme_preference(mode: ThemeMode) -> egui::ThemePreference {
    match mode {
        ThemeMode::System => egui::ThemePreference::System,
        ThemeMode::Dark => egui::ThemePreference::Dark,
        ThemeMode::Light => egui::ThemePreference::Light,
    }
}

fn build_visuals(dark: bool) -> egui::Visuals {
    let pal = Palette::for_theme(dark);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    v.panel_fill = pal.canvas;
    v.window_fill = pal.surface;
    v.faint_bg_color = pal.surface_alt;
    v.extreme_bg_color = pal.well;
    v.code_bg_color = pal.surface_alt;

    v.window_rounding = Rounding::same(10.0);
    v.menu_rounding = Rounding::same(8.0);
    v.window_stroke = Stroke::new(1.0_f32, pal.border);
    v.slider_trailing_fill = true;
    v.interact_cursor = Some(CursorIcon::PointingHand);

    v.selection.bg_fill = Color32::from_rgba_unmultiplied(
        pal.accent.r(),
        pal.accent.g(),
        pal.accent.b(),
        if dark { 85 } else { 50 },
    );
    v.selection.stroke = Stroke::new(1.0_f32, pal.text_primary);

    let r = Rounding::same(6.0);
    v.widgets.noninteractive.bg_fill = pal.surface;
    v.widgets.noninteractive.weak_bg_fill = pal.surface;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, pal.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, pal.text_primary);
    v.widgets.noninteractive.rounding = r;

    v.widgets.inactive.bg_fill = pal.surface_alt;
    v.widgets.inactive.weak_bg_fill = pal.surface_alt;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, pal.border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, pal.text_primary);
    v.widgets.inactive.rounding = r;

    let hover_fill = if dark {
        Color32::from_rgb(54, 54, 54)
    } else {
        Color32::from_rgb(202, 222, 246)
    };
    v.widgets.hovered.bg_fill = hover_fill;
    v.widgets.hovered.weak_bg_fill = hover_fill;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, pal.text_primary);
    v.widgets.hovered.rounding = r;

    let active_fill = if dark {
        Color32::from_rgb(66, 66, 66)
    } else {
        Color32::from_rgb(187, 213, 244)
    };
    v.widgets.active.bg_fill = active_fill;
    v.widgets.active.weak_bg_fill = active_fill;
    v.widgets.active.bg_stroke = Stroke::new(1.5_f32, pal.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, pal.text_primary);
    v.widgets.active.rounding = r;

    v.widgets.open.bg_fill = active_fill;
    v.widgets.open.weak_bg_fill = active_fill;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    v.widgets.open.rounding = r;

    v
}

fn configure_theme_styles(ctx: &egui::Context, theme_mode: ThemeMode) {
    ctx.set_visuals_of(egui::Theme::Dark, build_visuals(true));
    ctx.set_visuals_of(egui::Theme::Light, build_visuals(false));
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.spacing.window_margin = Margin::same(14.0);
        style.spacing.scroll.bar_width = 8.0;
    });
    ctx.set_theme(to_egui_theme_preference(theme_mode));
}

fn primary_button<'a>(label: &'a str, enabled: bool, pal: &Palette) -> egui::Button<'a> {
    if enabled {
        egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
            .fill(pal.accent)
            .stroke(Stroke::new(1.0_f32, pal.accent))
            .rounding(Rounding::same(6.0))
    } else {
        egui::Button::new(RichText::new(label).color(pal.text_muted))
            .fill(pal.surface_alt)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .rounding(Rounding::same(6.0))
    }
}

fn pill_toggle_button(
    ui: &mut egui::Ui,
    label: &str,
    active: &mut bool,
    tooltip: &str,
    pal: &Palette,
) -> bool {
    let (fill, text_col, border_col) = if *active {
        (
            pal.pill_active_fill,
            pal.pill_active_text,
            pal.pill_active_fill,
        )
    } else {
        (
            pal.pill_inactive_fill,
            pal.pill_inactive_text,
            pal.pill_inactive_border,
        )
    };
    let btn = egui::Button::new(RichText::new(label).size(12.0).strong().color(text_col))
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, border_col))
        .rounding(Rounding::same(13.0))
        .min_size(Vec2::new(0.0, 26.0));
    if ui.add(btn).on_hover_text(tooltip).clicked() {
        *active = !*active;
        true
    } else {
        false
    }
}

fn settings_section_header(ui: &mut egui::Ui, pal: &Palette, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(20.0).strong().color(pal.text_primary));
    ui.add_space(2.0);
    ui.label(RichText::new(subtitle).size(13.0).color(pal.text_muted));
    ui.add_space(12.0);
}

fn settings_card(
    ui: &mut egui::Ui,
    pal: &Palette,
    title: &str,
    subtitle: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    Frame::none()
        .fill(pal.surface)
        .rounding(Rounding::same(10.0))
        .stroke(Stroke::new(1.0_f32, pal.border))
        .inner_margin(Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(15.0).strong().color(pal.text_primary));
            ui.label(RichText::new(subtitle).size(12.0).color(pal.text_muted));
            ui.add_space(10.0);
            add_contents(ui);
        });
}

fn setting_row(
    ui: &mut egui::Ui,
    pal: &Palette,
    title: &str,
    subtitle: &str,
    tooltip: &str,
    add_control: impl FnOnce(&mut egui::Ui),
) {
    let avail_w = ui.available_width();
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width((avail_w - 260.0).max(160.0));
            ui.label(RichText::new(title).strong().color(pal.text_primary))
                .on_hover_text(tooltip);
            ui.label(RichText::new(subtitle).size(12.0).color(pal.text_muted))
                .on_hover_text(tooltip);
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            add_control(ui);
        });
    });
}

fn checkbox_row_tip(
    ui: &mut egui::Ui,
    value: &mut bool,
    title: &str,
    subtitle: &str,
    tooltip: &str,
    pal: &Palette,
) -> bool {
    let mut changed = false;
    ui.horizontal_top(|ui| {
        let cb_resp = ui.checkbox(value, "").on_hover_text(tooltip);
        changed |= cb_resp.changed();
        ui.add_space(4.0);
        let text_resp = ui.vertical(|ui| {
            ui.label(RichText::new(title).strong().color(pal.text_primary))
                .on_hover_text(tooltip);
            ui.label(RichText::new(subtitle).size(12.0).color(pal.text_muted))
                .on_hover_text(tooltip);
        });
        if text_resp
            .response
            .interact(Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(tooltip)
            .clicked()
        {
            *value = !*value;
            changed = true;
        }
    });
    changed
}

pub struct ADMasterApp {
    config: AppConfig,
    active_view: ActiveView,
    about_tab: AboutTab,
    settings_section: SettingsSection,

    // Active pipeline job state
    is_running: bool,
    job_queue: VecDeque<JobTarget>,
    job_rx: Option<Receiver<PipelineEvent>>,
    cancel_flag: Option<Arc<AtomicBool>>,
    job_start_instant: Option<Instant>,
    progress_completed: usize,
    progress_total: usize,
    current_step_label: String,

    // Explicit input staging state
    explicit_ref: Option<PathBuf>,
    explicit_ads: Vec<(String, PathBuf)>,

    // Results & console log state
    status_message: String,
    last_was_error: bool,
    last_was_success: bool,
    console_log: String,
    reports: Vec<MovieJobReport>,
    visual_state: VisualState,
    copied_at: Option<Instant>,
    about_url_copied_at: Option<Instant>,

    // Rasterized SVG icon cache
    icon_cache: SvgIconCache,

    // Window geometry persistence
    window_frames_seen: u32,
    window_save_pending_at: Option<Instant>,
}

impl ADMasterApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: AppConfig,
        initial_target: Option<JobTarget>,
    ) -> Self {
        configure_theme_styles(&cc.egui_ctx, config.theme);
        let mut app = Self {
            config,
            active_view: ActiveView::Workspace,
            about_tab: AboutTab::Overview,
            settings_section: SettingsSection::Alignment,
            is_running: false,
            job_queue: VecDeque::new(),
            job_rx: None,
            cancel_flag: None,
            job_start_instant: None,
            progress_completed: 0,
            progress_total: 100,
            current_step_label: String::new(),
            explicit_ref: None,
            explicit_ads: Vec::new(),
            status_message:
                "Drop a reference .mkv and audio tracks (.mp3, .m4a, .flac, .aac, .wav) or a movie folder anywhere to begin."
                    .to_string(),
            last_was_error: false,
            last_was_success: false,
            console_log: String::new(),
            reports: Vec::new(),
            visual_state: VisualState::default(),
            copied_at: None,
            about_url_copied_at: None,
            icon_cache: SvgIconCache::default(),
            window_frames_seen: 0,
            window_save_pending_at: None,
        };
        if let Some(target) = initial_target {
            app.start_job(target);
        }
        app
    }

    fn reset_window_geometry(&mut self, ctx: &egui::Context) {
        self.config.window_x = None;
        self.config.window_y = None;
        self.config.window_width = DEFAULT_WINDOW_WIDTH;
        self.config.window_height = DEFAULT_WINDOW_HEIGHT;
        self.window_save_pending_at = None;
        self.window_frames_seen = 0;

        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
            DEFAULT_WINDOW_WIDTH,
            DEFAULT_WINDOW_HEIGHT,
        )));
        if let Some(mon) = ctx.input(|i| i.viewport().monitor_size) {
            let cx = ((mon.x - DEFAULT_WINDOW_WIDTH) * 0.5).max(0.0);
            let cy = ((mon.y - DEFAULT_WINDOW_HEIGHT) * 0.5).max(0.0);
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::new(cx, cy)));
        }
    }

    fn track_window_geometry(&mut self, ctx: &egui::Context) {
        self.window_frames_seen = self.window_frames_seen.saturating_add(1);
        if self.window_frames_seen < 3 {
            return;
        }
        let vp = ctx.input(|i| i.viewport().clone());
        if vp.minimized.unwrap_or(false)
            || vp.maximized.unwrap_or(false)
            || vp.fullscreen.unwrap_or(false)
        {
            return;
        }
        let mut updated = false;
        if let Some(inner) = vp.inner_rect {
            let new_w = inner.width().round();
            let new_h = inner.height().round();
            if new_w.is_finite()
                && new_h.is_finite()
                && new_w >= MIN_WINDOW_WIDTH
                && new_h >= MIN_WINDOW_HEIGHT
                && ((self.config.window_width - new_w).abs() > 1.0
                    || (self.config.window_height - new_h).abs() > 1.0)
            {
                self.config.window_width = new_w;
                self.config.window_height = new_h;
                updated = true;
            }
            if let Some(outer) = vp.outer_rect {
                let pos = outer.min;
                if pos.x.is_finite()
                    && pos.y.is_finite()
                    && (-4096.0..=16_384.0).contains(&pos.x)
                    && (-4096.0..=16_384.0).contains(&pos.y)
                {
                    let nx = pos.x.round();
                    let ny = pos.y.round();
                    let changed = match (self.config.window_x, self.config.window_y) {
                        (Some(ox), Some(oy)) => (ox - nx).abs() > 1.0 || (oy - ny).abs() > 1.0,
                        _ => true,
                    };
                    if changed {
                        self.config.window_x = Some(nx);
                        self.config.window_y = Some(ny);
                        updated = true;
                    }
                }
            }
        }
        if updated {
            self.window_save_pending_at = Some(Instant::now());
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if let Some(since) = self.window_save_pending_at {
            if since.elapsed() >= Duration::from_millis(450) {
                self.window_save_pending_at = None;
                let _ = self.config.save();
            }
        }
    }

    fn apply_and_save_theme(&mut self, ctx: &egui::Context, mode: ThemeMode) {
        self.config.theme = mode;
        ctx.set_theme(to_egui_theme_preference(mode));
        let _ = self.config.save();
    }

    fn pick_movie_folder(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.enqueue_or_start_job(JobTarget::Directory(dir));
        }
    }

    fn pick_batch_queue_folder(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.enqueue_or_start_job(JobTarget::BatchQueue(dir));
        }
    }

    fn pick_reference_file(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter(
                "Reference Video / Surround Master",
                &[
                    "mkv", "mp4", "m2ts", "ts", "mov", "dts", "dtshd", "thd", "ac3", "eac3",
                    "flac", "wav",
                ],
            )
            .pick_file()
        {
            self.explicit_ref = Some(p);
            self.update_staging_status();
        }
    }

    fn pick_ad_files(&mut self) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter(
                "Syncable Audio Tracks",
                &[
                    "mp3", "m4a", "flac", "aac", "ogg", "opus", "wav", "w64", "ac3", "eac3",
                    "mka", "dts", "thd",
                ],
            )
            .pick_files()
        {
            for f in files {
                if !self.explicit_ads.iter().any(|(_, ex)| ex == &f) {
                    let lbl = f
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("AD")
                        .to_string();
                    self.explicit_ads.push((lbl, f));
                }
            }
            self.update_staging_status();
        }
    }

    fn update_staging_status(&mut self) {
        self.last_was_error = false;
        self.last_was_success = false;
        let ref_str = self
            .explicit_ref
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("None");
        self.status_message = format!(
            "Staged Reference: {}  |  Tracks to Sync: {}",
            ref_str,
            self.explicit_ads.len()
        );
    }

    fn enqueue_or_start_job(&mut self, target: JobTarget) {
        if self.is_running {
            self.job_queue.push_back(target);
        } else {
            self.start_job(target);
        }
    }

    fn start_job(&mut self, target: JobTarget) {
        if self.is_running {
            return;
        }
        let label = target.display_label();
        let (tx, rx) = unbounded();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.active_view = ActiveView::Workspace;
        self.is_running = true;
        self.job_rx = Some(rx);
        self.cancel_flag = Some(cancel_flag.clone());
        self.job_start_instant = Some(Instant::now());
        self.progress_completed = 0;
        self.progress_total = 100;
        self.current_step_label = format!("Starting {}...", label);
        self.last_was_error = false;
        self.last_was_success = false;
        self.console_log.clear();
        self.reports.clear();
        self.visual_state.selected_movie_idx = 0;
        self.visual_state.selected_ad_idx = 0;
        self.status_message = format!("Processing {}...", label);

        spawn_pipeline_job(target, self.config.clone(), false, cancel_flag, tx);
    }

    fn cancel_active_job(&mut self) {
        if let Some(ref flag) = self.cancel_flag {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn poll_job_events(&mut self) {
        let mut finished = false;
        if let Some(ref rx) = self.job_rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    PipelineEvent::Started { label, .. } => {
                        self.status_message = format!("Running: {}", label);
                    }
                    PipelineEvent::LogLine(line) => {
                        if !self.console_log.is_empty() {
                            self.console_log.push('\n');
                        }
                        self.console_log.push_str(&line);
                    }
                    PipelineEvent::Progress {
                        step_label,
                        completed,
                        total,
                    } => {
                        self.current_step_label = step_label.clone();
                        self.progress_completed = completed;
                        self.progress_total = total.max(1);
                        self.status_message = step_label;
                    }
                    PipelineEvent::MovieCompleted(rep) => {
                        self.reports.push(rep);
                    }
                    PipelineEvent::BatchCompleted {
                        reports,
                        elapsed_secs,
                    } => {
                        if !reports.is_empty() {
                            self.reports = reports;
                        }
                        self.last_was_error = false;
                        self.last_was_success = true;
                        self.progress_completed = self.progress_total;
                        self.status_message = format!(
                            "Completed {} movie job(s) in {:.2}s",
                            self.reports.len(),
                            elapsed_secs
                        );
                        finished = true;
                    }
                    PipelineEvent::Cancelled => {
                        self.last_was_error = false;
                        self.last_was_success = false;
                        self.status_message = "Job cancelled.".to_string();
                        finished = true;
                    }
                    PipelineEvent::Error(err) => {
                        self.last_was_error = true;
                        self.last_was_success = false;
                        self.status_message = format!("Error: {}", err);
                        if !self.console_log.is_empty() {
                            self.console_log.push('\n');
                        }
                        self.console_log.push_str(&format!("[ERROR] {}", err));
                        finished = true;
                    }
                }
            }
        }
        if finished {
            self.is_running = false;
            self.job_rx = None;
            self.cancel_flag = None;
            self.job_start_instant = None;
            if let Some(next_job) = self.job_queue.pop_front() {
                self.start_job(next_job);
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for item in dropped {
            if let Some(p) = item.path {
                if p.is_dir() {
                    dirs.push(p);
                } else if p.is_file() {
                    files.push(p);
                }
            }
        }
        if !dirs.is_empty() {
            for d in dirs {
                self.enqueue_or_start_job(JobTarget::Directory(d));
            }
        } else if !files.is_empty() {
            if !self.is_running {
                self.active_view = ActiveView::Workspace;
                self.reports.clear();
                self.console_log.clear();
            }

            let mut added_ref = false;
            let mut added_ads = 0usize;
            let mut ignored = Vec::new();

            for f in files {
                let ext = f
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();

                let is_video_or_surround_master = matches!(
                    ext.as_str(),
                    "mkv" | "mp4" | "m2ts" | "ts" | "mov" | "avi" | "webm" | "dts" | "dtshd" | "thd"
                );
                let is_syncable_audio = matches!(
                    ext.as_str(),
                    "mp3" | "m4a" | "flac" | "aac" | "ogg" | "oga" | "opus" | "wav" | "w64" | "ac3" | "eac3" | "mka"
                );

                if is_video_or_surround_master {
                    self.explicit_ref = Some(f);
                    added_ref = true;
                } else if is_syncable_audio {
                    if !self.explicit_ads.iter().any(|(_, existing)| existing == &f) {
                        let lbl = f
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("AD")
                            .to_string();
                        self.explicit_ads.push((lbl, f));
                        added_ads += 1;
                    }
                } else {
                    ignored.push(
                        f.file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("file")
                            .to_string(),
                    );
                }
            }

            if added_ref || added_ads > 0 {
                self.update_staging_status();
            } else if !ignored.is_empty() {
                self.last_was_error = true;
                self.last_was_success = false;
                self.status_message = format!("Unsupported dropped file(s): {}", ignored.join(", "));
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let open_folder =
            ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::O));
        let toggle_settings = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Comma));
        let escape = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));

        if open_folder {
            self.pick_movie_folder();
        } else if toggle_settings {
            if self.active_view == ActiveView::Settings {
                self.active_view = ActiveView::Workspace;
            } else {
                self.active_view = ActiveView::Settings;
            }
        } else if escape {
            if self.is_running {
                self.cancel_active_job();
            } else if self.active_view != ActiveView::Workspace {
                self.active_view = ActiveView::Workspace;
            }
        }
    }

    fn render_top_toolbar(&mut self, ctx: &egui::Context, pal: &Palette) {
        let frame = Frame::none()
            .fill(pal.surface)
            .inner_margin(Margin::symmetric(14.0, 9.0))
            .stroke(Stroke::new(1.0_f32, pal.border));

        egui::TopBottomPanel::top("admaster_top_toolbar")
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let title_galley = ui.painter().layout_no_wrap(
                        "ADMaster".to_string(),
                        FontId::proportional(17.5),
                        pal.text_primary,
                    );
                    let badge_sz = 22.0_f32;
                    let pad_x = 7.0_f32;
                    let gap = 7.0_f32;
                    let brand_w = pad_x + badge_sz + gap + title_galley.size().x + pad_x;
                    let (brand_rect, brand_resp) =
                        ui.allocate_exact_size(Vec2::new(brand_w, 30.0), Sense::click());
                    let is_about = self.active_view == ActiveView::About;
                    let brand_fill = if is_about {
                        Color32::from_rgba_unmultiplied(
                            pal.accent.r(),
                            pal.accent.g(),
                            pal.accent.b(),
                            if pal.dark { 42 } else { 24 },
                        )
                    } else if brand_resp.hovered() {
                        pal.surface_alt
                    } else {
                        Color32::TRANSPARENT
                    };
                    let brand_stroke = if is_about {
                        Stroke::new(1.0_f32, pal.accent)
                    } else if brand_resp.hovered() {
                        Stroke::new(1.0_f32, pal.border)
                    } else {
                        Stroke::NONE
                    };
                    ui.painter()
                        .rect(brand_rect, Rounding::same(6.0), brand_fill, brand_stroke);
                    let badge_rect = Rect::from_min_size(
                        Pos2::new(
                            brand_rect.left() + pad_x,
                            brand_rect.center().y - badge_sz * 0.5,
                        ),
                        Vec2::splat(badge_sz),
                    );
                    self.icon_cache.paint_app_icon(ctx, ui.painter(), badge_rect);
                    ui.painter().galley(
                        Pos2::new(
                            badge_rect.right() + gap,
                            brand_rect.center().y - title_galley.size().y * 0.5,
                        ),
                        title_galley,
                        pal.text_primary,
                    );
                    if brand_resp
                        .on_hover_cursor(CursorIcon::PointingHand)
                        .on_hover_text(format!(
                            "About ADMaster v{} -- Audio Description Sync & Mastering Studio",
                            env!("CARGO_PKG_VERSION")
                        ))
                        .clicked()
                    {
                        self.active_view = if is_about {
                            ActiveView::Workspace
                        } else {
                            ActiveView::About
                        };
                    }

                    ui.separator();

                    if ui
                        .add(
                            primary_button("Open Movie Folder...", true, pal)
                                .min_size(Vec2::new(0.0, 28.0)),
                        )
                        .on_hover_text("Auto-discover source/ and tosync/ inside a movie folder (Ctrl+Shift+O)")
                        .clicked()
                    {
                        self.pick_movie_folder();
                    }

                    if ui
                        .add(egui::Button::new("Batch Queue...").min_size(Vec2::new(0.0, 28.0)))
                        .on_hover_text("Process all movie folders inside a parent queue directory")
                        .clicked()
                    {
                        self.pick_batch_queue_folder();
                    }

                    if self.is_running
                        && ui
                            .add(
                                egui::Button::new(RichText::new("Cancel Job").color(pal.error))
                                    .min_size(Vec2::new(0.0, 28.0)),
                            )
                            .clicked()
                    {
                        self.cancel_active_job();
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.active_view != ActiveView::Workspace {
                            if ui
                                .add(
                                    primary_button("Back to Workspace", true, pal)
                                        .min_size(Vec2::new(0.0, 28.0)),
                                )
                                .on_hover_text("Return to main workspace (Esc)")
                                .clicked()
                            {
                                self.active_view = ActiveView::Workspace;
                            }
                            if self.active_view == ActiveView::About
                                && ui
                                    .add(
                                        egui::Button::new("Settings")
                                            .min_size(Vec2::new(0.0, 28.0)),
                                    )
                                    .clicked()
                            {
                                self.active_view = ActiveView::Settings;
                            }
                        } else if ui
                            .add(egui::Button::new("Settings").min_size(Vec2::new(0.0, 28.0)))
                            .on_hover_text("Configure alignment, downmix, loudnorm & tool paths (Ctrl+,)")
                            .clicked()
                        {
                            self.active_view = ActiveView::Settings;
                        }

                        let theme_label = match self.config.theme {
                            ThemeMode::System => {
                                let eff = if pal.dark { "Dark" } else { "Light" };
                                format!("Theme: Auto ({})", eff)
                            }
                            ThemeMode::Dark => "Theme: Dark".to_string(),
                            ThemeMode::Light => "Theme: Light".to_string(),
                        };
                        if ui
                            .add(egui::Button::new(theme_label).min_size(Vec2::new(0.0, 28.0)))
                            .on_hover_text("Cycle color theme: System Auto -> Dark -> Light")
                            .clicked()
                        {
                            let next = self.config.theme.next();
                            self.apply_and_save_theme(ctx, next);
                        }

                        if self.active_view == ActiveView::Workspace
                            && (!self.console_log.is_empty()
                                || !self.reports.is_empty()
                                || self.explicit_ref.is_some()
                                || !self.explicit_ads.is_empty())
                        {
                            if ui
                                .add_enabled(
                                    !self.is_running,
                                    egui::Button::new("Reset").min_size(Vec2::new(0.0, 28.0)),
                                )
                                .on_hover_text("Clear staged files and results")
                                .clicked()
                            {
                                self.console_log.clear();
                                self.reports.clear();
                                self.explicit_ref = None;
                                self.explicit_ads.clear();
                                self.status_message = "Ready. Drop a reference .mkv and audio tracks (.mp3, .m4a, .flac) to begin.".to_string();
                            }
                            if !self.console_log.is_empty()
                                && ui
                                    .add(egui::Button::new("Copy Log").min_size(Vec2::new(0.0, 28.0)))
                                    .clicked()
                            {
                                ctx.copy_text(self.console_log.clone());
                                self.copied_at = Some(Instant::now());
                            }
                        }
                    });
                });
            });
    }

    fn render_bottom_status_bar(&mut self, ctx: &egui::Context, pal: &Palette) {
        let frame = Frame::none()
            .fill(pal.surface)
            .inner_margin(Margin::symmetric(14.0, 9.0))
            .stroke(Stroke::new(1.0_f32, pal.border));

        egui::TopBottomPanel::bottom("admaster_bottom_status")
            .frame(frame)
            .show(ctx, |ui| {
                let dot_color = if self.is_running {
                    pal.accent
                } else if self.last_was_error {
                    pal.error
                } else if self.last_was_success {
                    pal.success
                } else {
                    pal.text_muted
                };

                if self.is_running {
                    let total = self.progress_total.max(1);
                    let completed = self.progress_completed.min(total);
                    let raw_frac = (completed as f32) / (total as f32);
                    let anim_frac = ctx.animate_value_with_time(
                        egui::Id::new("admaster_progress_bar"),
                        raw_frac,
                        0.14,
                    );
                    let pct = (raw_frac * 100.0).round().clamp(0.0, 100.0) as u32;
                    let elapsed = self
                        .job_start_instant
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or(0.0);

                    ui.horizontal(|ui| {
                        let (dot_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                        ui.painter()
                            .circle_filled(dot_rect.center(), 4.2, dot_color);
                        ui.label(
                            RichText::new(&self.status_message)
                                .size(13.5)
                                .color(pal.text_primary),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.1}s", elapsed))
                                    .size(13.0)
                                    .monospace()
                                    .color(pal.text_muted),
                            );
                            ui.label(
                                RichText::new(format!("{}%", pct))
                                    .size(13.0)
                                    .strong()
                                    .color(pal.accent_hover),
                            );
                        });
                    });

                    ui.add_space(6.0);
                    let bar_w = ui.available_width().max(60.0);
                    let (bar_rect, _) =
                        ui.allocate_exact_size(Vec2::new(bar_w, 8.0), Sense::hover());
                    ui.painter().rect(
                        bar_rect,
                        Rounding::same(4.0),
                        pal.well,
                        Stroke::new(1.0_f32, pal.border),
                    );
                    let inner = bar_rect.shrink(1.0);
                    let fill_w = (inner.width() * anim_frac.clamp(0.02, 1.0)).max(4.0);
                    let fill_rect = Rect::from_min_max(
                        inner.min,
                        Pos2::new((inner.left() + fill_w).min(inner.right()), inner.bottom()),
                    );
                    ui.painter()
                        .rect_filled(fill_rect, Rounding::same(3.5), pal.accent);
                } else {
                    ui.horizontal(|ui| {
                        let (dot_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                        ui.painter()
                            .circle_filled(dot_rect.center(), 4.2, dot_color);
                        let msg_col = if self.last_was_error {
                            pal.error
                        } else {
                            pal.text_primary
                        };
                        ui.label(RichText::new(&self.status_message).size(13.5).color(msg_col));
                    });
                }
            });
    }

    fn render_workspace_view(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        // 1. Clean, symmetrical Profile Pill Bar at the top
        Frame::none()
            .fill(pal.surface)
            .rounding(Rounding::same(8.0))
            .stroke(Stroke::new(1.0_f32, pal.border))
            .inner_margin(Margin::symmetric(14.0, 9.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(
                        RichText::new("Alignment:")
                            .size(12.0)
                            .strong()
                            .color(pal.text_muted),
                    );
                    egui::ComboBox::from_id_salt("ws_sync_mode")
                        .width(115.0)
                        .selected_text(self.config.sync_mode.short_label())
                        .show_ui(ui, |ui| {
                            for &m in SyncMode::all() {
                                if ui
                                    .selectable_value(&mut self.config.sync_mode, m, m.label())
                                    .changed()
                                {
                                    let _ = self.config.save();
                                }
                            }
                        });

                    ui.label(
                        RichText::new("PAL:")
                            .size(12.0)
                            .strong()
                            .color(pal.text_muted),
                    );
                    egui::ComboBox::from_id_salt("ws_pal_mode")
                        .width(105.0)
                        .selected_text(self.config.pal_mode.short_label())
                        .show_ui(ui, |ui| {
                            for &pm in PalMode::all() {
                                if ui
                                    .selectable_value(&mut self.config.pal_mode, pm, pm.label())
                                    .changed()
                                {
                                    let _ = self.config.save();
                                }
                            }
                        });

                    ui.separator();
                    let mut changed = false;
                    changed |= pill_toggle_button(
                        ui,
                        "Convert Main 5.1/2.0",
                        &mut self.config.convert_main,
                        "Also downmix and convert the main reference soundtrack (Surround AC3 + True 2.0 Stereo + Night Mode)",
                        pal,
                    );
                    changed |= pill_toggle_button(
                        ui,
                        "Night Mode (-16 LUFS)",
                        &mut self.config.generate_night_mode,
                        "Generate EBU R128 loudness-normalized Night Mode deliverables",
                        pal,
                    );
                    changed |= pill_toggle_button(
                        ui,
                        "Mux Final MKV",
                        &mut self.config.mux_final_mkv,
                        "Multiplex video + main stereo + synced AD stereo + English subtitles into a final MKV",
                        pal,
                    );
                    changed |= pill_toggle_button(
                        ui,
                        "WAV Output",
                        &mut self.config.wav_output,
                        "Output uncompressed 16-bit 48kHz PCM WAV instead of AAC (.m4a) for rapid sync testing",
                        pal,
                    );
                    changed |= pill_toggle_button(
                        ui,
                        "Overwrite",
                        &mut self.config.overwrite,
                        "Overwrite existing deliverables in output/",
                        pal,
                    );
                    if changed {
                        let _ = self.config.save();
                    }
                });
            });

        ui.add_space(12.0);

        // If no job is running and no results are active, show the Two-Column Color-Coded Staging Studio
        if self.reports.is_empty() && self.console_log.is_empty() && !self.is_running {
            self.render_staging_studio(ui, pal);
            return;
        }

        // View switcher tabs: Visual Inspector vs Console Log
        ui.horizontal(|ui| {
            for (tab, lbl) in [
                (ResultsTab::VisualInspector, "Visual Timeline & Deliverables"),
                (ResultsTab::ConsoleLog, "Console Sync Log"),
            ] {
                let active = self.visual_state.tab == tab;
                let fill = if active {
                    pal.pill_active_fill
                } else {
                    pal.pill_inactive_fill
                };
                let text_col = if active {
                    pal.pill_active_text
                } else {
                    pal.pill_inactive_text
                };
                if ui
                    .add(
                        egui::Button::new(RichText::new(lbl).size(12.5).strong().color(text_col))
                            .fill(fill)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(170.0, 28.0)),
                    )
                    .clicked()
                {
                    self.visual_state.tab = tab;
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !self.is_running
                    && ui
                        .add(
                            egui::Button::new("Back to Drop Zones")
                                .min_size(Vec2::new(140.0, 28.0)),
                        )
                        .clicked()
                {
                    self.reports.clear();
                    self.console_log.clear();
                }
            });
        });
        ui.add_space(8.0);

        match self.visual_state.tab {
            ResultsTab::VisualInspector => {
                if self.reports.is_empty() && self.is_running {
                    self.render_console_log_panel(ui, pal);
                } else {
                    let vcols = pal.to_visual_colors();
                    render_visual_results(ui, &self.reports, &mut self.visual_state, &vcols);
                }
            }
            ResultsTab::ConsoleLog => {
                self.render_console_log_panel(ui, pal);
            }
        }
    }

    fn render_staging_studio(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        let avail_w = ui.available_width().max(640.0);
        let avail_h = ui.available_height().max(340.0);
        let action_bar_h = 64.0_f32;
        let cards_h = (avail_h - action_bar_h - 14.0).max(260.0);
        let col_gap = 14.0_f32;
        let col_w = ((avail_w - col_gap) * 0.5).max(280.0);

        // Zone 1 & Zone 2 Color-Coding Palette
        let ref_color = pal.accent; // Royal Blue (#328EFF) for Main Reference MKV
        let ad_color = pal.cyan_track; // Sky Cyan / Emerald (#38CCF8) for Tracks to Sync

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = col_gap;

            // =================================================================
            // LEFT ZONE: 1. REFERENCE VIDEO / MASTER (SYNC TO)
            // =================================================================
            let has_ref = self.explicit_ref.is_some();
            let ref_border = if has_ref {
                Stroke::new(1.5_f32, ref_color)
            } else {
                Stroke::new(1.0_f32, pal.border)
            };
            let ref_header_bg = Color32::from_rgba_unmultiplied(
                ref_color.r(),
                ref_color.g(),
                ref_color.b(),
                if pal.dark { 28 } else { 18 },
            );

            Frame::none()
                .fill(pal.surface)
                .rounding(Rounding::same(10.0))
                .stroke(ref_border)
                .inner_margin(Margin::same(0.0))
                .show(ui, |ui| {
                    ui.set_width(col_w);
                    ui.set_min_height(cards_h);

                    // Zone 1 Color-Coded Header Banner
                    Frame::none()
                        .fill(ref_header_bg)
                        .rounding(Rounding {
                            nw: 9.0,
                            ne: 9.0,
                            sw: 0.0,
                            se: 0.0,
                        })
                        .inner_margin(Margin::symmetric(16.0, 12.0))
                        .show(ui, |ui| {
                            ui.set_width(col_w - 32.0);
                            ui.horizontal(|ui| {
                                Frame::none()
                                    .fill(ref_color)
                                    .rounding(Rounding::same(5.0))
                                    .inner_margin(Margin::symmetric(7.0, 2.0))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new("ZONE 1")
                                                .size(10.5)
                                                .strong()
                                                .color(Color32::WHITE),
                                        );
                                    });
                                ui.label(
                                    RichText::new("Reference Master (Sync To)")
                                        .size(14.5)
                                        .strong()
                                        .color(pal.text_primary),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if has_ref && ui.small_button("Clear").clicked() {
                                        self.explicit_ref = None;
                                        self.update_staging_status();
                                    }
                                    if ui.button("Browse MKV / Master...").clicked() {
                                        self.pick_reference_file();
                                    }
                                });
                            });
                        });

                    // Zone 1 Body
                    Frame::none()
                        .inner_margin(Margin::same(16.0))
                        .show(ui, |ui| {
                            ui.set_width(col_w - 32.0);
                            let inner_well_h = (cards_h - 86.0).max(160.0);

                            if let Some(ref rp) = self.explicit_ref.clone() {
                                let fname = rp
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("Reference.mkv");
                                let parent_dir = rp
                                    .parent()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default();
                                let ext_upper = rp
                                    .extension()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("MKV")
                                    .to_ascii_uppercase();

                                Frame::none()
                                    .fill(pal.well)
                                    .rounding(Rounding::same(8.0))
                                    .stroke(Stroke::new(1.0_f32, ref_color))
                                    .inner_margin(Margin::same(16.0))
                                    .show(ui, |ui| {
                                        ui.set_width(col_w - 64.0);
                                        ui.set_min_height(inner_well_h - 32.0);
                                        ui.horizontal(|ui| {
                                            Frame::none()
                                                .fill(ref_color)
                                                .rounding(Rounding::same(5.0))
                                                .inner_margin(Margin::symmetric(8.0, 3.0))
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        RichText::new(&ext_upper)
                                                            .size(11.5)
                                                            .strong()
                                                            .monospace()
                                                            .color(Color32::WHITE),
                                                    );
                                                });
                                            ui.label(
                                                RichText::new("READY AS REFERENCE TARGET")
                                                    .size(11.5)
                                                    .strong()
                                                    .color(ref_color),
                                            );
                                        });
                                        ui.add_space(10.0);
                                        ui.label(
                                            RichText::new(fname)
                                                .size(15.0)
                                                .strong()
                                                .color(pal.text_primary),
                                        );
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new(parent_dir)
                                                .size(11.5)
                                                .monospace()
                                                .color(pal.text_muted),
                                        );
                                        ui.add_space(14.0);
                                        ui.separator();
                                        ui.add_space(8.0);
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new("Audio Stream:")
                                                    .size(12.5)
                                                    .color(pal.text_muted),
                                            );
                                            if ui
                                                .add(
                                                    egui::TextEdit::singleline(
                                                        &mut self.config.ref_stream,
                                                    )
                                                    .desired_width(80.0),
                                                )
                                                .changed()
                                            {
                                                let _ = self.config.save();
                                            }
                                            ui.label(
                                                RichText::new("(e.g. 0:a:0)")
                                                    .size(11.5)
                                                    .color(pal.text_muted),
                                            );
                                        });
                                    });
                            } else {
                                // Empty Drop Well for Reference MKV
                                let (well_rect, well_resp) = ui.allocate_exact_size(
                                    Vec2::new(col_w - 32.0, inner_well_h),
                                    Sense::click(),
                                );
                                let hovered = well_resp.hovered();
                                let stroke_c = if hovered { ref_color } else { pal.border };
                                ui.painter().rect(
                                    well_rect,
                                    Rounding::same(8.0),
                                    pal.well,
                                    Stroke::new(if hovered { 1.5_f32 } else { 1.0_f32 }, stroke_c),
                                );
                                let center = well_rect.center();
                                ui.painter().circle_filled(
                                    Pos2::new(center.x, center.y - 26.0),
                                    20.0,
                                    ref_header_bg,
                                );
                                ui.painter().text(
                                    Pos2::new(center.x, center.y - 26.0),
                                    Align2::CENTER_CENTER,
                                    "MKV",
                                    FontId::monospace(12.0),
                                    ref_color,
                                );
                                ui.painter().text(
                                    Pos2::new(center.x, center.y + 8.0),
                                    Align2::CENTER_CENTER,
                                    "Drop Main Reference .MKV Here",
                                    FontId::proportional(14.5),
                                    pal.text_primary,
                                );
                                ui.painter().text(
                                    Pos2::new(center.x, center.y + 30.0),
                                    Align2::CENTER_CENTER,
                                    "Supports .mkv, .mp4, .m2ts, .dts, .dtshd, .thd (or click to browse)",
                                    FontId::proportional(12.0),
                                    pal.text_muted,
                                );
                                if well_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    self.pick_reference_file();
                                }
                            }
                        });
                });

            // =================================================================
            // RIGHT ZONE: 2. AUDIO TRACKS TO SYNC (AD / COMMENTARY)
            // =================================================================
            let has_ads = !self.explicit_ads.is_empty();
            let ad_border = if has_ads {
                Stroke::new(1.5_f32, ad_color)
            } else {
                Stroke::new(1.0_f32, pal.border)
            };
            let ad_header_bg = Color32::from_rgba_unmultiplied(
                ad_color.r(),
                ad_color.g(),
                ad_color.b(),
                if pal.dark { 28 } else { 18 },
            );

            Frame::none()
                .fill(pal.surface)
                .rounding(Rounding::same(10.0))
                .stroke(ad_border)
                .inner_margin(Margin::same(0.0))
                .show(ui, |ui| {
                    ui.set_width(col_w);
                    ui.set_min_height(cards_h);

                    // Zone 2 Color-Coded Header Banner
                    Frame::none()
                        .fill(ad_header_bg)
                        .rounding(Rounding {
                            nw: 9.0,
                            ne: 9.0,
                            sw: 0.0,
                            se: 0.0,
                        })
                        .inner_margin(Margin::symmetric(16.0, 12.0))
                        .show(ui, |ui| {
                            ui.set_width(col_w - 32.0);
                            ui.horizontal(|ui| {
                                Frame::none()
                                    .fill(ad_color)
                                    .rounding(Rounding::same(5.0))
                                    .inner_margin(Margin::symmetric(7.0, 2.0))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new("ZONE 2")
                                                .size(10.5)
                                                .strong()
                                                .color(Color32::from_rgb(10, 16, 24)),
                                        );
                                    });
                                ui.label(
                                    RichText::new(format!(
                                        "Tracks to Sync ({})",
                                        self.explicit_ads.len()
                                    ))
                                    .size(14.5)
                                    .strong()
                                    .color(pal.text_primary),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if has_ads && ui.small_button("Clear All").clicked() {
                                        self.explicit_ads.clear();
                                        self.update_staging_status();
                                    }
                                    if ui.button("Add Audio Track(s)...").clicked() {
                                        self.pick_ad_files();
                                    }
                                });
                            });
                        });

                    // Zone 2 Body
                    Frame::none()
                        .inner_margin(Margin::same(16.0))
                        .show(ui, |ui| {
                            ui.set_width(col_w - 32.0);
                            let inner_well_h = (cards_h - 86.0).max(160.0);

                            if !self.explicit_ads.is_empty() {
                                Frame::none()
                                    .fill(pal.well)
                                    .rounding(Rounding::same(8.0))
                                    .stroke(Stroke::new(1.0_f32, ad_color))
                                    .inner_margin(Margin::same(12.0))
                                    .show(ui, |ui| {
                                        ui.set_width(col_w - 56.0);
                                        ui.set_min_height(inner_well_h - 24.0);

                                        egui::ScrollArea::vertical()
                                            .id_salt("staged_ads_scroll")
                                            .max_height(inner_well_h - 30.0)
                                            .show(ui, |ui| {
                                                let mut remove_idx = None;
                                                let mut promote_idx = None;
                                                for (idx, (lbl, path)) in
                                                    self.explicit_ads.iter().enumerate()
                                                {
                                                    let fname = path
                                                        .file_name()
                                                        .and_then(|s| s.to_str())
                                                        .unwrap_or(lbl);
                                                    let ext_badge = path
                                                        .extension()
                                                        .and_then(|s| s.to_str())
                                                        .unwrap_or("AUDIO")
                                                        .to_ascii_uppercase();

                                                    Frame::none()
                                                        .fill(pal.surface)
                                                        .rounding(Rounding::same(6.0))
                                                        .stroke(Stroke::new(1.0_f32, pal.border))
                                                        .inner_margin(Margin::symmetric(10.0, 8.0))
                                                        .show(ui, |ui| {
                                                            ui.set_width(ui.available_width());
                                                            ui.horizontal(|ui| {
                                                                Frame::none()
                                                                    .fill(ad_color)
                                                                    .rounding(Rounding::same(4.0))
                                                                    .inner_margin(
                                                                        Margin::symmetric(6.0, 1.5),
                                                                    )
                                                                    .show(ui, |ui| {
                                                                        ui.label(
                                                                            RichText::new(
                                                                                &ext_badge,
                                                                            )
                                                                            .size(10.5)
                                                                            .strong()
                                                                            .monospace()
                                                                            .color(
                                                                                Color32::from_rgb(
                                                                                    10, 16, 24,
                                                                                ),
                                                                            ),
                                                                        );
                                                                    });
                                                                ui.add(
                                                                    egui::Label::new(
                                                                        RichText::new(fname)
                                                                            .size(12.5)
                                                                            .strong()
                                                                            .color(
                                                                                pal.text_primary,
                                                                            ),
                                                                    )
                                                                    .truncate(),
                                                                );
                                                                ui.with_layout(
                                                                    Layout::right_to_left(
                                                                        Align::Center,
                                                                    ),
                                                                    |ui| {
                                                                        if ui
                                                                            .small_button("Remove")
                                                                            .clicked()
                                                                        {
                                                                            remove_idx = Some(idx);
                                                                        }
                                                                        if ui
                                                                            .small_button(
                                                                                "Set as Ref",
                                                                            )
                                                                            .on_hover_text(
                                                                                "Move to Zone 1 as Reference target",
                                                                            )
                                                                            .clicked()
                                                                        {
                                                                            promote_idx = Some(idx);
                                                                        }
                                                                    },
                                                                );
                                                            });
                                                        });
                                                    ui.add_space(6.0);
                                                }
                                                if let Some(pi) = promote_idx {
                                                    let (_, p) = self.explicit_ads.remove(pi);
                                                    self.explicit_ref = Some(p);
                                                    self.update_staging_status();
                                                } else if let Some(ri) = remove_idx {
                                                    self.explicit_ads.remove(ri);
                                                    self.update_staging_status();
                                                }
                                            });
                                    });
                            } else {
                                // Empty Drop Well for Audio Tracks to Sync
                                let (well_rect, well_resp) = ui.allocate_exact_size(
                                    Vec2::new(col_w - 32.0, inner_well_h),
                                    Sense::click(),
                                );
                                let hovered = well_resp.hovered();
                                let stroke_c = if hovered { ad_color } else { pal.border };
                                ui.painter().rect(
                                    well_rect,
                                    Rounding::same(8.0),
                                    pal.well,
                                    Stroke::new(if hovered { 1.5_f32 } else { 1.0_f32 }, stroke_c),
                                );
                                let center = well_rect.center();
                                ui.painter().circle_filled(
                                    Pos2::new(center.x, center.y - 26.0),
                                    20.0,
                                    ad_header_bg,
                                );
                                ui.painter().text(
                                    Pos2::new(center.x, center.y - 26.0),
                                    Align2::CENTER_CENTER,
                                    "AD",
                                    FontId::monospace(12.5),
                                    ad_color,
                                );
                                ui.painter().text(
                                    Pos2::new(center.x, center.y + 8.0),
                                    Align2::CENTER_CENTER,
                                    "Drop Audio Tracks to Sync Here",
                                    FontId::proportional(14.5),
                                    pal.text_primary,
                                );
                                ui.painter().text(
                                    Pos2::new(center.x, center.y + 30.0),
                                    Align2::CENTER_CENTER,
                                    "Supports .mp3, .m4a, .flac, .aac, .ogg, .wav, .ac3, .eac3",
                                    FontId::proportional(12.0),
                                    pal.text_muted,
                                );
                                if well_resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    self.pick_ad_files();
                                }
                            }
                        });
                });
        });

        ui.add_space(12.0);

        // =====================================================================
        // BOTTOM LAUNCH BAR
        // =====================================================================
        let can_start = self.explicit_ref.is_some()
            && (!self.explicit_ads.is_empty() || self.config.convert_main);

        Frame::none()
            .fill(pal.surface)
            .rounding(Rounding::same(10.0))
            .stroke(Stroke::new(1.0_f32, pal.border))
            .inner_margin(Margin::symmetric(18.0, 12.0))
            .show(ui, |ui| {
                ui.set_width(avail_w - 36.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        let summary = match (&self.explicit_ref, self.explicit_ads.len()) {
                            (Some(r), n) if n > 0 => format!(
                                "Ready to sync {} AD track(s) against '{}'",
                                n,
                                r.file_name().and_then(|s| s.to_str()).unwrap_or("Reference")
                            ),
                            (Some(r), 0) if self.config.convert_main => format!(
                                "Ready to downmix & convert Main Audio from '{}'",
                                r.file_name().and_then(|s| s.to_str()).unwrap_or("Reference")
                            ),
                            (Some(_), 0) => {
                                "Drop at least one audio track (.mp3, .m4a, .flac) into Zone 2 to sync."
                                    .to_string()
                            }
                            (None, n) if n > 0 => format!(
                                "{} track(s) queued in Zone 2 -- drop a Reference .MKV into Zone 1.",
                                n
                            ),
                            _ => "Tip: You can also drop an entire Movie Folder (with source/ & tosync/) anywhere."
                                .to_string(),
                        };
                        ui.label(
                            RichText::new(summary)
                                .size(13.0)
                                .strong()
                                .color(if can_start {
                                    pal.success
                                } else {
                                    pal.text_muted
                                }),
                        );
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                can_start,
                                primary_button("Start Synchronization & Render", can_start, pal)
                                    .min_size(Vec2::new(240.0, 36.0)),
                            )
                            .clicked()
                        {
                            if let Some(ref rp) = self.explicit_ref {
                                self.start_job(JobTarget::Explicit {
                                    ref_path: rp.clone(),
                                    ad_paths: self.explicit_ads.clone(),
                                    title: None,
                                    out_dir: None,
                                    scratch_dir: None,
                                    mux_video_path: None,
                                });
                            }
                        }
                    });
                });
            });
    }

    fn render_console_log_panel(&mut self, ui: &mut egui::Ui, pal: &Palette) {
        Frame::none()
            .fill(pal.well)
            .rounding(Rounding::same(8.0))
            .stroke(Stroke::new(1.0_f32, pal.border))
            .inner_margin(Margin::same(12.0))
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .id_salt("admaster_console_log_scroll")
                    .auto_shrink([false, false])
                    .stick_to_bottom(self.is_running)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&self.console_log)
                                    .font(FontId::monospace(12.0))
                                    .color(pal.text_primary),
                            )
                            .selectable(true),
                        );
                    });
            });
    }

    // =========================================================================
    // FULL-HEIGHT TWO-PANE SETTINGS VIEW (Matching TruePeaks Architecture)
    // =========================================================================
    fn render_settings_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, pal: &Palette) {
        let mut config_changed = false;
        let full_h = ui.available_height();

        ui.spacing_mut().item_spacing.x = 0.0;
        ui.horizontal_top(|ui| {
            // Left Sidebar Navigation (pinned full height)
            let nav_width = 236.0;
            let (nav_rect, _) =
                ui.allocate_exact_size(Vec2::new(nav_width, full_h), Sense::hover());
            ui.painter()
                .rect_filled(nav_rect, Rounding::ZERO, pal.surface);
            ui.painter().line_segment(
                [nav_rect.right_top(), nav_rect.right_bottom()],
                Stroke::new(1.0_f32, pal.border),
            );

            let mut nav_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(nav_rect.shrink2(Vec2::new(12.0, 14.0)))
                    .layout(Layout::top_down(Align::Min)),
            );
            nav_ui.set_clip_rect(nav_rect.shrink2(Vec2::new(4.0, 0.0)));
            nav_ui.spacing_mut().item_spacing = Vec2::new(0.0, 4.0);

            nav_ui.label(
                RichText::new("SETTINGS")
                    .size(11.0)
                    .strong()
                    .color(pal.text_muted),
            );
            nav_ui.add_space(6.0);

            for &sec in SettingsSection::all() {
                let selected = self.settings_section == sec;
                let (btn_rect, resp) = nav_ui.allocate_exact_size(
                    Vec2::new(nav_rect.width() - 24.0, 48.0),
                    Sense::click(),
                );
                let fill = if selected {
                    Color32::from_rgba_unmultiplied(
                        pal.accent.r(),
                        pal.accent.g(),
                        pal.accent.b(),
                        if pal.dark { 42 } else { 26 },
                    )
                } else if resp.hovered() {
                    pal.surface_alt
                } else {
                    Color32::TRANSPARENT
                };
                nav_ui
                    .painter()
                    .rect_filled(btn_rect, Rounding::same(8.0), fill);
                if selected {
                    let bar = Rect::from_min_max(
                        Pos2::new(btn_rect.left() + 2.0, btn_rect.top() + 8.0),
                        Pos2::new(btn_rect.left() + 5.0, btn_rect.bottom() - 8.0),
                    );
                    nav_ui
                        .painter()
                        .rect_filled(bar, Rounding::same(2.0), pal.accent);
                }
                let title_color = if selected {
                    if pal.dark {
                        pal.accent_hover
                    } else {
                        pal.accent
                    }
                } else {
                    pal.text_primary
                };
                nav_ui.painter().text(
                    Pos2::new(btn_rect.left() + 14.0, btn_rect.top() + 15.0),
                    Align2::LEFT_CENTER,
                    sec.label(),
                    FontId::proportional(13.5),
                    title_color,
                );
                nav_ui.painter().text(
                    Pos2::new(btn_rect.left() + 14.0, btn_rect.top() + 33.0),
                    Align2::LEFT_CENTER,
                    sec.description(),
                    FontId::proportional(11.0),
                    pal.text_muted,
                );
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    self.settings_section = sec;
                }
            }

            // Bottom sidebar actions (Restore Defaults)
            let bottom_size = nav_ui.available_size();
            nav_ui.allocate_ui_with_layout(bottom_size, Layout::bottom_up(Align::Min), |ui| {
                if ui
                    .add_sized(
                        Vec2::new(nav_rect.width() - 24.0, 30.0),
                        egui::Button::new(RichText::new("Restore Defaults").size(12.5)),
                    )
                    .on_hover_text("Reset all alignment, deliverable, and theme settings back to defaults")
                    .clicked()
                {
                    self.config = AppConfig::default();
                    ctx.set_theme(to_egui_theme_preference(self.config.theme));
                    self.reset_window_geometry(ctx);
                    config_changed = true;
                }
                ui.add_space(4.0);
                if let Some(cfg_path) = AppConfig::config_file_path() {
                    ui.label(
                        RichText::new("Changes save automatically")
                            .size(11.0)
                            .color(pal.text_muted),
                    )
                    .on_hover_text(format!("Config path:\n{}", cfg_path.display()));
                }
            });

            // Right Scrollable Settings Content Pane
            let content_rect = ui.available_rect_before_wrap();
            let mut content_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(content_rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            content_ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);

            egui::ScrollArea::vertical()
                .id_salt("admaster_settings_content_scroll")
                .auto_shrink([false, false])
                .show(&mut content_ui, |ui| {
                    Frame::none()
                        .inner_margin(Margin::symmetric(28.0, 22.0))
                        .show(ui, |ui| {
                            let max_w = ui.available_width().min(780.0);
                            ui.set_width(max_w);

                            match self.settings_section {
                                SettingsSection::Alignment => {
                                    config_changed |= self.render_settings_alignment(ui, pal);
                                }
                                SettingsSection::Deliverables => {
                                    config_changed |= self.render_settings_deliverables(ui, pal);
                                }
                                SettingsSection::ToolPaths => {
                                    config_changed |= self.render_settings_tool_paths(ui, pal);
                                }
                                SettingsSection::Appearance => {
                                    config_changed |= self.render_settings_appearance(ui, ctx, pal);
                                }
                            }
                        });
                });
        });

        if config_changed {
            let _ = self.config.save();
        }
    }

    fn render_settings_alignment(&mut self, ui: &mut egui::Ui, pal: &Palette) -> bool {
        let mut changed = false;
        settings_section_header(
            ui,
            pal,
            "Alignment & Non-Linear Warping",
            "Configure 3-band FFT cross-correlation, PAL landmark probing, Viterbi DP lattice, and PCHIP cubic splines.",
        );

        settings_card(
            ui,
            pal,
            "Synchronization Strategy & PAL Probing",
            "Select how ADMaster handles global offsets, alternate cuts, and 25fps PAL transfers.",
            |ui| {
                setting_row(
                    ui,
                    pal,
                    "Alignment Mode",
                    "Auto uses 100-anchor correlation and falls back to Viterbi+PCHIP warp on cut differences",
                    "Auto is recommended for all movies. Force Warp bypasses global correlation and runs non-linear warping directly.",
                    |ui| {
                        egui::ComboBox::from_id_salt("set_sync_mode_combo")
                            .width(240.0)
                            .selected_text(self.config.sync_mode.short_label())
                            .show_ui(ui, |ui| {
                                for &m in SyncMode::all() {
                                    changed |= ui
                                        .selectable_value(&mut self.config.sync_mode, m, m.label())
                                        .changed();
                                }
                            });
                    },
                );

                ui.separator();

                setting_row(
                    ui,
                    pal,
                    "PAL 25fps Speed Handling",
                    "Automatically probes 25.0 vs 23.976 fps landmark constellations in ~2 seconds",
                    "Applies exact dynamic sample-rate slowdown (input_sr * 23.976024 / 25.0) when PAL speedup is detected.",
                    |ui| {
                        egui::ComboBox::from_id_salt("set_pal_mode_combo")
                            .width(240.0)
                            .selected_text(self.config.pal_mode.short_label())
                            .show_ui(ui, |ui| {
                                for &pm in PalMode::all() {
                                    changed |= ui
                                        .selectable_value(&mut self.config.pal_mode, pm, pm.label())
                                        .changed();
                                }
                            });
                    },
                );

                ui.separator();

                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.multiband,
                    "3-Band Narration Speech Suppression",
                    "Downweights 250 Hz - 1200 Hz speech band to 0.2 so correlation locks onto effects & score",
                    "Prevents loud narrator speech from causing false correlation peaks against movie dialogue.",
                    pal,
                );

                ui.separator();

                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.speed_detect,
                    "Automatic Acoustic Landmark PAL Probing",
                    "Matches Shazam-style spectral peak pairs at native 1.0x and PAL 1.0427x before correlation",
                    "Detects unlabelled PAL DVD/TV Audio Description captures automatically without requiring [UK] in the filename.",
                    pal,
                );
            },
        );

        ui.add_space(12.0);

        settings_card(
            ui,
            pal,
            "Correlation Anchors, Spline Warping & Gap Filling",
            "Fine-tune anchor density, Viterbi discontinuity splitting, and Master Audio gap splicing.",
            |ui| {
                setting_row(
                    ui,
                    pal,
                    "Correlation Analysis Anchors",
                    "Number of 30-second windows sampled across 5%..88% of the timeline",
                    "Default: 100 anchors. Higher counts increase consensus precision on long features.",
                    |ui| {
                        changed |= ui
                            .add(egui::Slider::new(&mut self.config.anchors, 15..=250))
                            .changed();
                    },
                );

                ui.separator();

                setting_row(
                    ui,
                    pal,
                    "Minimum Consensus Confidence",
                    "Peak correlation threshold before engaging Viterbi+PCHIP non-linear warp fallback",
                    "Default: 0.15 (15%).",
                    |ui| {
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut self.config.min_confidence, 0.05..=0.50)
                                    .step_by(0.01),
                            )
                            .changed();
                    },
                );

                ui.separator();

                setting_row(
                    ui,
                    pal,
                    "Warp Discontinuity Threshold",
                    "Offset jump (in seconds) that splits the PCHIP spline into separate structural blocks",
                    "Default: 0.80 s. Cut boundaries are automatically nudged into narrator silence pauses.",
                    |ui| {
                        changed |= ui
                            .add(
                                egui::Slider::new(
                                    &mut self.config.discontinuity_threshold,
                                    0.25..=3.0,
                                )
                                .suffix(" s")
                                .step_by(0.05),
                            )
                            .changed();
                    },
                );

                ui.separator();

                setting_row(
                    ui,
                    pal,
                    "Equal-Power Crossfade Duration",
                    "Cosine crossfade length at structural segment junctions and master gap boundaries",
                    "Default: 50 ms.",
                    |ui| {
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut self.config.crossfade_ms, 10..=200)
                                    .suffix(" ms"),
                            )
                            .changed();
                    },
                );

                ui.separator();

                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.gap_fill,
                    "Windowed Reference Master Audio Gap Filling",
                    "Splices level-matched Blu-ray soundtrack audio into excised scene voids with <15 MB RAM",
                    "Eliminates dead silence drops when syncing a Theatrical Cut AD track onto an Extended Cut video master.",
                    pal,
                );
            },
        );

        changed
    }

    fn render_settings_deliverables(&mut self, ui: &mut egui::Ui, pal: &Palette) -> bool {
        let mut changed = false;
        settings_section_header(
            ui,
            pal,
            "Encoding, Downmixing & MKV Multiplexing",
            "Configure ITU-R BS.775 True 2.0 stereo downmixing, EBU R128 Night Mode normalization, and MKV output.",
        );

        settings_card(
            ui,
            pal,
            "Audio Deliverables & Downmixing",
            "Choose which output variants are rendered into the movie's output/ directory.",
            |ui| {
                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.convert_main,
                    "Convert & Downmix Main Reference Audio",
                    "Generates 5.1 Surround AC3 (640k), True 2.0 Stereo (alimiter), and Night Mode (loudnorm)",
                    "Applies strict ITU-R BS.775 downmix matrix (L + 0.7071*C + 0.7071*Ls) and -ac 2.",
                    pal,
                );

                ui.separator();

                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.stereo_only,
                    "Stereo Only Mode (Skip 5.1 Surround AC3)",
                    "When converting main audio, only output 2.0 Stereo deliverables",
                    "Useful for lightweight stereo-only playback devices.",
                    pal,
                );

                ui.separator();

                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.generate_night_mode,
                    "Generate EBU R128 Night Mode Deliverables",
                    "Renders a loudness-normalized (-16 LUFS) variant alongside the full-dynamic-range Stereo track",
                    "Disable (--no-night) for faster test runs when you only need the standard Stereo track.",
                    pal,
                );

                ui.separator();

                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.wav_output,
                    "Output Uncompressed 16-bit 48kHz PCM WAV (.wav)",
                    "Bypasses AAC compression and writes 48kHz stereo WAV deliverables directly",
                    "Ideal for rapid sync verification or external mastering.",
                    pal,
                );

                ui.separator();

                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.mux_final_mkv,
                    "Multiplex Final MKV Container via MKVMerge",
                    "Combines reference video, Main Stereo, Synced AD Stereo (flagged visual-impaired), and English subtitles",
                    "Requires mkvmerge (MKVToolNix) to be installed.",
                    pal,
                );
            },
        );

        ui.add_space(12.0);

        settings_card(
            ui,
            pal,
            "Loudness Targets & Stream Selection",
            "Broadcast EBU R128 loudness parameters and reference audio track mapping.",
            |ui| {
                setting_row(
                    ui,
                    pal,
                    "Reference Audio Stream Selector",
                    "FFmpeg stream specifier used when extracting reference audio from multi-track MKVs",
                    "Default: 0:a:0 (first audio track). Use 0:a:1 for the second audio track.",
                    |ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.config.ref_stream)
                                    .desired_width(120.0),
                            )
                            .changed();
                    },
                );

                ui.separator();

                setting_row(
                    ui,
                    pal,
                    "Night Mode Integrated Loudness (LUFS)",
                    "Target EBU R128 integrated loudness for Night Mode tracks",
                    "Default: -16.0 LUFS.",
                    |ui| {
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut self.config.target_i, -24.0..=-12.0)
                                    .suffix(" LUFS")
                                    .step_by(0.5),
                            )
                            .changed();
                    },
                );

                ui.separator();

                setting_row(
                    ui,
                    pal,
                    "Night Mode True Peak Ceiling (dBTP)",
                    "Maximum true peak level permitted by loudnorm",
                    "Default: -1.5 dBTP.",
                    |ui| {
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut self.config.target_tp, -3.0..=-0.2)
                                    .suffix(" dBTP")
                                    .step_by(0.1),
                            )
                            .changed();
                    },
                );
            },
        );

        changed
    }

    fn render_settings_tool_paths(&mut self, ui: &mut egui::Ui, pal: &Palette) -> bool {
        let mut changed = false;
        let discovered = ToolPaths::discover(&self.config);

        settings_section_header(
            ui,
            pal,
            "Binaries & External Tool Paths",
            "Leave fields blank to auto-discover in ./bin/, standard OS install locations (Program Files / Homebrew / Applications), or system PATH.",
        );

        settings_card(
            ui,
            pal,
            "FFmpeg & FFprobe",
            "Required for media probing, stream decoding, loudnorm filtering, and AAC/AC3 encoding.",
            |ui| {
                changed |= path_picker_row(
                    ui,
                    pal,
                    "FFmpeg Executable",
                    &format!("Active: {}", discovered.ffmpeg.display()),
                    &mut self.config.ffmpeg_path,
                );
                ui.separator();
                changed |= path_picker_row(
                    ui,
                    pal,
                    "FFprobe Executable",
                    &format!("Active: {}", discovered.ffprobe.display()),
                    &mut self.config.ffprobe_path,
                );
            },
        );

        ui.add_space(12.0);

        settings_card(
            ui,
            pal,
            "MKVToolNix Suite",
            "Used for final MKV multiplexing (--mux), commentary track flagging, and chapter management.",
            |ui| {
                let mkvmerge_status = discovered
                    .mkvmerge
                    .as_ref()
                    .map(|p| format!("Active: {}", p.display()))
                    .unwrap_or_else(|| "Not found (install MKVToolNix or browse)".to_string());
                changed |= path_picker_row(
                    ui,
                    pal,
                    "mkvmerge Executable",
                    &mkvmerge_status,
                    &mut self.config.mkvmerge_path,
                );
                ui.separator();
                let propedit_status = discovered
                    .mkvpropedit
                    .as_ref()
                    .map(|p| format!("Active: {}", p.display()))
                    .unwrap_or_else(|| "Not found".to_string());
                changed |= path_picker_row(
                    ui,
                    pal,
                    "mkvpropedit Executable",
                    &propedit_status,
                    &mut self.config.mkvpropedit_path,
                );
                ui.separator();
                let extract_status = discovered
                    .mkvextract
                    .as_ref()
                    .map(|p| format!("Active: {}", p.display()))
                    .unwrap_or_else(|| "Not found".to_string());
                changed |= path_picker_row(
                    ui,
                    pal,
                    "mkvextract Executable",
                    &extract_status,
                    &mut self.config.mkvextract_path,
                );
            },
        );

        changed
    }

    fn render_settings_appearance(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        pal: &Palette,
    ) -> bool {
        let mut changed = false;
        settings_section_header(
            ui,
            pal,
            "Appearance, Concurrency & Scratch Cache",
            "Customize light/dark theme, FFmpeg thread throttling, scratch caching, and session log files.",
        );

        settings_card(
            ui,
            pal,
            "Theme & Window Geometry",
            "Visual presentation and window state persistence.",
            |ui| {
                setting_row(
                    ui,
                    pal,
                    "Color Theme",
                    "System Default automatically follows your OS Dark/Light mode setting",
                    "Switch between System Default, Dark, and Light.",
                    |ui| {
                        ui.horizontal(|ui| {
                            for &mode in ThemeMode::all() {
                                let selected = self.config.theme == mode;
                                if ui
                                    .add(egui::Button::new(mode.label()).selected(selected))
                                    .clicked()
                                {
                                    self.config.theme = mode;
                                    ctx.set_theme(to_egui_theme_preference(mode));
                                    changed = true;
                                }
                            }
                        });
                    },
                );

                ui.separator();

                setting_row(
                    ui,
                    pal,
                    "Window Position & Size",
                    "Automatically saved across restarts; reset restores centered 1080x760 default",
                    "Restore default centered window geometry.",
                    |ui| {
                        if ui.button("Reset Window Geometry").clicked() {
                            self.reset_window_geometry(ctx);
                            changed = true;
                        }
                    },
                );
            },
        );

        ui.add_space(12.0);
        let max_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);

        settings_card(
            ui,
            pal,
            "Concurrency, Scratch Caching & Logs",
            "Control CPU thread allocation and temporary analysis caching.",
            |ui| {
                setting_row(
                    ui,
                    pal,
                    "FFmpeg Encoding Threads",
                    &format!("Default: 2 threads (up to {} logical cores available)", max_cores),
                    "Limits FFmpeg decoder/encoder threads so your PC stays responsive during heavy batch runs.",
                    |ui| {
                        changed |= ui
                            .add(egui::Slider::new(&mut self.config.threads, 1..=max_cores))
                            .changed();
                    },
                );

                ui.separator();

                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.overwrite,
                    "Overwrite Existing Deliverables in output/",
                    "When disabled, existing files in output/ are preserved and skipped",
                    "Enable (--overwrite) to force re-rendering deliverables.",
                    pal,
                );

                ui.separator();

                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.clean_scratch,
                    "Wipe scratch/ Directory Before Each Run (--clean-scratch)",
                    "Clears cached sync_params.json and analysis WAVs for a 100% fresh recalculation",
                    "When disabled, cached sync parameters in scratch/ are reused across reruns.",
                    pal,
                );

                ui.separator();

                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.keep_scratch,
                    "Keep Temporary Analysis WAVs in scratch/ After Completion",
                    "Retains 8kHz mono and intermediate 48kHz stereo WAVs for inspection",
                    "Useful when debugging alignment or running multiple test passes.",
                    pal,
                );

                ui.separator();

                changed |= checkbox_row_tip(
                    ui,
                    &mut self.config.log_to_file,
                    "Write Timestamped Session Log Files to Disk",
                    "Saves full console sync summary to YYYYMMDD_HHMMSS_Title.log",
                    "Logs are written to Custom Log Directory (or the movie project folder if blank).",
                    pal,
                );
            },
        );

        changed
    }

    fn render_about_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, pal: &Palette) {
        let repo_url = env!("CARGO_PKG_REPOSITORY");
        let version = env!("CARGO_PKG_VERSION");
        let license = env!("CARGO_PKG_LICENSE");
        let brand_magenta = if pal.dark {
            Color32::from_rgb(232, 70, 245)
        } else {
            Color32::from_rgb(192, 38, 211)
        };
        let brand_purple = if pal.dark {
            Color32::from_rgb(124, 58, 237)
        } else {
            Color32::from_rgb(109, 40, 217)
        };
        let accent_text = if pal.dark { pal.accent_hover } else { pal.accent };

        let url_copied = self
            .about_url_copied_at
            .map(|t| t.elapsed().as_secs_f32() < 2.0)
            .unwrap_or(false);
        if url_copied {
            ctx.request_repaint();
        }

        let max_card_w = match self.about_tab {
            AboutTab::Overview => 640.0,
            AboutTab::CreditsAndLicenses => 720.0,
        };

        egui::ScrollArea::vertical()
            .id_salt("admaster_about_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let avail_w = ui.available_width();
                let avail_h = ui.available_height();
                let card_w = (avail_w - 40.0).clamp(360.0, max_card_w);
                let top_pad = match self.about_tab {
                    AboutTab::Overview => ((avail_h - 540.0) * 0.35).max(16.0),
                    AboutTab::CreditsAndLicenses => 16.0,
                };

                ui.vertical_centered(|ui| {
                    ui.add_space(top_pad);
                    Frame::none()
                        .fill(pal.surface)
                        .rounding(Rounding::same(12.0))
                        .stroke(Stroke::new(1.0_f32, pal.border))
                        .inner_margin(Margin::symmetric(28.0, 24.0))
                        .show(ui, |ui| {
                            ui.set_width(card_w);
                            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                // Hero Header: SVG App Icon + Title + Version Pill + Subtitle
                                ui.horizontal(|ui| {
                                    let (badge_rect, _) =
                                        ui.allocate_exact_size(Vec2::splat(52.0), Sense::hover());
                                    self.icon_cache.paint_app_icon(ctx, ui.painter(), badge_rect);
                                    ui.add_space(10.0);
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new("ADMaster")
                                                    .size(24.0)
                                                    .strong()
                                                    .color(pal.text_primary),
                                            );
                                            let pill_fill = Color32::from_rgba_unmultiplied(
                                                brand_magenta.r(),
                                                brand_magenta.g(),
                                                brand_magenta.b(),
                                                if pal.dark { 36 } else { 22 },
                                            );
                                            Frame::none()
                                                .fill(pill_fill)
                                                .stroke(Stroke::new(1.0_f32, brand_purple))
                                                .rounding(Rounding::same(10.0))
                                                .inner_margin(Margin::symmetric(8.0, 2.0))
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        RichText::new(format!("v{}", version))
                                                            .size(12.5)
                                                            .strong()
                                                            .color(brand_magenta),
                                                    );
                                                });
                                        });
                                        ui.label(
                                            RichText::new(
                                                "Audio Description Synchronization & Mastering Studio",
                                            )
                                            .size(13.5)
                                            .color(pal.text_muted),
                                        );
                                    });
                                });

                                ui.add_space(12.0);
                                // Tab switcher: Overview vs Credits & MIT Licenses
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(RichText::new("Overview").size(13.0))
                                                .selected(self.about_tab == AboutTab::Overview)
                                                .min_size(Vec2::new(0.0, 28.0)),
                                        )
                                        .clicked()
                                    {
                                        self.about_tab = AboutTab::Overview;
                                    }
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("Credits & MIT Licenses").size(13.0),
                                            )
                                            .selected(
                                                self.about_tab == AboutTab::CreditsAndLicenses,
                                            )
                                            .min_size(Vec2::new(0.0, 28.0)),
                                        )
                                        .on_hover_text(
                                            "Full third-party open-source credits, GitHub repositories, and MIT license notices",
                                        )
                                        .clicked()
                                    {
                                        self.about_tab = AboutTab::CreditsAndLicenses;
                                    }
                                });

                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(10.0);

                                match self.about_tab {
                                    AboutTab::Overview => {
                                        self.render_about_overview_tab(
                                            ctx,
                                            ui,
                                            pal,
                                            accent_text,
                                            repo_url,
                                            version,
                                            license,
                                            url_copied,
                                        );
                                    }
                                    AboutTab::CreditsAndLicenses => {
                                        self.render_about_credits_tab(ctx, ui, pal, accent_text);
                                    }
                                }

                                ui.add_space(16.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("Open GitHub").size(13.0),
                                            )
                                            .min_size(Vec2::new(0.0, 30.0)),
                                        )
                                        .on_hover_text(repo_url)
                                        .clicked()
                                    {
                                        ctx.open_url(egui::OpenUrl::new_tab(repo_url));
                                    }

                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("Settings...").size(13.0),
                                            )
                                            .min_size(Vec2::new(0.0, 30.0)),
                                        )
                                        .clicked()
                                    {
                                        self.active_view = ActiveView::Settings;
                                    }

                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui
                                            .add(
                                                primary_button("Back to Workspace", true, pal)
                                                    .min_size(Vec2::new(150.0, 30.0)),
                                            )
                                            .clicked()
                                        {
                                            self.active_view = ActiveView::Workspace;
                                        }
                                    });
                                });
                            });
                        });
                    ui.add_space(20.0);
                });
            });
    }

    #[allow(clippy::too_many_arguments)]
    fn render_about_overview_tab(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        pal: &Palette,
        accent_text: Color32,
        repo_url: &str,
        version: &str,
        license: &str,
        url_copied: bool,
    ) {
        ui.label(
            RichText::new(
                "Automatically aligns Audio Description tracks to reference Blu-ray and UHD MKV masters, correcting speed differences, alternate cuts, and missing scenes before rendering broadcast-ready stereo deliverables.",
            )
            .size(13.0)
            .color(pal.text_primary),
        );

        ui.add_space(12.0);

        // Key-Value Specification Well
        Frame::none()
            .fill(pal.surface_alt)
            .rounding(Rounding::same(8.0))
            .stroke(Stroke::new(1.0_f32, pal.border))
            .inner_margin(Margin::symmetric(14.0, 12.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                about_kv_row(ui, pal, "GitHub", |ui| {
                    ui.horizontal(|ui| {
                        ui.hyperlink_to(
                            RichText::new(repo_url).size(13.0).color(accent_text),
                            repo_url,
                        )
                        .on_hover_text("Open ADMaster on GitHub in your browser");

                        let copy_label = if url_copied { "Copied" } else { "Copy" };
                        let copy_btn = if url_copied {
                            egui::Button::new(
                                RichText::new(copy_label)
                                    .size(12.0)
                                    .strong()
                                    .color(pal.success),
                            )
                            .min_size(Vec2::new(0.0, 22.0))
                        } else {
                            egui::Button::new(RichText::new(copy_label).size(12.0))
                                .min_size(Vec2::new(0.0, 22.0))
                        };
                        if ui
                            .add(copy_btn)
                            .on_hover_text("Copy repository URL to clipboard")
                            .clicked()
                        {
                            ctx.copy_text(repo_url.to_string());
                            self.about_url_copied_at = Some(Instant::now());
                        }
                    });
                });

                ui.add_space(6.0);
                let platform = format!(
                    "v{}  --  {} ({})",
                    version,
                    std::env::consts::OS,
                    std::env::consts::ARCH
                );
                about_kv_row(ui, pal, "Version", |ui| {
                    ui.label(RichText::new(platform).size(13.0).color(pal.text_primary));
                });

                ui.add_space(6.0);
                about_kv_row(ui, pal, "License", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} License (Copyright (c) 2026 torrq)",
                                license
                            ))
                            .size(13.0)
                            .color(pal.text_primary),
                        );
                        if ui
                            .small_button("View Notices")
                            .on_hover_text("View full MIT License and third-party copyright notices")
                            .clicked()
                        {
                            self.about_tab = AboutTab::CreditsAndLicenses;
                        }
                    });
                });

                ui.add_space(6.0);
                about_kv_row(ui, pal, "Alignment", |ui| {
                    ui.label(
                        RichText::new("100-anchor FFT correlation + Viterbi & PCHIP spline warp")
                            .size(13.0)
                            .color(pal.text_primary),
                    );
                });

                ui.add_space(6.0);
                about_kv_row(ui, pal, "Mastering", |ui| {
                    ui.label(
                        RichText::new(
                            "ITU-R BS.775 True 2.0 Stereo & EBU R128 Night Mode (-16 LUFS)",
                        )
                        .size(13.0)
                        .color(pal.text_primary),
                    );
                });

                ui.add_space(6.0);
                about_kv_row(ui, pal, "Powered By", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let links = [
                            ("ADSync (Anilo)", "https://github.com/JohnnyTheCoder1/ADSync"),
                            ("FFmpeg", "https://github.com/FFmpeg/FFmpeg"),
                            ("MKVToolNix", "https://github.com/mbunkus/mkvtoolnix"),
                            ("egui", "https://github.com/emilk/egui"),
                            ("resvg", "https://github.com/linebender/resvg"),
                            ("RustFFT", "https://github.com/ejmahler/RustFFT"),
                            ("Rayon", "https://github.com/rayon-rs/rayon"),
                        ];
                        for (idx, (lbl, url)) in links.iter().enumerate() {
                            if idx > 0 {
                                ui.label(RichText::new("*").size(11.5).color(pal.text_muted));
                            }
                            ui.hyperlink_to(
                                RichText::new(*lbl).size(12.5).color(accent_text),
                                *url,
                            )
                            .on_hover_text(*url);
                        }
                    });
                });

                if let Some(cfg_path) = AppConfig::config_file_path() {
                    ui.add_space(6.0);
                    about_kv_row(ui, pal, "Config File", |ui| {
                        ui.horizontal(|ui| {
                            let max_p = (ui.available_width() - 68.0).max(80.0);
                            ui.allocate_ui_with_layout(
                                Vec2::new(max_p, 20.0),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(cfg_path.display().to_string())
                                                .size(12.5)
                                                .color(pal.text_muted),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(cfg_path.display().to_string());
                                },
                            );
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("Reveal").size(12.0))
                                        .min_size(Vec2::new(0.0, 22.0)),
                                )
                                .on_hover_text(format!("Locate {}", cfg_path.display()))
                                .clicked()
                            {
                                if !cfg_path.exists() {
                                    let _ = self.config.save();
                                }
                                reveal_in_file_manager(&cfg_path);
                            }
                        });
                    });
                }
            });
    }

    fn render_about_credits_tab(
        &mut self,
        _ctx: &egui::Context,
        ui: &mut egui::Ui,
        pal: &Palette,
        accent_text: Color32,
    ) {
        ui.label(
            RichText::new(
                "ADMaster is built on open-source audio, video, and Rust projects. Click any repository link below to visit its GitHub page:",
            )
            .size(13.0)
            .color(pal.text_muted),
        );
        ui.add_space(8.0);

        let credits: &[(&str, &str, &str, &str)] = &[
            (
                "ADSync",
                "Anilo (JohnnyTheCoder1) -- MIT License (Copyright (c) 2026 Anilo)",
                "Non-linear Audio Description alignment architecture (3-band speech suppression, STFT landmark constellation PAL prober, second-order Viterbi candidate lattice DP, monotone Fritsch-Carlson PCHIP spline fitting, and silence-nudged cut refinement).",
                "https://github.com/JohnnyTheCoder1/ADSync",
            ),
            (
                "FFmpeg & FFprobe",
                "The FFmpeg Developers -- LGPLv2.1+ / GPLv2+",
                "External media engine used for stream probing, audio decoding, dynamic asetrate PAL clock slowdown, ITU-R BS.775 stereo downmixing, EBU R128 loudnorm normalization, and AAC/AC3 encoding.",
                "https://github.com/FFmpeg/FFmpeg",
            ),
            (
                "MKVToolNix",
                "Moritz Bunkus -- GPLv2",
                "External Matroska toolkit (mkvmerge, mkvpropedit, mkvextract) used for container multiplexing, visual-impaired & commentary track flagging, and chapter management.",
                "https://github.com/mbunkus/mkvtoolnix",
            ),
            (
                "egui & eframe",
                "Emil Ernerfeldt (emilk) -- MIT / Apache-2.0",
                "Immediate-mode pure-Rust GUI framework powering the ADMaster desktop studio.",
                "https://github.com/emilk/egui",
            ),
            (
                "resvg & tiny-skia",
                "Yevhenii Reizner & Linebender -- MIT / Apache-2.0",
                "Pure-Rust SVG parsing and 2D vector rasterization for crisp HiDPI icons.",
                "https://github.com/linebender/resvg",
            ),
            (
                "RustFFT",
                "Elliott Mahler (ejmahler) -- MIT / Apache-2.0",
                "SIMD-accelerated pure-Rust Fast Fourier Transform library for 100-anchor cross-correlation and STFT spectral analysis.",
                "https://github.com/ejmahler/RustFFT",
            ),
            (
                "Rayon",
                "Niko Matsakis & Josh Stone -- MIT / Apache-2.0",
                "Data-parallel work-stealing thread pool accelerating multi-core anchor and lattice scoring.",
                "https://github.com/rayon-rs/rayon",
            ),
            (
                "rfd, Crossbeam, Serde, toml-rs, Chrono, directories-rs & winresource",
                "Rust Open-Source Contributors -- MIT / Apache-2.0",
                "Native OS file dialogs, lock-free worker channels, TOML/JSON serialization, timestamp formatting, platform config paths, and Win32 PE icon resource embedding.",
                "https://github.com/PolyMeilex/rfd",
            ),
        ];

        for (name, author_lic, desc, url) in credits {
            Frame::none()
                .fill(pal.surface_alt)
                .rounding(Rounding::same(8.0))
                .stroke(Stroke::new(1.0_f32, pal.border))
                .inner_margin(Margin::symmetric(12.0, 9.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.hyperlink_to(
                            RichText::new(*name).size(13.5).strong().color(accent_text),
                            *url,
                        )
                        .on_hover_text(*url);
                        ui.label(
                            RichText::new(format!("-- {}", author_lic))
                                .size(12.0)
                                .color(pal.text_muted),
                        );
                    });
                    ui.add_space(2.0);
                    ui.label(RichText::new(*desc).size(12.0).color(pal.text_primary));
                });
            ui.add_space(6.0);
        }

        ui.add_space(4.0);
        ui.collapsing(
            RichText::new("Full MIT License & Third-Party Copyright Notices")
                .size(13.0)
                .strong()
                .color(pal.text_primary),
            |ui| {
                Frame::none()
                    .fill(pal.well)
                    .rounding(Rounding::same(6.0))
                    .stroke(Stroke::new(1.0_f32, pal.border))
                    .inner_margin(Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(
                            egui::Label::new(
                                RichText::new(include_str!("../LICENSE"))
                                    .font(FontId::monospace(11.5))
                                    .color(pal.text_primary),
                            )
                            .selectable(true),
                        );
                    });
            },
        );
    }
}

fn about_kv_row(
    ui: &mut egui::Ui,
    pal: &Palette,
    key: &str,
    add_value: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(112.0, 20.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(
                    RichText::new(key)
                        .size(13.0)
                        .strong()
                        .color(pal.text_muted),
                );
            },
        );
        add_value(ui);
    });
}

fn reveal_in_file_manager(path: &Path) {
    let raw = path.to_string_lossy();
    let clean = raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string();

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", clean))
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-R", &clean])
            .spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let target = Path::new(&clean)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(clean);
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

fn path_picker_row(
    ui: &mut egui::Ui,
    pal: &Palette,
    title: &str,
    status_subtitle: &str,
    path_val: &mut String,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(220.0);
            ui.label(RichText::new(title).strong().color(pal.text_primary));
            ui.label(
                RichText::new(status_subtitle)
                    .size(11.5)
                    .color(pal.text_muted),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !path_val.is_empty() && ui.small_button("Auto").on_hover_text("Clear custom path and use auto-discovery").clicked() {
                path_val.clear();
                changed = true;
            }
            if ui.button("Browse...").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_file() {
                    *path_val = p.display().to_string();
                    changed = true;
                }
            }
            let avail_edit = (ui.available_width() - 8.0).max(120.0);
            changed |= ui
                .add(
                    egui::TextEdit::singleline(path_val)
                        .hint_text("Auto-discover in ./bin/ or PATH")
                        .desired_width(avail_edit),
                )
                .changed();
        });
    });
    changed
}

impl eframe::App for ADMasterApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_job_events();
        self.handle_dropped_files(ctx);
        self.handle_shortcuts(ctx);
        self.track_window_geometry(ctx);

        if self.is_running {
            ctx.request_repaint_after(Duration::from_millis(33));
        }

        let pal = Palette::for_theme(ctx.theme() == egui::Theme::Dark);

        self.render_top_toolbar(ctx, &pal);
        self.render_bottom_status_bar(ctx, &pal);

        let central_margin = if self.active_view == ActiveView::Settings {
            Margin::ZERO
        } else {
            Margin::same(14.0)
        };

        egui::CentralPanel::default()
            .frame(Frame::none().fill(pal.canvas).inner_margin(central_margin))
            .show(ctx, |ui| match self.active_view {
                ActiveView::Workspace => self.render_workspace_view(ui, &pal),
                ActiveView::Settings => self.render_settings_view(ctx, ui, &pal),
                ActiveView::About => self.render_about_view(ctx, ui, &pal),
            });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.window_save_pending_at.take().is_some() {
            let _ = self.config.save();
        }
    }
}

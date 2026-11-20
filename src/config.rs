use chrono::Local;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_WINDOW_WIDTH: f32 = 1080.0;
pub const DEFAULT_WINDOW_HEIGHT: f32 = 760.0;
pub const MIN_WINDOW_WIDTH: f32 = 820.0;
pub const MIN_WINDOW_HEIGHT: f32 = 580.0;
pub const DEFAULT_LOG_FILENAME_PATTERN: &str = "{title} - ADMaster Sync ({date}).log";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeMode {
    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::System => "System Default",
            ThemeMode::Dark => "Dark",
            ThemeMode::Light => "Light",
        }
    }

    pub fn next(self) -> Self {
        match self {
            ThemeMode::System => ThemeMode::Dark,
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::System,
        }
    }

    pub fn all() -> &'static [ThemeMode] {
        &[ThemeMode::System, ThemeMode::Dark, ThemeMode::Light]
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncMode {
    #[default]
    Auto,
    OffsetOnly,
    ForceWarp,
}

impl SyncMode {
    pub fn label(self) -> &'static str {
        match self {
            SyncMode::Auto => "Automatic (Matches Same Cut or Different Cuts)",
            SyncMode::OffsetOnly => "Simple Delay Only (Same Cut, No Time-Warping)",
            SyncMode::ForceWarp => "Force Time-Warp (Different Cuts / Edited Scenes)",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            SyncMode::Auto => "Automatic",
            SyncMode::OffsetOnly => "Simple Delay",
            SyncMode::ForceWarp => "Force Time-Warp",
        }
    }

    pub fn all() -> &'static [SyncMode] {
        &[SyncMode::Auto, SyncMode::OffsetOnly, SyncMode::ForceWarp]
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PalMode {
    #[default]
    AutoDetect,
    ForcePal,
    ForceNative,
}

impl PalMode {
    pub fn label(self) -> &'static str {
        match self {
            PalMode::AutoDetect => "Auto-Detect (Checks for 25fps PAL Speedup)",
            PalMode::ForcePal => "Force PAL Slowdown (25fps -> 23.976fps)",
            PalMode::ForceNative => "Keep Original Speed (No PAL Slowdown)",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            PalMode::AutoDetect => "Auto-Detect",
            PalMode::ForcePal => "Slow Down PAL",
            PalMode::ForceNative => "Keep Original",
        }
    }

    pub fn all() -> &'static [PalMode] {
        &[PalMode::AutoDetect, PalMode::ForcePal, PalMode::ForceNative]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    // UI & Window Geometry
    pub theme: ThemeMode,
    pub window_width: f32,
    pub window_height: f32,
    pub window_x: Option<f32>,
    pub window_y: Option<f32>,

    // External Binary Paths (empty = auto-discover in ./bin/, Program Files, or PATH)
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub mkvmerge_path: String,
    pub mkvpropedit_path: String,
    pub mkvextract_path: String,

    // Alignment & DSP Parameters
    pub sync_mode: SyncMode,
    pub pal_mode: PalMode,
    pub anchors: usize,
    pub min_confidence: f64,
    pub multiband: bool,
    pub speed_detect: bool,
    pub gap_fill: bool,
    pub crossfade_ms: u32,
    pub discontinuity_threshold: f64,

    // Encoding & Deliverables
    pub convert_main: bool,
    pub stereo_only: bool,
    pub generate_night_mode: bool,
    pub mux_final_mkv: bool,
    pub wav_output: bool,
    pub target_i: f64,
    pub target_tp: f64,
    pub threads: usize,
    pub ref_stream: String,
    pub preview_sec: Option<f64>,

    // File & Scratch Handling
    pub overwrite: bool,
    pub clean_scratch: bool,
    pub keep_scratch: bool,
    pub debug_mode: bool,
    pub log_to_file: bool,
    pub log_dir: String,
    pub log_filename_pattern: String,
    pub custom_output_dir: String,
    pub custom_mkv_dir: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            window_width: DEFAULT_WINDOW_WIDTH,
            window_height: DEFAULT_WINDOW_HEIGHT,
            window_x: None,
            window_y: None,

            ffmpeg_path: String::new(),
            ffprobe_path: String::new(),
            mkvmerge_path: String::new(),
            mkvpropedit_path: String::new(),
            mkvextract_path: String::new(),

            sync_mode: SyncMode::Auto,
            pal_mode: PalMode::AutoDetect,
            anchors: 100,
            min_confidence: 0.15,
            multiband: true,
            speed_detect: true,
            gap_fill: true,
            crossfade_ms: 50,
            discontinuity_threshold: 0.80,

            convert_main: false,
            stereo_only: false,
            generate_night_mode: true,
            mux_final_mkv: false,
            wav_output: false,
            target_i: -16.0,
            target_tp: -1.5,
            threads: 2,
            ref_stream: "0:a:0".to_string(),
            preview_sec: None,

            overwrite: false,
            clean_scratch: false,
            keep_scratch: false,
            debug_mode: false,
            log_to_file: true,
            log_dir: String::new(),
            log_filename_pattern: DEFAULT_LOG_FILENAME_PATTERN.to_string(),
            custom_output_dir: String::new(),
            custom_mkv_dir: String::new(),
        }
    }
}

/// Legacy JSON configuration schema for seamless backwards-compatibility with `config.json` / `config.local.json`.
#[derive(Debug, Default, Deserialize)]
struct LegacyJsonConfig {
    ffmpeg_path: Option<String>,
    ffprobe_path: Option<String>,
    mkvmerge_path: Option<String>,
    mkvpropedit_path: Option<String>,
    mkvextract_path: Option<String>,
    default_threads: Option<usize>,
    default_target_i: Option<f64>,
    default_target_tp: Option<f64>,
    default_anchors: Option<usize>,
    default_multiband: Option<bool>,
    default_speed_detect: Option<bool>,
    default_overwrite: Option<bool>,
    debug_mode: Option<bool>,
    log_to_file: Option<bool>,
    log_dir: Option<String>,
}

pub fn strip_json_comments(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
        } else if c == '"' {
            in_string = true;
            out.push(c);
            i += 1;
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

impl AppConfig {
    pub fn config_file_path() -> Option<PathBuf> {
        // Check local portable config.toml next to executable or in workspace first
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let local_toml = dir.join("admaster.toml");
                if local_toml.exists() {
                    return Some(local_toml);
                }
            }
        }
        ProjectDirs::from("com", "torrq", "ADMaster")
            .map(|proj| proj.config_dir().join("config.toml"))
    }

    pub fn load() -> Self {
        let mut cfg = Self::default();

        // 1. Apply legacy config.local.json / config.json from workspace or exe dir if present
        let mut search_dirs = Vec::new();
        if let Ok(cwd) = std::env::current_dir() {
            search_dirs.push(cwd);
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(p) = exe.parent() {
                search_dirs.push(p.to_path_buf());
                if let Some(gp) = p.parent().and_then(|x| x.parent()) {
                    search_dirs.push(gp.to_path_buf());
                }
            }
        }
        for dir in &search_dirs {
            for name in ["config.local.json", "config.json", "config.default.json"] {
                let candidate = dir.join(name);
                if candidate.exists() {
                    if let Ok(raw) = fs::read_to_string(&candidate) {
                        let clean = strip_json_comments(&raw);
                        if let Ok(legacy) = serde_json::from_str::<LegacyJsonConfig>(&clean) {
                            cfg.apply_legacy(legacy);
                            break;
                        }
                    }
                }
            }
        }

        // 2. Overlay saved TOML config if present
        if let Some(path) = Self::config_file_path() {
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(loaded) = toml::from_str::<AppConfig>(&raw) {
                    cfg = loaded;
                }
            }
        }

        cfg.sanitize();
        cfg
    }

    pub fn load_from_path(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config '{}': {}", path.display(), e))?;
        if path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("json"))
            .unwrap_or(false)
        {
            let clean = strip_json_comments(&raw);
            let legacy: LegacyJsonConfig = serde_json::from_str(&clean)
                .map_err(|e| format!("Failed to parse JSON config '{}': {}", path.display(), e))?;
            let mut cfg = Self::default();
            cfg.apply_legacy(legacy);
            cfg.sanitize();
            Ok(cfg)
        } else {
            let mut cfg: AppConfig = toml::from_str(&raw)
                .map_err(|e| format!("Failed to parse TOML config '{}': {}", path.display(), e))?;
            cfg.sanitize();
            Ok(cfg)
        }
    }

    fn apply_legacy(&mut self, leg: LegacyJsonConfig) {
        if let Some(v) = leg.ffmpeg_path {
            if !v.is_empty() {
                self.ffmpeg_path = v;
            }
        }
        if let Some(v) = leg.ffprobe_path {
            if !v.is_empty() {
                self.ffprobe_path = v;
            }
        }
        if let Some(v) = leg.mkvmerge_path {
            if !v.is_empty() {
                self.mkvmerge_path = v;
            }
        }
        if let Some(v) = leg.mkvpropedit_path {
            if !v.is_empty() {
                self.mkvpropedit_path = v;
            }
        }
        if let Some(v) = leg.mkvextract_path {
            if !v.is_empty() {
                self.mkvextract_path = v;
            }
        }
        if let Some(v) = leg.default_threads {
            self.threads = v.max(1);
        }
        if let Some(v) = leg.default_target_i {
            self.target_i = v;
        }
        if let Some(v) = leg.default_target_tp {
            self.target_tp = v;
        }
        if let Some(v) = leg.default_anchors {
            self.anchors = v.max(5);
        }
        if let Some(v) = leg.default_multiband {
            self.multiband = v;
        }
        if let Some(v) = leg.default_speed_detect {
            self.speed_detect = v;
        }
        if let Some(v) = leg.default_overwrite {
            self.overwrite = v;
        }
        if let Some(v) = leg.debug_mode {
            self.debug_mode = v;
        }
        if let Some(v) = leg.log_to_file {
            self.log_to_file = v;
        }
        // Note: intentionally ignore legacy `log_dir` so fresh setups default to saving logs
        // in the same directory as the MKV file unless explicitly configured in Settings.
        let _ = leg.log_dir;
    }

    /// Expands syntax tokens in `log_filename_pattern`:
    /// `{title}`, `{mkv}`, `{date}`, `{time}`, `{datetime}`, `{year}`, `{month}`, `{day}`,
    /// `{hour}`, `{minute}`, `{second}`, `{tracks}`, `{method}`.
    pub fn format_log_filename(
        &self,
        title: &str,
        mkv_stem: &str,
        track_count: usize,
        method_tag: &str,
    ) -> String {
        let now = Local::now();
        let raw_pat = if self.log_filename_pattern.trim().is_empty() {
            DEFAULT_LOG_FILENAME_PATTERN
        } else {
            self.log_filename_pattern.trim()
        };

        let sanitize_part = |s: &str| -> String {
            s.chars()
                .map(|c| match c {
                    '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                    c if c.is_control() => '_',
                    other => other,
                })
                .collect::<String>()
                .trim()
                .to_string()
        };

        let clean_title = sanitize_part(title);
        let clean_mkv = sanitize_part(mkv_stem);
        let mut out = raw_pat
            .replace("{title}", &clean_title)
            .replace("{mkv}", &clean_mkv)
            .replace("{date}", &now.format("%Y-%m-%d").to_string())
            .replace("{time}", &now.format("%H-%M-%S").to_string())
            .replace("{datetime}", &now.format("%Y%m%d_%H%M%S").to_string())
            .replace("{year}", &now.format("%Y").to_string())
            .replace("{month}", &now.format("%m").to_string())
            .replace("{day}", &now.format("%d").to_string())
            .replace("{hour}", &now.format("%H").to_string())
            .replace("{minute}", &now.format("%M").to_string())
            .replace("{second}", &now.format("%S").to_string())
            .replace("{tracks}", &track_count.to_string())
            .replace("{method}", &sanitize_part(method_tag));

        out = sanitize_part(&out);
        if out.is_empty() {
            out = format!("{}_ADMaster.log", now.format("%Y%m%d_%H%M%S"));
        }
        let lower = out.to_ascii_lowercase();
        if !(lower.ends_with(".log") || lower.ends_with(".txt") || lower.ends_with(".md")) {
            out.push_str(".log");
        }
        out
    }

    /// Resolves the output directory for session logs.
    /// Defaults to the directory containing the movie/MKV file when `log_dir` is blank.
    /// Supports optional tokens in custom `log_dir`: `{mkv_dir}`, `{output_dir}`, `{date}`, `{year}`, `{month}`, `{day}`, `{title}`.
    pub fn resolve_log_dir(
        &self,
        ref_path: &Path,
        project_dir: &Path,
        out_dir: &Path,
        title: &str,
    ) -> PathBuf {
        let mkv_dir = ref_path.parent().unwrap_or(project_dir);
        let raw = self.log_dir.trim();
        if raw.is_empty() {
            return mkv_dir.to_path_buf();
        }
        let now = Local::now();
        let clean_title: String = title
            .chars()
            .map(|c| match c {
                '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                c if c.is_control() => '_',
                other => other,
            })
            .collect::<String>()
            .trim()
            .to_string();
        let expanded = raw
            .replace("{mkv_dir}", &mkv_dir.to_string_lossy())
            .replace("{output_dir}", &out_dir.to_string_lossy())
            .replace("{date}", &now.format("%Y-%m-%d").to_string())
            .replace("{year}", &now.format("%Y").to_string())
            .replace("{month}", &now.format("%m").to_string())
            .replace("{day}", &now.format("%d").to_string())
            .replace("{title}", &clean_title);
        PathBuf::from(expanded)
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::config_file_path() else {
            return Err("Could not resolve configuration directory".to_string());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        fs::write(&path, toml_str)
            .map_err(|e| format!("Failed to write config '{}': {}", path.display(), e))
    }

    pub fn sanitize(&mut self) {
        self.anchors = self.anchors.clamp(5, 500);
        self.min_confidence = self.min_confidence.clamp(0.01, 0.95);
        self.threads = self.threads.clamp(1, 64);
        self.crossfade_ms = self.crossfade_ms.clamp(5, 500);
        self.discontinuity_threshold = self.discontinuity_threshold.clamp(0.20, 5.0);
        self.target_i = self.target_i.clamp(-30.0, -5.0);
        self.target_tp = self.target_tp.clamp(-9.0, 0.0);
        if self.ref_stream.trim().is_empty() {
            self.ref_stream = "0:a:0".to_string();
        }
        if self.log_filename_pattern.trim().is_empty() {
            self.log_filename_pattern = DEFAULT_LOG_FILENAME_PATTERN.to_string();
        }
        self.sanitize_window_geometry();
    }

    pub fn sanitize_window_geometry(&mut self) {
        if !self.window_width.is_finite() || self.window_width < MIN_WINDOW_WIDTH {
            self.window_width = DEFAULT_WINDOW_WIDTH;
        }
        if !self.window_height.is_finite() || self.window_height < MIN_WINDOW_HEIGHT {
            self.window_height = DEFAULT_WINDOW_HEIGHT;
        }
        if let (Some(x), Some(y)) = (self.window_x, self.window_y) {
            if !x.is_finite()
                || !y.is_finite()
                || !(-4096.0..=16384.0).contains(&x)
                || !(-4096.0..=16384.0).contains(&y)
            {
                self.window_x = None;
                self.window_y = None;
            }
        } else {
            self.window_x = None;
            self.window_y = None;
        }
    }

    pub fn has_saved_window_position(&self) -> bool {
        self.window_x.is_some() && self.window_y.is_some()
    }
}

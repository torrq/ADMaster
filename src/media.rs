use crate::config::AppConfig;
use serde::Deserialize;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

///ITU-R BS.775 True 2.0 stereo downmix matrix for 5.1/7.1 sources
pub const PAN_STEREO_BS775: &str =
    "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5";

#[derive(Debug, Clone)]
pub struct ToolPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub mkvmerge: Option<PathBuf>,
    pub mkvpropedit: Option<PathBuf>,
    pub mkvextract: Option<PathBuf>,
}

impl ToolPaths {
    pub fn discover(cfg: &AppConfig) -> Self {
        let common_dirs = &[
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/opt/local/bin",
            "/usr/bin",
            "/snap/bin",
        ];
        let ffmpeg = find_tool(&cfg.ffmpeg_path, "ffmpeg", common_dirs);
        let ffprobe = if !cfg.ffprobe_path.is_empty() && Path::new(&cfg.ffprobe_path).exists() {
            PathBuf::from(&cfg.ffprobe_path)
        } else if let Some(parent) = ffmpeg.parent() {
            let sibling = parent.join(exe_name("ffprobe"));
            if sibling.exists() {
                sibling
            } else {
                find_tool("", "ffprobe", common_dirs)
            }
        } else {
            find_tool("", "ffprobe", common_dirs)
        };

        let mkv_dirs = mkv_search_dirs();
        let mkv_dir_refs: Vec<&str> = mkv_dirs.iter().map(|s| s.as_str()).collect();
        let mkvmerge = find_optional_tool(&cfg.mkvmerge_path, "mkvmerge", &mkv_dir_refs);
        let mkvpropedit = find_optional_tool(&cfg.mkvpropedit_path, "mkvpropedit", &mkv_dir_refs);
        let mkvextract = find_optional_tool(&cfg.mkvextract_path, "mkvextract", &mkv_dir_refs);

        Self {
            ffmpeg,
            ffprobe,
            mkvmerge,
            mkvpropedit,
            mkvextract,
        }
    }
}

fn mkv_search_dirs() -> Vec<String> {
    let mut dirs = vec![
        r"C:\Program Files\MKVToolNix".to_string(),
        r"C:\Program Files (x86)\MKVToolNix".to_string(),
        "/Applications/MKVToolNix.app/Contents/MacOS".to_string(),
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
        "/opt/local/bin".to_string(),
        "/usr/bin".to_string(),
        "/snap/bin".to_string(),
    ];
    // Official macOS MKVToolNix bundles are often named /Applications/MKVToolNix-XX.0.app
    if let Ok(entries) = fs::read_dir("/Applications") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Some(s) = name.to_str() {
                if s.starts_with("MKVToolNix") && s.ends_with(".app") {
                    let macos_bin = entry.path().join("Contents").join("MacOS");
                    if macos_bin.is_dir() {
                        dirs.push(macos_bin.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    dirs
}

fn exe_name(base: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{}.exe", base)
    } else {
        base.to_string()
    }
}

fn candidate_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("bin"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent() {
            dirs.push(p.join("bin"));
            dirs.push(p.to_path_buf());
            if let Some(gp) = p.parent().and_then(|x| x.parent()) {
                dirs.push(gp.join("bin"));
            }
        }
    }
    dirs
}

fn which_in_path(exe: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let full = dir.join(exe);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

fn find_tool(custom: &str, name: &str, extra_dirs: &[&str]) -> PathBuf {
    if !custom.trim().is_empty() {
        let p = PathBuf::from(custom.trim());
        if p.exists() {
            return p;
        }
    }
    let exe = exe_name(name);
    for d in candidate_bin_dirs() {
        let cand = d.join(&exe);
        if cand.exists() {
            return cand;
        }
    }
    for &ed in extra_dirs {
        let cand = PathBuf::from(ed).join(&exe);
        if cand.exists() {
            return cand;
        }
    }
    if let Some(found) = which_in_path(&exe) {
        return found;
    }
    PathBuf::from(exe)
}

fn find_optional_tool(custom: &str, name: &str, extra_dirs: &[&str]) -> Option<PathBuf> {
    if !custom.trim().is_empty() {
        let p = PathBuf::from(custom.trim());
        if p.exists() {
            return Some(p);
        }
    }
    let exe = exe_name(name);
    for d in candidate_bin_dirs() {
        let cand = d.join(&exe);
        if cand.exists() {
            return Some(cand);
        }
    }
    for &ed in extra_dirs {
        let cand = PathBuf::from(ed).join(&exe);
        if cand.exists() {
            return Some(cand);
        }
    }
    which_in_path(&exe)
}

pub fn apply_no_window(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd;
}

/// Formats stream selector into `(ffprobe_selector, ffmpeg_map_selector)`.
pub fn format_stream_selector(stream_str: &str) -> (String, String) {
    let s = stream_str.trim().to_ascii_lowercase();
    if s.is_empty() {
        return ("a:0".to_string(), "0:a:0".to_string());
    }
    let stripped = s.strip_prefix("0:").unwrap_or(&s);
    if stripped.starts_with("a:") {
        (stripped.to_string(), format!("0:{}", stripped))
    } else if stripped.chars().all(|c| c.is_ascii_digit()) {
        (format!("a:{}", stripped), format!("0:a:{}", stripped))
    } else {
        (stripped.to_string(), format!("0:{}", stripped))
    }
}

#[derive(Debug, Clone)]
pub struct AudioStreamInfo {
    pub duration: f64,
    pub channels: u32,
    pub sample_rate: u32,
}

#[derive(Deserialize)]
struct FfprobeOutput {
    streams: Option<Vec<FfprobeStream>>,
    format: Option<FfprobeFormat>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    channels: Option<u32>,
    sample_rate: Option<String>,
    duration: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

pub fn get_audio_info(
    ffprobe_bin: &Path,
    input_path: &Path,
    stream_selector: &str,
) -> Result<AudioStreamInfo, String> {
    // Fast native path for standard RIFF / RF64 WAV files (no subprocess needed)
    if let Some(ext) = input_path.extension().and_then(|s| s.to_str()) {
        if ext.eq_ignore_ascii_case("wav") {
            if let Ok(hdr) = probe_wav_header(input_path) {
                return Ok(hdr);
            }
        }
    }

    let (select_stream, _) = format_stream_selector(stream_selector);
    let mut cmd = Command::new(ffprobe_bin);
    apply_no_window(&mut cmd);
    cmd.args([
        "-v",
        "error",
        "-select_streams",
        &select_stream,
        "-show_entries",
        "format=duration:stream=channels,channel_layout,duration,sample_rate",
        "-of",
        "json",
    ])
    .arg(input_path)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let out = cmd.output().map_err(|e| {
        format!(
            "Failed to run ffprobe on '{}': {}",
            input_path.display(),
            e
        )
    })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "ffprobe failed on '{}': {}",
            input_path.display(),
            err.trim()
        ));
    }

    let stdout_str = String::from_utf8_lossy(&out.stdout);
    let parsed: FfprobeOutput = serde_json::from_str(&stdout_str)
        .map_err(|e| format!("Failed to parse ffprobe JSON: {}", e))?;

    let mut channels = 2u32;
    let mut sample_rate = 48_000u32;
    let mut duration = 0.0f64;

    if let Some(streams) = parsed.streams {
        if let Some(st) = streams.into_iter().next() {
            if let Some(ch) = st.channels {
                channels = ch.max(1);
            }
            if let Some(sr_str) = st.sample_rate {
                if let Ok(sr) = sr_str.parse::<u32>() {
                    sample_rate = sr.max(1000);
                }
            }
            if let Some(d_str) = st.duration {
                if let Ok(d) = d_str.parse::<f64>() {
                    if d > 0.0 {
                        duration = d;
                    }
                }
            }
        }
    }
    if duration <= 0.0 {
        if let Some(fmt) = parsed.format {
            if let Some(d_str) = fmt.duration {
                if let Ok(d) = d_str.parse::<f64>() {
                    if d > 0.0 {
                        duration = d;
                    }
                }
            }
        }
    }

    Ok(AudioStreamInfo {
        duration,
        channels,
        sample_rate,
    })
}

/// Pre-decodes any audio/video stream to an 8kHz (or 16kHz) mono 16-bit PCM WAV with atomic temp-file replacement.
pub fn predecode_to_mono_wav(
    ffmpeg_bin: &Path,
    input_path: &Path,
    scratch_dir: &Path,
    label: &str,
    target_sr: u32,
    stream_map: Option<&str>,
    is_pal: bool,
    input_sr: u32,
    threads: usize,
) -> Result<PathBuf, String> {
    fs::create_dir_all(scratch_dir)
        .map_err(|e| format!("Failed to create scratch dir: {}", e))?;

    let stem = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    let raw_tag = format!("{}_{}", label, stem);
    let mut safe_name: String = raw_tag
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if is_pal {
        safe_name.push_str("_pal");
    }
    let suffix = format!("_mono{}k.wav", target_sr / 1000);
    let out_wav = scratch_dir.join(format!("{}{}", safe_name, suffix));
    let tmp_wav = scratch_dir.join(format!("{}_tmp{}", safe_name, suffix));

    if out_wav.exists() {
        if let Ok(meta) = fs::metadata(&out_wav) {
            if meta.len() > 1024 {
                return Ok(out_wav);
            }
        }
        let _ = fs::remove_file(&out_wav);
    }

    let mut af_chain = vec!["asetpts=PTS-STARTPTS".to_string()];
    if is_pal {
        let pal_rate = (input_sr as f64) * (23.976023976 / 25.0);
        af_chain.push(format!("asetrate={:.3}", pal_rate));
    }
    af_chain.push(format!("aresample={}", target_sr));
    af_chain.push("pan=mono|c0=c0".to_string());

    let mut cmd = Command::new(ffmpeg_bin);
    apply_no_window(&mut cmd);
    cmd.args([
        "-y",
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-threads",
        &threads.to_string(),
        "-fflags",
        "+genpts",
        "-i",
    ])
    .arg(input_path);

    if let Some(smap) = stream_map {
        if !smap.is_empty() {
            cmd.args(["-map", smap]);
        }
    }

    cmd.args([
        "-vn",
        "-af",
        &af_chain.join(","),
        "-c:a",
        "pcm_s16le",
        "-rf64",
        "auto",
        "-f",
        "wav",
    ])
    .arg(&tmp_wav)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let out = cmd
        .output()
        .map_err(|e| format!("Failed to spawn ffmpeg: {}", e))?;
    if !out.status.success() || !tmp_wav.exists() {
        let _ = fs::remove_file(&tmp_wav);
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "Failed to pre-decode '{}' to {}Hz mono WAV: {}",
            input_path.display(),
            target_sr,
            err.trim()
        ));
    }

    fs::rename(&tmp_wav, &out_wav).map_err(|e| format!("Failed to rename temp WAV: {}", e))?;
    Ok(out_wav)
}

/// Extracts or retrieves a clean 48kHz 2.0 stereo 16-bit PCM WAV downmix of the reference master audio.
pub fn extract_master_stereo_wav(
    ffmpeg_bin: &Path,
    ref_path: &Path,
    ref_stream_map: Option<&str>,
    channels: u32,
    scratch_dir: &Path,
    threads: usize,
) -> Result<PathBuf, String> {
    let ref_stereo_wav = scratch_dir.join("ref_master_stereo_48k.wav");
    if ref_stereo_wav.exists() {
        if let Ok(meta) = fs::metadata(&ref_stereo_wav) {
            if meta.len() > 1000 {
                return Ok(ref_stereo_wav);
            }
        }
    }

    let tmp_wav = scratch_dir.join("ref_master_stereo_48k_tmp.wav");
    let filter_str = if channels >= 6 {
        format!("{},asetpts=PTS-STARTPTS,aresample=48000", PAN_STEREO_BS775)
    } else {
        "asetpts=PTS-STARTPTS,aresample=48000,aformat=channel_layouts=stereo".to_string()
    };

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
        "-fflags",
        "+genpts",
        "-i",
    ])
    .arg(ref_path);

    if let Some(smap) = ref_stream_map {
        if !smap.is_empty() {
            cmd.args(["-map", smap]);
        }
    }

    cmd.args([
        "-vn",
        "-af",
        &filter_str,
        "-ac",
        "2",
        "-c:a",
        "pcm_s16le",
        "-rf64",
        "auto",
        "-f",
        "wav",
    ])
    .arg(&tmp_wav)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let out = cmd
        .output()
        .map_err(|e| format!("Failed to extract master stereo WAV: {}", e))?;
    if !out.status.success() || !tmp_wav.exists() {
        let _ = fs::remove_file(&tmp_wav);
        return Err(format!(
            "FFmpeg failed extracting master stereo WAV: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    fs::rename(&tmp_wav, &ref_stereo_wav)
        .map_err(|e| format!("Failed to rename master stereo WAV: {}", e))?;
    Ok(ref_stereo_wav)
}

#[derive(Debug, Clone)]
pub struct WavLayout {
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub audio_format: u16, // 1 = PCM, 3 = IEEE float
    pub data_offset: u64,
    pub data_bytes: u64,
    pub total_frames: u64,
}

/// Parses both standard RIFF WAV and 64-bit RF64 WAV headers without loading sample data into RAM.
pub fn parse_wav_layout(path: &Path) -> Result<WavLayout, String> {
    let mut f = File::open(path)
        .map_err(|e| format!("Failed to open WAV '{}': {}", path.display(), e))?;
    let file_len = f
        .metadata()
        .map(|m| m.len())
        .unwrap_or(0);

    let mut head = [0u8; 12];
    f.read_exact(&mut head)
        .map_err(|e| format!("WAV header too short in '{}': {}", path.display(), e))?;

    let is_rf64 = &head[0..4] == b"RF64";
    if (&head[0..4] != b"RIFF" && !is_rf64) || &head[8..12] != b"WAVE" {
        return Err(format!("Not a valid RIFF/RF64 WAVE file: {}", path.display()));
    }

    let mut rf64_data_size: Option<u64> = None;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits_per_sample = 0u16;
    let mut audio_format = 1u16;
    let mut data_offset = 0u64;
    let mut data_bytes = 0u64;

    loop {
        let mut chdr = [0u8; 8];
        if f.read_exact(&mut chdr).is_err() {
            break;
        }
        let cid = &chdr[0..4];
        let csize32 = u32::from_le_bytes([chdr[4], chdr[5], chdr[6], chdr[7]]) as u64;
        let cur_pos = f.stream_position().unwrap_or(0);

        if cid == b"ds64" && csize32 >= 24 {
            let mut ds = vec![0u8; csize32 as usize];
            f.read_exact(&mut ds)
                .map_err(|e| format!("Failed to read ds64 chunk: {}", e))?;
            rf64_data_size = Some(u64::from_le_bytes(ds[8..16].try_into().unwrap()));
        } else if cid == b"fmt " && csize32 >= 16 {
            let mut fmt = vec![0u8; csize32 as usize];
            f.read_exact(&mut fmt)
                .map_err(|e| format!("Failed to read fmt chunk: {}", e))?;
            audio_format = u16::from_le_bytes([fmt[0], fmt[1]]);
            channels = u16::from_le_bytes([fmt[2], fmt[3]]);
            sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
            bits_per_sample = u16::from_le_bytes([fmt[14], fmt[15]]);
            if audio_format == 0xFFFE && fmt.len() >= 40 {
                audio_format = u16::from_le_bytes([fmt[24], fmt[25]]);
            }
        } else if cid == b"data" {
            data_offset = cur_pos;
            data_bytes = if is_rf64 && csize32 == 0xFFFF_FFFF {
                rf64_data_size.unwrap_or_else(|| file_len.saturating_sub(cur_pos))
            } else if csize32 == 0xFFFF_FFFF {
                file_len.saturating_sub(cur_pos)
            } else {
                csize32.min(file_len.saturating_sub(cur_pos))
            };
            break;
        } else {
            let skip = csize32 + (csize32 & 1);
            if f.seek(SeekFrom::Current(skip as i64)).is_err() {
                break;
            }
        }
    }

    if channels == 0 || sample_rate == 0 || data_offset == 0 {
        return Err(format!("Incomplete WAV chunks in '{}'", path.display()));
    }
    let bytes_per_frame = (channels as u64) * ((bits_per_sample as u64) / 8).max(1);
    let total_frames = data_bytes / bytes_per_frame;

    Ok(WavLayout {
        channels,
        sample_rate,
        bits_per_sample,
        audio_format,
        data_offset,
        data_bytes,
        total_frames,
    })
}

pub fn probe_wav_header(path: &Path) -> Result<AudioStreamInfo, String> {
    let layout = parse_wav_layout(path)?;
    Ok(AudioStreamInfo {
        duration: (layout.total_frames as f64) / (layout.sample_rate as f64),
        channels: layout.channels as u32,
        sample_rate: layout.sample_rate,
    })
}

/// Reads a mono (or multi-channel averaged to mono) WAV file into `Vec<f32>` in `[-1.0, 1.0]`
/// using a tiny 64 KB streaming buffer (never allocates raw byte buffer + int16 buffer simultaneously).
pub fn read_wav_mono_f32(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let layout = parse_wav_layout(path)?;
    let mut f = BufReader::with_capacity(64 * 1024, File::open(path).map_err(|e| e.to_string())?);
    f.seek(SeekFrom::Start(layout.data_offset))
        .map_err(|e| e.to_string())?;

    let ch = layout.channels as usize;
    let total_frames = layout.total_frames as usize;
    let mut out = Vec::with_capacity(total_frames);

    if layout.bits_per_sample == 16 {
        let frame_bytes = ch * 2;
        let chunk_frames = 8192;
        let mut buf = vec![0u8; chunk_frames * frame_bytes];
        let mut remaining = total_frames;
        let inv = 1.0_f32 / 32768.0_f32;
        while remaining > 0 {
            let take = remaining.min(chunk_frames);
            let slice = &mut buf[..take * frame_bytes];
            if f.read_exact(slice).is_err() {
                break;
            }
            if ch == 1 {
                for pair in slice.chunks_exact(2) {
                    let s = i16::from_le_bytes([pair[0], pair[1]]);
                    out.push((s as f32) * inv);
                }
            } else {
                for frame in slice.chunks_exact(frame_bytes) {
                    let mut sum = 0.0_f32;
                    for c in 0..ch {
                        let s = i16::from_le_bytes([frame[c * 2], frame[c * 2 + 1]]);
                        sum += (s as f32) * inv;
                    }
                    out.push(sum / (ch as f32));
                }
            }
            remaining -= take;
        }
    } else if layout.bits_per_sample == 32 && layout.audio_format == 3 {
        let frame_bytes = ch * 4;
        let chunk_frames = 8192;
        let mut buf = vec![0u8; chunk_frames * frame_bytes];
        let mut remaining = total_frames;
        while remaining > 0 {
            let take = remaining.min(chunk_frames);
            let slice = &mut buf[..take * frame_bytes];
            if f.read_exact(slice).is_err() {
                break;
            }
            for frame in slice.chunks_exact(frame_bytes) {
                let mut sum = 0.0_f32;
                for c in 0..ch {
                    let s = f32::from_le_bytes([
                        frame[c * 4],
                        frame[c * 4 + 1],
                        frame[c * 4 + 2],
                        frame[c * 4 + 3],
                    ]);
                    sum += s;
                }
                out.push(sum / (ch as f32));
            }
            remaining -= take;
        }
    } else {
        return Err(format!(
            "Unsupported WAV sample format ({} bits, fmt={})",
            layout.bits_per_sample, layout.audio_format
        ));
    }

    Ok((out, layout.sample_rate))
}

/// Random-access WAV window reader: seeks directly to `start_frame` and reads `frame_count` frames
/// as interleaved stereo `f32` in `[-1.0, 1.0]` without loading the rest of the file into RAM.
pub struct WavStreamReader {
    file: File,
    pub layout: WavLayout,
}

impl WavStreamReader {
    pub fn open(path: &Path) -> Result<Self, String> {
        let layout = parse_wav_layout(path)?;
        let file = File::open(path).map_err(|e| e.to_string())?;
        Ok(Self { file, layout })
    }

    pub fn read_frames_stereo_f32(
        &mut self,
        start_frame: u64,
        frame_count: usize,
        out_interleaved: &mut Vec<f32>,
    ) -> Result<usize, String> {
        out_interleaved.clear();
        if start_frame >= self.layout.total_frames || frame_count == 0 {
            return Ok(0);
        }
        let avail = (self.layout.total_frames - start_frame) as usize;
        let take = frame_count.min(avail);
        let ch = self.layout.channels as usize;
        let bps = (self.layout.bits_per_sample as usize) / 8;
        let frame_bytes = ch * bps;

        let byte_off = self.layout.data_offset + start_frame * (frame_bytes as u64);
        self.file
            .seek(SeekFrom::Start(byte_off))
            .map_err(|e| e.to_string())?;

        let mut raw = vec![0u8; take * frame_bytes];
        let n_read = self.file.read(&mut raw).map_err(|e| e.to_string())?;
        let actual_frames = n_read / frame_bytes;
        out_interleaved.reserve(actual_frames * 2);

        if self.layout.bits_per_sample == 16 {
            let inv = 1.0_f32 / 32768.0_f32;
            for frame in raw[..actual_frames * frame_bytes].chunks_exact(frame_bytes) {
                let l = i16::from_le_bytes([frame[0], frame[1]]) as f32 * inv;
                let r = if ch >= 2 {
                    i16::from_le_bytes([frame[2], frame[3]]) as f32 * inv
                } else {
                    l
                };
                out_interleaved.push(l);
                out_interleaved.push(r);
            }
        } else if self.layout.bits_per_sample == 32 && self.layout.audio_format == 3 {
            for frame in raw[..actual_frames * frame_bytes].chunks_exact(frame_bytes) {
                let l = f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]);
                let r = if ch >= 2 {
                    f32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]])
                } else {
                    l
                };
                out_interleaved.push(l);
                out_interleaved.push(r);
            }
        } else {
            return Err("Unsupported WAV bit depth for windowed read".to_string());
        }

        Ok(actual_frames)
    }
}

/// Streaming 16-bit PCM WAV / RF64 writer (automatically writes 64-bit RF64 ds64 chunk when > 4GB).
pub struct WavStreamWriter {
    writer: BufWriter<File>,
    sample_rate: u32,
    channels: u16,
    frames_written: u64,
    is_rf64: bool,
    pcm_byte_buf: Vec<u8>,
}

impl WavStreamWriter {
    pub fn create(
        path: &Path,
        sample_rate: u32,
        channels: u16,
        expected_frames: u64,
    ) -> Result<Self, String> {
        let f = File::create(path)
            .map_err(|e| format!("Failed to create WAV '{}': {}", path.display(), e))?;
        let mut writer = BufWriter::with_capacity(128 * 1024, f);
        let expected_bytes = expected_frames * (channels as u64) * 2;
        let is_rf64 = expected_bytes > 0xFFFF_0000;

        Self::write_header(&mut writer, sample_rate, channels, 0, is_rf64)?;
        Ok(Self {
            writer,
            sample_rate,
            channels,
            frames_written: 0,
            is_rf64,
            pcm_byte_buf: Vec::with_capacity(64 * 1024),
        })
    }

    fn write_header(
        w: &mut BufWriter<File>,
        sr: u32,
        channels: u16,
        frames: u64,
        is_rf64: bool,
    ) -> Result<(), String> {
        let block_align = channels * 2;
        let byte_rate = sr * (block_align as u32);
        let data_bytes = frames * (block_align as u64);

        if is_rf64 {
            let riff_size64 = 64u64 + data_bytes;
            w.write_all(b"RF64").map_err(|e| e.to_string())?;
            w.write_all(&0xFFFF_FFFFu32.to_le_bytes())
                .map_err(|e| e.to_string())?;
            w.write_all(b"WAVE").map_err(|e| e.to_string())?;
            // ds64 chunk (28 bytes)
            w.write_all(b"ds64").map_err(|e| e.to_string())?;
            w.write_all(&28u32.to_le_bytes()).map_err(|e| e.to_string())?;
            w.write_all(&riff_size64.to_le_bytes())
                .map_err(|e| e.to_string())?;
            w.write_all(&data_bytes.to_le_bytes())
                .map_err(|e| e.to_string())?;
            w.write_all(&frames.to_le_bytes()).map_err(|e| e.to_string())?;
            w.write_all(&0u32.to_le_bytes()).map_err(|e| e.to_string())?;
        } else {
            let data32 = data_bytes.min(0xFFFF_FF00) as u32;
            let riff32 = 36u32.saturating_add(data32);
            w.write_all(b"RIFF").map_err(|e| e.to_string())?;
            w.write_all(&riff32.to_le_bytes()).map_err(|e| e.to_string())?;
            w.write_all(b"WAVE").map_err(|e| e.to_string())?;
        }

        // fmt chunk (16 bytes)
        w.write_all(b"fmt ").map_err(|e| e.to_string())?;
        w.write_all(&16u32.to_le_bytes()).map_err(|e| e.to_string())?;
        w.write_all(&1u16.to_le_bytes()).map_err(|e| e.to_string())?; // PCM
        w.write_all(&channels.to_le_bytes()).map_err(|e| e.to_string())?;
        w.write_all(&sr.to_le_bytes()).map_err(|e| e.to_string())?;
        w.write_all(&byte_rate.to_le_bytes()).map_err(|e| e.to_string())?;
        w.write_all(&block_align.to_le_bytes()).map_err(|e| e.to_string())?;
        w.write_all(&16u16.to_le_bytes()).map_err(|e| e.to_string())?;

        // data chunk header
        w.write_all(b"data").map_err(|e| e.to_string())?;
        let data_hdr32 = if is_rf64 {
            0xFFFF_FFFFu32
        } else {
            data_bytes.min(0xFFFF_FF00) as u32
        };
        w.write_all(&data_hdr32.to_le_bytes())
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn write_interleaved_f32(&mut self, samples: &[f32]) -> Result<(), String> {
        self.pcm_byte_buf.clear();
        self.pcm_byte_buf.reserve(samples.len() * 2);
        for &s in samples {
            let clamped = s.clamp(-1.0, 1.0);
            let q = (clamped * 32767.0).round() as i16;
            self.pcm_byte_buf.extend_from_slice(&q.to_le_bytes());
        }
        self.writer
            .write_all(&self.pcm_byte_buf)
            .map_err(|e| e.to_string())?;
        self.frames_written += (samples.len() as u64) / (self.channels as u64);
        Ok(())
    }

    pub fn finalize(mut self) -> Result<(), String> {
        self.writer.flush().map_err(|e| e.to_string())?;
        self.writer
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        Self::write_header(
            &mut self.writer,
            self.sample_rate,
            self.channels,
            self.frames_written,
            self.is_rf64,
        )?;
        self.writer.flush().map_err(|e| e.to_string())
    }
}

/// Multiplexes a video sample preview MKV using zero-re-encode stream copy (`-c:v copy -c:a copy`).
pub fn mux_video_sample(
    ffmpeg_bin: &Path,
    video_src: &Path,
    audio_src: &Path,
    out_mkv: &Path,
    render_duration: f64,
) -> Result<(), String> {
    let mut cmd = Command::new(ffmpeg_bin);
    apply_no_window(&mut cmd);
    cmd.args([
        "-y",
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "fatal",
        "-ss",
        "0",
        "-t",
        &format!("{:.3}", render_duration),
        "-i",
    ])
    .arg(video_src)
    .arg("-i")
    .arg(audio_src)
    .args([
        "-map",
        "0:v:0",
        "-map",
        "1:a:0",
        "-c:v",
        "copy",
        "-c:a",
        "copy",
        "-metadata:s:a:0",
        "title=Audio Description (Synced)",
        "-disposition:a:0",
        "default",
    ])
    .arg(out_mkv)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

#[derive(Deserialize)]
struct MkvMergeInfo {
    tracks: Option<Vec<MkvMergeTrack>>,
}

#[derive(Deserialize)]
struct MkvMergeTrack {
    id: u32,
    #[serde(rename = "type")]
    track_type: String,
    properties: Option<MkvMergeTrackProps>,
}

#[derive(Deserialize)]
struct MkvMergeTrackProps {
    language: Option<String>,
    language_ietf: Option<String>,
}

/// Multiplexes reference video, downmixed main stereo, and synced AD stereo tracks into a final MKV using `mkvmerge`.
pub fn mux_final_mkv(
    mkvmerge_bin: &Path,
    ref_mkv: &Path,
    main_stereo: &Path,
    ad_tracks_stereo: &[PathBuf],
    out_mkv: &Path,
) -> Result<(), String> {
    if !mkvmerge_bin.exists() {
        return Err("mkvmerge binary not found for final MKV multiplexing".to_string());
    }

    let mut probe_cmd = Command::new(mkvmerge_bin);
    apply_no_window(&mut probe_cmd);
    probe_cmd
        .arg("-J")
        .arg(ref_mkv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut video_ids = Vec::new();
    let mut eng_sub_ids = Vec::new();
    let probe_ok = if let Ok(res) = probe_cmd.output() {
        if res.status.success() {
            if let Ok(info) = serde_json::from_slice::<MkvMergeInfo>(&res.stdout) {
                for t in info.tracks.unwrap_or_default() {
                    if t.track_type == "video" {
                        video_ids.push(t.id);
                    } else if t.track_type == "subtitles" {
                        if let Some(p) = t.properties {
                            let lang = p.language.unwrap_or_default().to_ascii_lowercase();
                            let ietf = p.language_ietf.unwrap_or_default().to_ascii_lowercase();
                            if lang == "eng" || lang == "en" || ietf.starts_with("en") {
                                eng_sub_ids.push(t.id);
                            }
                        }
                    }
                }
            }
            true
        } else {
            false
        }
    } else {
        false
    };

    let mut mux_cmd = Command::new(mkvmerge_bin);
    apply_no_window(&mut mux_cmd);
    mux_cmd.arg("-o").arg(out_mkv).arg("--no-audio");

    if !eng_sub_ids.is_empty() {
        let sub_list: Vec<String> = eng_sub_ids.iter().map(|i| i.to_string()).collect();
        mux_cmd.args(["--stracks", &sub_list.join(",")]);
    } else if probe_ok {
        mux_cmd.arg("--no-subtitles");
    }
    mux_cmd.arg(ref_mkv);

    // Main stereo track
    mux_cmd
        .args([
            "--track-name",
            "0:Main, Stereo",
            "--language",
            "0:eng",
            "--default-track-flag",
            "0:yes",
        ])
        .arg(main_stereo);

    // AD stereo track(s)
    for ad_path in ad_tracks_stereo {
        mux_cmd
            .args([
                "--track-name",
                "0:AD, Stereo",
                "--language",
                "0:eng",
                "--default-track-flag",
                "0:no",
                "--visual-impaired-flag",
                "0:yes",
            ])
            .arg(ad_path);
    }

    if !video_ids.is_empty() {
        let mut order: Vec<String> = video_ids.iter().map(|id| format!("0:{}", id)).collect();
        order.push("1:0".to_string());
        for idx in 0..ad_tracks_stereo.len() {
            order.push(format!("{}:0", idx + 2));
        }
        for sid in &eng_sub_ids {
            order.push(format!("0:{}", sid));
        }
        mux_cmd.args(["--track-order", &order.join(",")]);
    }

    mux_cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let res = mux_cmd.output().map_err(|e| e.to_string())?;
    let code = res.status.code().unwrap_or(2);
    if (code == 0 || code == 1)
        && out_mkv.exists()
        && fs::metadata(out_mkv).map(|m| m.len()).unwrap_or(0) > 1000
    {
        Ok(())
    } else {
        Err(format!(
            "mkvmerge failed (code {}): {}",
            code,
            String::from_utf8_lossy(&res.stderr).trim()
        ))
    }
}

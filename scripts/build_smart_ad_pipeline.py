import struct
import os
import sys
import wave
import subprocess

temp_dir = r"scratch"
os.makedirs(temp_dir, exist_ok=True)

ffmpeg_bin = r"path/to/file\Local\Microsoft\WinGet\Packages\yt-dlp.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-N-125365-g9a01c1cb6a-win64-gpl\bin\ffmpeg.exe"

def parse_mpls(filepath):
    with open(filepath, 'rb') as f:
        data = f.read()
    
    playlist_pos = struct.unpack('>I', data[8:12])[0]
    pos = playlist_pos
    length = struct.unpack('>I', data[pos:pos+4])[0]
    num_playitems = struct.unpack('>H', data[pos+6:pos+8])[0]
    
    pos += 10
    playitems = []
    
    for _ in range(num_playitems):
        item_len = struct.unpack('>H', data[pos:pos+2])[0]
        item_start = pos
        clip_name = data[pos+2:pos+7].decode('ascii', errors='ignore')
        in_time = struct.unpack('>I', data[pos+14:pos+18])[0]
        out_time = struct.unpack('>I', data[pos+18:pos+22])[0]
        duration_sec = (out_time - in_time) / 45000.0
        
        playitems.append({
            'clip': clip_name,
            'duration': duration_sec
        })
        pos = item_start + 2 + item_len
        
    return playitems

theatrical = parse_mpls(r"X:\incoming\The.Martian.2015.EXTENDED.COMPLETE.BLURAY\DiSC1\BDMV\PLAYLIST\00800.mpls")
extended = parse_mpls(r"X:\incoming\The.Martian.2015.EXTENDED.COMPLETE.BLURAY\DiSC1\BDMV\PLAYLIST\00801.mpls")

ad_ac3_file = r"F:\uploads\The Martian - Theatrical Edition_t01_Audio04-AD.ac3"
ext_file = r"F:\uploads\The Martian - Extended Edition_t02_Audio02.flac"

out_flac = r"F:\uploads\The Martian - Extended Edition_t02_Audio-AD.flac"
out_ac3 = r"F:\uploads\The Martian - Extended Edition_t02_Audio-AD.ac3"

# Function to find exact end of speech after target_t in AD track
def get_sentence_end_time(ad_input_path, start_t, max_extend=3.5):
    sample_wav = os.path.join(temp_dir, "sample_speech.wav")
    cmd = [
        ffmpeg_bin, "-y", "-ss", f"{start_t:.3f}", "-t", f"{max_extend + 1.0:.3f}",
        "-i", ad_input_path,
        "-af", "pan=mono|c0=c2",
        "-c:a", "pcm_s16le", sample_wav
    ]
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if res.returncode != 0:
        return 0.0
        
    if not os.path.exists(sample_wav):
        return 0.0
        
    with wave.open(sample_wav, 'rb') as wf:
        nframes = wf.getnframes()
        framerate = wf.getframerate()
        raw_data = wf.readframes(nframes)
        samples = struct.unpack(f"<{nframes}h", raw_data)
        
    window_ms = 50 # 50ms windows
    window_size = int(framerate * (window_ms / 1000.0))
    
    extra_time = 0.0
    speech_threshold = 450 # RMS threshold for voice activity
    consecutive_silence_ms = 0
    
    for i in range(0, len(samples), window_size):
        chunk = samples[i:i+window_size]
        if not chunk:
            break
        rms = (sum(s*s for s in chunk) / len(chunk)) ** 0.5
        current_rel_sec = i / framerate
        
        if rms > speech_threshold:
            consecutive_silence_ms = 0
            extra_time = current_rel_sec + (window_ms / 1000.0)
        else:
            consecutive_silence_ms += window_ms
            if consecutive_silence_ms >= 200 and extra_time > 0:
                break
                
        if current_rel_sec >= max_extend:
            break
            
    return extra_time

print("Building Smart AD Timeline with Speech-Preserving Priority...")

t_cum = 0.0
e_cum = 0.0

slices = []

for i in range(len(extended)):
    t_item = theatrical[i]
    e_item = extended[i]
    
    t_dur = t_item['duration']
    e_dur = e_item['duration']
    
    is_shared = (t_item['clip'] == e_item['clip'])
    
    next_is_extended = False
    if i + 1 < len(extended):
        next_is_extended = (theatrical[i+1]['clip'] != extended[i+1]['clip'])
        
    actual_t_dur = t_dur
    actual_e_dur = e_dur
    
    # If this is a shared AD slice AND the next slice is an Extended scene,
    # check if speech extends past the boundary
    if is_shared and next_is_extended:
        cut_t = t_cum + t_dur
        extra_speech = get_sentence_end_time(ad_ac3_file, cut_t, max_extend=3.5)
        if extra_speech > 0.1:
            print(f"  [SPEECH PRESERVED] Item {i+1:02d} ({e_item['clip']}): Extending AD narration by +{extra_speech:.2f}s to avoid cutting off voice!")
            actual_t_dur += extra_speech
            
    slices.append({
        'index': i+1,
        'is_shared': is_shared,
        'clip': e_item['clip'],
        't_start': t_cum,
        't_dur': actual_t_dur,
        'e_start': e_cum,
        'e_dur': actual_e_dur
    })
    
    t_cum += t_dur
    e_cum += e_dur

print("\n--- EXTRACTING SMART SLICES ---")
chunk_files = []

for s in slices:
    i = s['index']
    chunk_filename = os.path.join(temp_dir, f"smart_chunk_{i:02d}.wav")
    
    if os.path.exists(chunk_filename):
        os.remove(chunk_filename)
        
    if s['is_shared']:
        cmd = [
            ffmpeg_bin, "-y",
            "-ss", f"{s['t_start']:.6f}", "-t", f"{s['t_dur']:.6f}",
            "-i", ad_ac3_file,
            "-af", "aformat=sample_fmts=s32:sample_rates=48000:channel_layouts=5.1",
            "-c:a", "pcm_s24le", chunk_filename
        ]
    else:
        cmd = [
            ffmpeg_bin, "-y",
            "-ss", f"{s['e_start']:.6f}", "-t", f"{s['e_dur']:.6f}",
            "-i", ext_file,
            "-af", "pan=5.1|c0=c0|c1=c1|c2=c2|c3=c3|c4=c4+0.7071*c6|c5=c5+0.7071*c7,aformat=sample_fmts=s32:sample_rates=48000:channel_layouts=5.1",
            "-c:a", "pcm_s24le", chunk_filename
        ]
        
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if res.returncode != 0:
        print(f"Error extracting smart chunk {i}: {res.stderr.decode('utf-8', errors='ignore')}")
        sys.exit(1)
        
    chunk_files.append(chunk_filename)

concat_list_file = os.path.join(temp_dir, "smart_concat_list.txt")
with open(concat_list_file, "w", encoding="utf-8") as f:
    for cf in chunk_files:
        f.write(f"file '{cf}'\n")

print("\n--- CONCATENATING ALL CHUNKS INTO FINAL 5.1 FLAC ---")
cmd_concat_flac = [
    ffmpeg_bin, "-y", "-f", "concat", "-safe", "0", "-i", concat_list_file,
    "-c:a", "flac", out_flac
]
res = subprocess.run(cmd_concat_flac, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
if res.returncode != 0:
    print(f"Error concatenating FLAC: {res.stderr.decode('utf-8', errors='ignore')}")
    sys.exit(1)

print(f"Successfully generated Smart 5.1 FLAC: {out_flac}")

print("\n--- ENCODING FINAL 5.1 AC3 (640 kbps) ---")
cmd_ac3 = [
    ffmpeg_bin, "-y", "-i", out_flac,
    "-c:a", "ac3", "-b:a", "640k", out_ac3
]
res = subprocess.run(cmd_ac3, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
if res.returncode != 0:
    print(f"Error encoding AC3: {res.stderr.decode('utf-8', errors='ignore')}")
    sys.exit(1)

print(f"Successfully generated Smart 5.1 AC3: {out_ac3}")
print("\n--- SMART PIPELINE COMPLETED SUCCESSFULLY! ---")

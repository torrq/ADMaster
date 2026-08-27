# SPDX-License-Identifier: MIT
import struct
import os
import sys
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

print("--- RENDERING OPTION A: SEAMLESS EXTENDED AD TRACK ---")
print("15 Shared Scenes -> Theatrical Audio Description Track (5.1)")
print("12 Extended Scenes -> Extended Edition Main Surround Mix (7.1 -> 5.1)\n")

t_cum = 0.0
e_cum = 0.0

chunk_files = []

for i in range(len(extended)):
    t_item = theatrical[i]
    e_item = extended[i]
    
    t_dur = t_item['duration']
    e_dur = e_item['duration']
    
    is_shared = (t_item['clip'] == e_item['clip'])
    chunk_filename = os.path.join(temp_dir, f"chunk_{i+1:02d}.wav")
    
    if os.path.exists(chunk_filename):
        os.remove(chunk_filename)
    
    fade_len = min(0.05, e_dur / 4.0)
    fade_out_st = max(0.0, e_dur - fade_len)
    
    if is_shared:
        # Extract slice from AD track directly into 5.1 24-bit PCM WAV
        cmd = [
            ffmpeg_bin, "-y",
            "-ss", f"{t_cum:.6f}", "-t", f"{t_dur:.6f}",
            "-i", ad_ac3_file,
            "-af", f"afade=t=in:st=0:d={fade_len:.4f},afade=t=out:st={fade_out_st:.4f}:d={fade_len:.4f},aformat=sample_fmts=s32:sample_rates=48000:channel_layouts=5.1",
            "-c:a", "pcm_s24le", chunk_filename
        ]
    else:
        # Extract slice from Extended Main FLAC (7.1 downmixed to 5.1 PCM WAV)
        cmd = [
            ffmpeg_bin, "-y",
            "-ss", f"{e_cum:.6f}", "-t", f"{e_dur:.6f}",
            "-i", ext_file,
            "-af", f"pan=5.1|c0=c0|c1=c1|c2=c2|c3=c3|c4=c4+0.7071*c6|c5=c5+0.7071*c7,afade=t=in:st=0:d={fade_len:.4f},afade=t=out:st={fade_out_st:.4f}:d={fade_len:.4f},aformat=sample_fmts=s32:sample_rates=48000:channel_layouts=5.1",
            "-c:a", "pcm_s24le", chunk_filename
        ]
        
    t_cum += t_dur
    e_cum += e_dur
        
    print(f"Extracting Chunk {i+1:02d}/{len(extended)}: {'[AD Track]' if is_shared else '[EXT Main Audio]'} Clip {e_item['clip']}.m2ts ({e_dur:.3f}s)")
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if res.returncode != 0:
        print(f"Error extracting chunk {i+1}: {res.stderr.decode('utf-8', errors='ignore')}")
        sys.exit(1)
        
    chunk_files.append(chunk_filename)

concat_list_file = os.path.join(temp_dir, "concat_list.txt")
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

print(f"Successfully generated 5.1 FLAC: {out_flac}")

print("\n--- ENCODING FINAL 5.1 AC3 (640 kbps) ---")
cmd_ac3 = [
    ffmpeg_bin, "-y", "-i", out_flac,
    "-c:a", "ac3", "-b:a", "640k", out_ac3
]
res = subprocess.run(cmd_ac3, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
if res.returncode != 0:
    print(f"Error encoding AC3: {res.stderr.decode('utf-8', errors='ignore')}")
    sys.exit(1)

print(f"Successfully generated 5.1 AC3: {out_ac3}")
print("\n--- OPTION A RENDERING COMPLETED SUCCESSFULLY! ---")

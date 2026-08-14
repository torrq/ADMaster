import os
import sys
import subprocess
import glob

DEFAULT_FFMPEG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "bin", "ffmpeg.exe")
if not os.path.exists(DEFAULT_FFMPEG):
    DEFAULT_FFMPEG = "ffmpeg"

DESCRIBEALIGN = r"describealign.py"

# 1. Render Movie Title 1
print("\n========================================================")
print("1. Rendering Movie Title 1 (Offset +8.979s)...")
print("========================================================")
hg_out = r"F:\uploads\MovieProjectDir\Movie Title 1\output"
os.makedirs(hg_out, exist_ok=True)
hg_ad = r"F:\uploads\MovieProjectDir\Movie Title 1\tosync\Movie Title 1 (Year) [AudioVault Original].mp3"
hg_stereo = os.path.join(hg_out, "Movie Title 1 (Year)-AD (Stereo).m4a")
hg_night = os.path.join(hg_out, "Movie Title 1 (Year)-AD (Night Mode).m4a")

cmd_hg = [
    DEFAULT_FFMPEG, "-y", "-hide_banner", "-loglevel", "warning",
    "-ss", "8.979", "-i", hg_ad,
    "-vn", "-af", "aresample=48000,alimiter=level_in=1:level_out=0.98:limit=0.98", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", hg_stereo,
    "-ss", "8.979", "-vn", "-af", "aresample=48000,loudnorm=I=-16:TP=-1.5:LRA=11", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", hg_night
]
res = subprocess.run(cmd_hg, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
if res.returncode == 0:
    print("  [SUCCESS] Movie Title 1 AD tracks rendered successfully!")
else:
    print(f"  [ERROR] Movie Title 1 render failed: {res.stderr}")


# 2. Run DescribeAlign for Movie Title 2
print("\n========================================================")
print("2. Aligning Movie Title 2 via DescribeAlign (DTW)...")
print("========================================================")
brian_scratch = r"F:\uploads\MovieProjectDir\Movie Title 2\scratch"
brian_out = r"F:\uploads\MovieProjectDir\Movie Title 2\output"
os.makedirs(brian_scratch, exist_ok=True)
os.makedirs(brian_out, exist_ok=True)

brian_ref = r"F:\uploads\MovieProjectDir\Movie Title 2\source\Life.of.Brian.1979.1080p.BluRay.FLAC2.0.x264-ZoroSenpai_track2_[eng]_DELAY 0ms.flac"
brian_ad = r"F:\uploads\MovieProjectDir\Movie Title 2\tosync\Movie Title 2 (Year) [AU].mp3"

cmd_brian_align = [
    sys.executable, DESCRIBEALIGN, "--yes", "--stretch_audio",
    brian_ref, brian_ad
]
print("  Running describealign on Movie Title 2...", flush=True)
res_brian = subprocess.run(cmd_brian_align, cwd=brian_scratch, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
print("  DescribeAlign stdout:", res_brian.stdout)
if res_brian.stderr:
    print("  DescribeAlign stderr:", res_brian.stderr)

# Locate output in scratch/videos_with_ad
aligned_candidates = glob.glob(os.path.join(brian_scratch, "videos_with_ad", "*"))
if aligned_candidates:
    aligned_brian = aligned_candidates[0]
    print(f"  Found aligned output: {aligned_brian}")
    
    # Render Stereo and Night Mode AAC
    brian_stereo = os.path.join(brian_out, "Movie Title 2 (Year)-AD (Stereo).m4a")
    brian_night = os.path.join(brian_out, "Movie Title 2 (Year)-AD (Night Mode).m4a")
    
    # Note: DescribeAlign audio-only output has the aligned AD as the audio track
    cmd_render_brian = [
        DEFAULT_FFMPEG, "-y", "-hide_banner", "-loglevel", "warning",
        "-i", aligned_brian,
        "-vn", "-af", "aresample=48000,alimiter=level_in=1:level_out=0.98:limit=0.98", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", brian_stereo,
        "-vn", "-af", "aresample=48000,loudnorm=I=-16:TP=-1.5:LRA=11", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", brian_night
    ]
    res_brian_render = subprocess.run(cmd_render_brian, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res_brian_render.returncode == 0:
        print("  [SUCCESS] Movie Title 2 AD tracks rendered successfully!")
    else:
        print(f"  [ERROR] Movie Title 2 render failed: {res_brian_render.stderr}")
else:
    print("  [ERROR] No aligned file produced for Movie Title 2")


# 3. Run DescribeAlign for Movie Title 3
print("\n========================================================")
print("3. Aligning Movie Title 3 via DescribeAlign (DTW)...")
print("========================================================")
mol_scratch = r"F:\uploads\MovieProjectDir\Movie Title 3\scratch"
mol_out = r"F:\uploads\MovieProjectDir\Movie Title 3\output"
os.makedirs(mol_scratch, exist_ok=True)
os.makedirs(mol_out, exist_ok=True)

mol_ref = r"F:\uploads\MovieProjectDir\Movie Title 3\source\The.Meaning.of.Life.1983.1080p.UHD.BluRay.DD+7.1.HDR10.x265-REBORN_track3_[eng]_DELAY 0ms.flac"
mol_ad = r"F:\uploads\MovieProjectDir\Movie Title 3\tosync\Monty Python's The Meaning of life.mp3"

cmd_mol_align = [
    sys.executable, DESCRIBEALIGN, "--yes", "--stretch_audio",
    mol_ref, mol_ad
]
print("  Running describealign on Movie Title 3...", flush=True)
res_mol = subprocess.run(cmd_mol_align, cwd=mol_scratch, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
print("  DescribeAlign stdout:", res_mol.stdout)
if res_mol.stderr:
    print("  DescribeAlign stderr:", res_mol.stderr)

aligned_mol_candidates = glob.glob(os.path.join(mol_scratch, "videos_with_ad", "*"))
if aligned_mol_candidates:
    aligned_mol = aligned_mol_candidates[0]
    print(f"  Found aligned output: {aligned_mol}")
    
    mol_stereo = os.path.join(mol_out, "Movie Title 3 (Year)-AD (Stereo).m4a")
    mol_night = os.path.join(mol_out, "Movie Title 3 (Year)-AD (Night Mode).m4a")
    
    cmd_render_mol = [
        DEFAULT_FFMPEG, "-y", "-hide_banner", "-loglevel", "warning",
        "-i", aligned_mol,
        "-vn", "-af", "aresample=48000,alimiter=level_in=1:level_out=0.98:limit=0.98", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", mol_stereo,
        "-vn", "-af", "aresample=48000,loudnorm=I=-16:TP=-1.5:LRA=11", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", mol_night
    ]
    res_mol_render = subprocess.run(cmd_render_mol, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res_mol_render.returncode == 0:
        print("  [SUCCESS] Movie Title 3 AD tracks rendered successfully!")
    else:
        print(f"  [ERROR] Movie Title 3 render failed: {res_mol_render.stderr}")
else:
    print("  [ERROR] No aligned file produced for Movie Title 3")

print("\n========================================================")
print("ALL MONTY PYTHON AD PROCESSING COMPLETED!")
print("========================================================")

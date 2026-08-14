import os
import subprocess

DEFAULT_FFMPEG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "bin", "ffmpeg.exe")
if not os.path.exists(DEFAULT_FFMPEG):
    DEFAULT_FFMPEG = "ffmpeg"

def encode_all(offset, atempo):
    out_dir = r"F:\uploads\MovieProjectDir\output"
    os.makedirs(out_dir, exist_ok=True)
    
    dts_input = r"F:\uploads\MovieProjectDir\source\Movie.Title.1992.BluRay.DTS.x264"
    ad_input = r"F:\uploads\MovieProjectDir\tosync\Movie Title (1992)"
    
    # Outputs
    main_ac3 = os.path.join(out_dir, "Movie Title (1992)")
    main_stereo = os.path.join(out_dir, "Movie Title (1992)")
    main_night = os.path.join(out_dir, "Movie Title (1992)")
    
    ad_stereo = os.path.join(out_dir, "Movie Title (1992)")
    ad_night = os.path.join(out_dir, "Movie Title (1992)")
    
    print("\n========================================================")
    print("1. Encoding Main DTS 5.1 -> AC3 5.1, Stereo AAC, Night Mode AAC...")
    print("========================================================")
    
    pan_filter = "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5"
    cmd_main = [
        DEFAULT_FFMPEG, "-y", "-hide_banner", "-loglevel", "warning", "-i", dts_input,
        "-c:a", "ac3", "-b:a", "384k", main_ac3,
        "-af", f"aresample=48000,{pan_filter},alimiter=level_in=1:level_out=0.98:limit=0.98", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", main_stereo,
        "-af", f"aresample=48000,{pan_filter},loudnorm=I=-16:TP=-1.5:LRA=11", "-ar", "48000", "-c:a", "aac", "-b:a", "256k", main_night
    ]
    res_main = subprocess.run(cmd_main, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res_main.returncode == 0:
        print("  [SUCCESS] Main audio tracks encoded!")
    else:
        print(f"  [ERROR] Main encode failed: {res_main.stderr}")
        
    print("\n========================================================")
    print("2. Encoding Synced AD Track -> Stereo AAC, Night Mode AAC...")
    print("========================================================")
    
    filters_ad_stereo = ["aresample=48000"]
    if abs(atempo - 1.0) > 0.00001:
        filters_ad_stereo.append(f"atempo={atempo:.8f}")
    if offset < 0:
        delay_ms = int(abs(offset) * 1000)
        filters_ad_stereo.append(f"adelay={delay_ms}:all=1")
    filters_ad_stereo.append("alimiter=level_in=1:level_out=0.98:limit=0.98")
    
    filters_ad_night = ["aresample=48000"]
    if abs(atempo - 1.0) > 0.00001:
        filters_ad_night.append(f"atempo={atempo:.8f}")
    if offset < 0:
        delay_ms = int(abs(offset) * 1000)
        filters_ad_night.append(f"adelay={delay_ms}:all=1")
    filters_ad_night.append("loudnorm=I=-16:TP=-1.5:LRA=11")
    
    cmd_ad = [DEFAULT_FFMPEG, "-y", "-hide_banner", "-loglevel", "warning", "-i", ad_input]
    if offset > 0:
        cmd_ad.extend(["-ss", f"{offset:.3f}"])
    cmd_ad.extend([
        "-vn", "-af", ",".join(filters_ad_stereo), "-ar", "48000", "-c:a", "aac", "-b:a", "256k", ad_stereo
    ])
    if offset > 0:
        cmd_ad.extend(["-ss", f"{offset:.3f}"])
    cmd_ad.extend([
        "-vn", "-af", ",".join(filters_ad_night), "-ar", "48000", "-c:a", "aac", "-b:a", "256k", ad_night
    ])
    
    res_ad = subprocess.run(cmd_ad, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res_ad.returncode == 0:
        print("  [SUCCESS] AD tracks encoded!")
    else:
        print(f"  [ERROR] AD encode failed: {res_ad.stderr}")

if __name__ == "__main__":
    import sys
    off = float(sys.argv[1]) if len(sys.argv) > 1 else 0.0
    at = float(sys.argv[2]) if len(sys.argv) > 2 else 1.0
    encode_all(off, at)

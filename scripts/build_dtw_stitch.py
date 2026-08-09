import sys
import os
import re
import subprocess

def time_to_sec(time_str):
    h, m, s = time_str.split(':')
    return int(h) * 3600 + int(m) * 60 + float(s)

def main():
    if len(sys.argv) < 4:
        print("Usage: python build_dtw_stitch.py <plot_txt> <input_mka> <output_m4a>")
        sys.exit(1)
        
    plot_file = sys.argv[1]
    input_audio = sys.argv[2]
    output_audio = sys.argv[3]
    
    with open(plot_file, "r", encoding="utf-8") as f:
        lines = f.readlines()
        
    audio_segments = []
    
    # Parse lines like: Rate change of      0.0% from  0:00:00.000 to  0:08:44.200 aligning with audio from  0:00:17.271 to  0:09:01.471
    regex = r"aligning with audio from\s+([0-9:\.]+)\s+to\s+([0-9:\.]+)"
    
    for line in lines:
        match = re.search(regex, line)
        if match:
            start_str, end_str = match.groups()
            start_sec = time_to_sec(start_str)
            end_sec = time_to_sec(end_str)
            audio_segments.append((start_sec, end_sec))
            
    if not audio_segments:
        print("No segments found in plot file.")
        sys.exit(1)
        
    print(f"Found {len(audio_segments)} audio segments to stitch:")
    for i, (start, end) in enumerate(audio_segments):
        print(f"  Segment {i+1}: {start:.3f} -> {end:.3f} (Duration: {end-start:.3f}s)")
        
    # Build filter graph
    graph = ""
    for i, (start, end) in enumerate(audio_segments):
        graph += f"[0:a]atrim=start={start}:end={end},asetpts=PTS-STARTPTS[a{i}]; "
        
    concat_inputs = "".join([f"[a{i}]" for i in range(len(audio_segments))])
    graph += f"{concat_inputs}concat=n={len(audio_segments)}:v=0:a=1[a_stitched]; "
    
    # Add loudnorm filter
    graph += f"[a_stitched]loudnorm=I=-16:TP=-1.5:LRA=11[a_out]"
    
    print("Filter graph:", graph)
    
    cmd = [
        "C:\\Users\\nathan\\Projects\\AudioDescriptionStuff\\bin\\ffmpeg.exe", "-y",
        "-i", input_audio,
        "-filter_complex", graph,
        "-map", "[a_out]",
        "-c:a", "aac", "-b:a", "256k", "-ar", "48000",
        output_audio
    ]
    
    print("Running ffmpeg...")
    subprocess.run(cmd, check=True)
    print("Done! Stitch saved to", output_audio)

if __name__ == "__main__":
    main()

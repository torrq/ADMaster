$ffmpeg = "C:\Users\nathan\Projects\AudioDescriptionStuff\bin\ffmpeg.exe"
$mkvmerge = "C:\Program Files\MKVToolNix\mkvmerge.exe"
$outDir = "F:\uploads\TPB\output"

# Process Safe Buffer AD Track
$ad_source = "F:\uploads\TPB\output\clean_master\TPB_Safe_Buffer_AD.wav"
$ad_aac = "F:\uploads\TPB\output\clean_master\TPB - Audio Description [AD] (Safe Buffer).m4a"

# Transcode VAD stitched AD WAV to AAC
& $ffmpeg -y -i $ad_source -af "loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 $ad_aac

# Mux the AAC AD track into the final MKV!
$clean_mkv = "F:\uploads\TPB\output\clean_master\Trailer Park Boys - S01E01 (Safe Buffer Master).mkv"
$final_mkv = "$outDir\Trailer Park Boys - S01E01 (Safe Buffer).mkv"

& $mkvmerge -o $final_mkv $clean_mkv $ad_aac

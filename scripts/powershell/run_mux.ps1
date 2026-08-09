$ffmpeg = "C:\Users\nathan\Projects\AudioDescriptionStuff\bin\ffmpeg.exe"
$mkvmerge = "C:\Program Files\MKVToolNix\mkvmerge.exe"
$outDir = "F:\uploads\TPB\output\clean_master"

# Process Perfect AD Track for Clean MKV
$ad_source = "F:\uploads\TPB\output\clean_master\TPB_Perfect_Clean_AD.wav"
$ad_aac = "$outDir\TPB - Audio Description [AD] (Synced Stereo).m4a"

# Transcode perfectly stitched AD WAV to AAC
& $ffmpeg -y -i $ad_source -af "loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 $ad_aac

# Mux the AAC AD track into the Clean MKV!
$clean_mkv = "F:\uploads\TPB\source\s01e01 - Take Your Little Gun and Get Out of My Trailer Park (Clean).mkv"
$final_mkv = "$outDir\Trailer Park Boys - S01E01 (Clean).mkv"

& $mkvmerge -o $final_mkv $clean_mkv $ad_aac

$mkvmerge = "C:\Program Files\MKVToolNix\mkvmerge.exe"
$outDir = "F:\uploads\TPB\output"

# Original MKV (contains original video with black screens and original audio)
$original_mkv = "F:\uploads\TPB\source\s01e01 - Take Your Little Gun and Get Out of My Trailer Park.mkv"

# Attempt 2 AAC AD Track (perfectly synced with inserted silence)
$ad_aac = "F:\uploads\TPB\output\attempt2\Trailer Park Boys - S01E01 - Audio Description (Synced Stereo).m4a"

# Final Output MKV
$final_mkv = "$outDir\Trailer Park Boys - S01E01 (Perfect Sync).mkv"

& $mkvmerge -o $final_mkv $original_mkv $ad_aac

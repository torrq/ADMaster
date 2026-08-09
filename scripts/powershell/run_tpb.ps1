$ffmpeg = "C:\Users\nathan\Projects\AudioDescriptionStuff\bin\ffmpeg.exe"
$outDir = "F:\uploads\TPB\output"

# Process Main Source (AC3 Stereo)
$source = "F:\uploads\TPB\source\s01e01 - Take Your Little Gun and Get Out of My Trailer Park_track2_[eng]_DELAY 26ms.ac3"

# Master Synced Stereo AC3
& $ffmpeg -y -i $source -c:a ac3 -b:a 256k "$outDir\Trailer Park Boys - S01E01 - Source (Synced Stereo).ac3"

# Master Synced Stereo AAC
& $ffmpeg -y -i $source -af "loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Trailer Park Boys - S01E01 - Source (Synced Stereo).m4a"

# Process AD Track (From describealign output)
$ad_source = "F:\uploads\TPB\videos_with_ad\ad_s01e01 - Take Your Little Gun and Get Out of My Trailer Park_track2_[eng]_DELAY 26ms.ac3"

# AD Synced Stereo MKA (with AAC)
& $ffmpeg -y -i $ad_source -c:a aac -b:a 256k "$outDir\Trailer Park Boys - S01E01 - Audio Description (Synced Stereo).mka"

# AD Synced Stereo AC3
& $ffmpeg -y -i $ad_source -c:a ac3 -b:a 256k "$outDir\Trailer Park Boys - S01E01 - Audio Description (Synced Stereo).ac3"

# AD Synced Stereo AAC
& $ffmpeg -y -i $ad_source -af "loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Trailer Park Boys - S01E01 - Audio Description (Synced Stereo).m4a"

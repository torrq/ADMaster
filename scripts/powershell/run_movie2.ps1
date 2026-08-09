$ffmpeg = "C:\Users\nathan\Projects\AudioDescriptionStuff\bin\ffmpeg.exe"
$outDir = "F:\uploads\Godfather Part II\output"

# Process Main Source
$source = "F:\uploads\Godfather Part II\source\The.Godfather.Part.II.1974.1080p.BluRay.DD5.1.x264-CtrlHD_track2_[eng]_DELAY 0ms.ac3"
& $ffmpeg -y -i $source -c:a ac3 -b:a 640k "$outDir\Godfather Part II - Source_Synced_5.1.ac3"
& $ffmpeg -y -i $source -af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Godfather Part II - Source_Synced_Stereo.m4a"

# Process AD Track (From describealign output)
$ad_source = "F:\uploads\Godfather Part II\videos_with_ad\ad_The.Godfather.Part.II.1974.1080p.BluRay.DD5.1.x264-CtrlHD_track2_[eng]_DELAY 0ms.ac3"

# Synced Stereo MP3
& $ffmpeg -y -i $ad_source -c:a libmp3lame -q:a 2 "$outDir\Godfather Part II - Audio Description [UK New Description] (Synced Stereo).mp3"

# Synced Stereo AC3
& $ffmpeg -y -i $ad_source -c:a ac3 -b:a 256k "$outDir\Godfather Part II - Audio Description [UK New Description] (Synced Stereo).ac3"

# Synced Stereo AAC
& $ffmpeg -y -i $ad_source -af "loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Godfather Part II - Audio Description [UK New Description] (Synced Stereo).m4a"

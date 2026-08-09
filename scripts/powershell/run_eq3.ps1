$ffmpeg = "C:\Users\nathan\Projects\AudioDescriptionStuff\bin\ffmpeg.exe"
$outDir = "F:\uploads\Equalizer 3\output"

# Source
$source = "F:\uploads\Equalizer 3\source\The Equalizer 3 2023 1080p WEBRip DD+5.1 x264-HiP_track2_[eng]_DELAY 0ms.eac3"
& $ffmpeg -y -i $source -c:a ac3 -b:a 640k "$outDir\Equalizer 3 - Source_Synced_5.1.ac3"
& $ffmpeg -y -i $source -af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Equalizer 3 - Source_Synced_Stereo.m4a"

# Track 2 (DTS)
$t2 = "F:\uploads\Equalizer 3\tosync\The Equalizer 3-SEG_FPL_MainFeature_BLK01_t00_track2_[eng]_DELAY 0ms.dts"
& $ffmpeg -y -i $t2 -af "adelay=955:all=1" -c:a dca -b:a 1536k -strict -2 "$outDir\Equalizer 3 - Track2_Synced_5.1.dts"
& $ffmpeg -y -i $t2 -af "adelay=955:all=1" -c:a ac3 -b:a 640k "$outDir\Equalizer 3 - Track2_Synced_5.1.ac3"
& $ffmpeg -y -i $t2 -af "adelay=955:all=1,pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Equalizer 3 - Track2_Synced_Stereo.m4a"

# Track 3 (DTS)
$t3 = "F:\uploads\Equalizer 3\tosync\The Equalizer 3-SEG_FPL_MainFeature_BLK01_t00_track3_[eng]_DELAY 0ms.dts"
& $ffmpeg -y -i $t3 -af "adelay=955:all=1" -c:a dca -b:a 1536k -strict -2 "$outDir\Equalizer 3 - Track3_Synced_5.1.dts"
& $ffmpeg -y -i $t3 -af "adelay=955:all=1" -c:a ac3 -b:a 640k "$outDir\Equalizer 3 - Track3_Synced_5.1.ac3"
& $ffmpeg -y -i $t3 -af "adelay=955:all=1,pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Equalizer 3 - Track3_Synced_Stereo.m4a"

# Track 4 (AC3)
$t4 = "F:\uploads\Equalizer 3\tosync\The Equalizer 3-SEG_FPL_MainFeature_BLK01_t00_track4_[eng]_DELAY 0ms.ac3"
& $ffmpeg -y -i $t4 -af "adelay=955:all=1" -c:a ac3 -b:a 640k "$outDir\Equalizer 3 - Track4_Synced_5.1.ac3"
& $ffmpeg -y -i $t4 -af "adelay=955:all=1,pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Equalizer 3 - Track4_Synced_Stereo.m4a"

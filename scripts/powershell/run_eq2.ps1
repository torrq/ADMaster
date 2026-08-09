$ffmpeg = "C:\Users\nathan\Projects\AudioDescriptionStuff\bin\ffmpeg.exe"
$outDir = "F:\uploads\Equalizer 2\output"

Remove-Item -Path "$outDir\*Preview*" -ErrorAction SilentlyContinue

# Source (AC3)
$source = "F:\uploads\Equalizer 2\source\The Equalizer 2 2018 1080p BluRay DD5.1 x264-VietHD_track1_[eng]_DELAY 0ms.ac3"
& $ffmpeg -y -i $source -c:a ac3 -b:a 640k "$outDir\Equalizer 2 - Source_Synced_5.1.ac3"
& $ffmpeg -y -i $source -af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Equalizer 2 - Source_Synced_Stereo.m4a"

# Track 2 (THD)
$t2 = "F:\uploads\Equalizer 2\tosync\Equalizer 2_t00_track2_[eng]_DELAY 0ms.thd"
& $ffmpeg -y -ss 0.007 -i $t2 -c:a ac3 -b:a 640k "$outDir\Equalizer 2 - Track2_Synced_5.1.ac3"
& $ffmpeg -y -ss 0.007 -i $t2 -af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Equalizer 2 - Track2_Synced_Stereo.m4a"

# Track 3 (AC3)
$t3 = "F:\uploads\Equalizer 2\tosync\Equalizer 2_t00_track3_[eng]_DELAY 0ms.ac3"
& $ffmpeg -y -ss 0.007 -i $t3 -c:a ac3 -b:a 640k "$outDir\Equalizer 2 - Track3_Synced_5.1.ac3"
& $ffmpeg -y -ss 0.007 -i $t3 -af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Equalizer 2 - Track3_Synced_Stereo.m4a"

# Track 4 (AC3)
$t4 = "F:\uploads\Equalizer 2\tosync\Equalizer 2_t00_track4_[eng]_DELAY 0ms.ac3"
& $ffmpeg -y -ss 0.007 -i $t4 -c:a ac3 -b:a 640k "$outDir\Equalizer 2 - Track4_Synced_5.1.ac3"
& $ffmpeg -y -ss 0.007 -i $t4 -af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Equalizer 2 - Track4_Synced_Stereo.m4a"

# Track 5 (AC3)
$t5 = "F:\uploads\Equalizer 2\tosync\Equalizer 2_t00_track5_[eng]_DELAY 0ms.ac3"
& $ffmpeg -y -ss 0.007 -i $t5 -c:a ac3 -b:a 640k "$outDir\Equalizer 2 - Track5_Synced_5.1.ac3"
& $ffmpeg -y -ss 0.007 -i $t5 -af "pan=stereo|c0=c0+0.7071*c2+0.7071*c4|c1=c1+0.7071*c2+0.7071*c5,loudnorm=I=-16:TP=-1.5:LRA=11" -c:a aac -b:a 256k -ar 48000 "$outDir\Equalizer 2 - Track5_Synced_Stereo.m4a"

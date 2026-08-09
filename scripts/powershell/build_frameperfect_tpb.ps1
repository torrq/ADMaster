$ffmpeg = "C:\Users\nathan\Projects\AudioDescriptionStuff\bin\ffmpeg.exe"

$original_mkv = "F:\uploads\TPB\source\s01e01 - Take Your Little Gun and Get Out of My Trailer Park.mkv"
$ad_mka = "F:\uploads\TPB\tosync\Trailer Park Boys 1x01 Take Your Little Gun and Get Out of My Trailer Park 70143633 2.0 English - Audio Description.mka"
$final_mkv = "F:\uploads\TPB\output\Trailer Park Boys - S01E01 (Frame Perfect).mkv"

# Filter graph for frame-perfect cut and sync
$filter = @"
[0:v]trim=start=0:end=512.4,setpts=PTS-STARTPTS[v1];
[0:a]atrim=start=0:end=512.4,asetpts=PTS-STARTPTS[a1];

[0:v]trim=start=522.176:end=940.1,setpts=PTS-STARTPTS[v2];
[0:a]atrim=start=522.176:end=940.1,asetpts=PTS-STARTPTS[a2];

[0:v]trim=start=949.777,setpts=PTS-STARTPTS[v3];
[0:a]atrim=start=949.777,asetpts=PTS-STARTPTS[a3];

[v1][a1][v2][a2][v3][a3]concat=n=3:v=1:a=1[v_out][a_out_master];

[1:a]atrim=start=17.788,asetpts=PTS-STARTPTS[a_out_ad]
"@

# Run FFmpeg to re-encode video and mux perfectly synced audio tracks
& $ffmpeg -y -i $original_mkv -i $ad_mka -filter_complex $filter -map "[v_out]" -map "[a_out_master]" -map "[a_out_ad]" -c:v libx264 -preset fast -crf 18 -c:a:0 ac3 -b:a:0 384k -c:a:1 aac -b:a:1 256k -ar:a:1 48000 $final_mkv

---
name: audio-description-frame-perfect-sync
description: Frame-perfect video re-encoding and audio description alignment for removing commercial breaks seamlessly without keyframe snapping or audio jumbling.
---

# Advanced Audio Description Syncing & Commercial Removal

This skill defines the standard procedures for removing commercial breaks from master MKV files and perfectly syncing uninterrupted TV-rip Audio Description (AD) tracks, ensuring zero loss of show footage and zero narrator cut-offs.

## 1. The Core Problem: Keyframe Snapping
When removing commercial black screens from highly compressed H.264/AVC MKV files, standard cutters (like `mkvmerge --split`) are physically incapable of cutting at exact milliseconds. They **snap to video keyframes (I-frames)**.

* **The Video Consequence**: `mkvmerge` will inadvertently delete 1-2 seconds of actual show footage adjacent to the commercial break.
* **The Audio Consequence**: Because show footage is permanently deleted, the synced AD track must be forced to skip that same footage. If the narrator is speaking across the transition, the narrator's sentence will be aggressively chopped in half, or they will end up "talking over themselves" if the audio is overlaid.

We have three distinct workflows to solve this, depending on the user's needs.

---

## Workflow A: Frame-Perfect FFmpeg Re-encode (The "Director's Cut")
To achieve a completely seamless, zero-black-screen transition, you **must bypass keyframe snapping** by re-encoding the video stream via FFmpeg using a complex filter graph.

### Step 1: Detect Commercial Breaks
Use FFmpeg's `blackdetect` to find the exact start and end timestamps of the commercial black screens.
* Example: Black screen detected from `512.400s` to `522.176s` (Duration: `9.776s`).

### Step 2: Calculate Global Initial AD Offset
Run `scripts/python/sync_ad_track.py` between the Master MKV and the AD track to find the initial global offset. 
* If Offset = `+17.788s`, trim `17.788s` from the AD track so it aligns with the Master video at `0s`.

### Step 3: Execute the Frame-Perfect Build
Run a single FFmpeg command with a multi-part `trim`/`atrim` filter graph that precisely preserves the show footage and completely drops the black screens.

```bash
ffmpeg -y -i original_master.mkv -i ad_track.mka \
    -filter_complex "[0:v]trim=start=0:end=512.4,setpts=PTS-STARTPTS[v1]; [0:a]atrim=start=0:end=512.4,asetpts=PTS-STARTPTS[a1]; [0:v]trim=start=522.176:end=940.1,setpts=PTS-STARTPTS[v2]; [0:a]atrim=start=522.176:end=940.1,asetpts=PTS-STARTPTS[a2]; [0:v]trim=start=949.777,setpts=PTS-STARTPTS[v3]; [0:a]atrim=start=949.777,asetpts=PTS-STARTPTS[a3]; [v1][a1][v2][a2][v3][a3]concat=n=3:v=1:a=1[v_out][a_out_master]; [1:a]atrim=start=17.788,asetpts=PTS-STARTPTS[a_out_ad]" \
    -map "[v_out]" -map "[a_out_master]" -map "[a_out_ad]" \
    -c:v libx264 -preset fast -crf 18 -c:a:0 ac3 -b:a:0 384k -c:a:1 aac -b:a:1 256k -ar:a:1 48000 \
    final_perfect_sync.mkv
```

---

## Workflow B: The 2-Second "Safe Buffer" (The Fast Method)
If the user wants instant processing (no video re-encoding) and doesn't mind a brief black screen, use the "Safe Buffer" logic to trick `mkvmerge`.

1. **Shrink the Cut Window**: Take the detected black screen (e.g., `512.212` to `522.355`) and add a 2.0-second safe buffer to both ends (`514.212` to `520.355`).
2. **Execute `mkvmerge`**: Tell `mkvmerge` to cut out this smaller inner window. When it snaps to keyframes, it will snap to keyframes *still inside the black screen*.
3. **The Result**: Zero show footage is deleted. The audio never gets jumbled. The transition features a quick ~4-second black screen, and the process takes 3 seconds to complete.

---

## Workflow C: VAD Speech Tail Preservation (The Keyframe Rescue)
If you *must* use `mkvmerge` without a safe buffer (resulting in deleted show footage), you must use Voice Activity Detection (VAD) to rescue the narrator's voice.

1. **Isolate the Missing Audio**: Calculate exactly how much audio was deleted by keyframe snapping (e.g., `2.068s`), and extract that exact chunk from the uninterrupted AD track.
2. **Center-Channel Extraction**: Collapse that chunk to mono (`(L+R)/2`) to isolate the narrator's voice from the stereo music background.
3. **Seamless Overlay**: Apply a 50ms fade-out and mathematically mix/overlay this mono chunk directly onto the beginning of the next scene (`next_seg[:mix_len] += gap_overlay[:mix_len]`).
4. **The Result**: The narrator seamlessly finishes their sentence over the transition cut, without throwing the rest of the timeline out of sync.

## Critical Rules
* **Never use mkvmerge for complete commercial removal if seamless audio is required.** `mkvmerge` will always snap to keyframes, breaking audio alignment.
* **Never use describealign on a keyframe-snapped MKV.** It will cause micro-drifts and incorrectly map audio across the missing show footage.

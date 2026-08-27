# Audio Description Pipeline Scripts

This directory contains the production-ready automation scripts for the Audio Description sync, main audio conversion, and TV episode processing pipeline.

## Core Production Tools

* **`sync_ad_track.py`** - Master universal acoustic sync & encoding tool. Computes 100-anchor in-memory FFT correlation, candidate clustering ($\pm 30\text{ms}$ sub-frame tolerance), dual AD variant rendering (`Stereo` + `Night Mode`), and triple Main audio conversion (`--convert-main`: 5.1 AC3 Surround, 2.0 Stereo AAC, 2.0 Night Mode AAC).
* **`batch_orchestrator.py`** - Master TV batch orchestrator. Automatically determines the optimal sync strategy (Direct Mux vs Frame-Perfect Black Screen Cuts vs DTW `describealign` + smooth stitch) per episode.
* **`set_commentary_flags.py`** - In-place Matroska track property manager. Inspects MKV headers via `mkvmerge -J` and sets `flag-commentary=1` instantly via `mkvpropedit`.
* **`plex_ad_commentary_collections.py`** - Plex library scanner and collection builder for files containing `flag-visual-impaired` and `flag-commentary`.
* **`convert_main_tracks.py`** - Dedicated converter for extracting and downmixing 5.1/7.1 surround masters into true 2.0 Stereo AAC (`alimiter`) and Night Mode AAC (`loudnorm`).
* **`dtw_smooth_stitch.py`** - Smooth DTW audio stitcher with 3-layer transition pipeline: VAD-aware cut nudging (protects speech at boundaries), zero-crossing snapping (eliminates clicks), and equal-power crossfades (smooth blending).
* **`batch_frameperfect.py`** - Automates Frame-Perfect FFmpeg cut workflow across MKV episodes to remove black screen commercial breaks without keyframe snapping.
* **`remove_commercial_breaks.py`** - Scans MKVs for black screens using FFmpeg `blackdetect` and generates slice timings.
* **`convert_ad_2pass.py`** - Performs standalone 2-pass EBU R128 Loudnorm normalization on 5.1/7.1 audio tracks without alignment.
* **`parse_mpls.py`** - Parses Blu-ray `.mpls` playlist files to extract 45 kHz PTS timestamps.

# PowerShell Scripts Index

This directory contains executable PowerShell wrappers and pipeline orchestrators for the Audio Description tools.

## Master Pipelines
* `build_frameperfect_tpb.ps1` - The ultimate FFmpeg filter-graph pipeline wrapper. Rebuilds the episode by re-encoding video and losslessly mapping the audio to slice out exact milliseconds of black screens. Generates a flawless "Director's Cut" MKV.

## Test & EQ Automation
* `run_eq2.ps1` - Automation wrapper to rapidly run EQ test profiles (Profile 2).
* `run_eq3.ps1` - Automation wrapper to rapidly run EQ test profiles (Profile 3).
* `run_godfather2.ps1` - Automation wrapper used for rendering the Godfather Part II AD timeline tests.

## Trailer Park Boys Batch Wrappers
* `run_tpb.ps1` - Basic TPB batch runner wrapper for executing the Python sync logic.
* `run_tpb_attempt2.ps1` - Alternative batch runner wrapper for TPB alignment.
* `run_tpb_mux.ps1` - Standard MKV muxing wrapper.
* `run_tpb_mux_attempt2.ps1` - Alternative MKV muxing wrapper.
* `run_tpb_mux_safebuffer.ps1` - Orchestrates the Python "Safe Buffer" timeline sync.
* `run_tpb_mux_vad.ps1` - Orchestrates the Python VAD (Voice Activity Detection) keyframe rescue muxing.

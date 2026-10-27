# Rich media: manual checks

Use **copies** of media in a separate folder, especially for rename/delete checks. No automated test files were added. These are checks to run manually; build success does not establish that they pass.

## Build and platform

The implemented native backend targets macOS. It uses ImageIO, AVFoundation, CoreGraphics and Vision, linked into the existing Rust application through a small Objective-C bridge. The face runtime includes pinned pillow-heif. Native rendering does not require a system ffmpeg installation.

Windows/Linux libheif and bundled ffmpeg backends are still outstanding. Do not treat the current build as cross-platform completion of the proposal.

## Prepare a small library

Include:

- JPEG controls, rotated JPEGs and the same images exported as HEIC/JPEG.
- 12MP and 48MP HEIC, HEIF/HIF/AVIF, an HDR/P3 HEIC, and HEIC bytes renamed to `.JPG`.
- One-frame GIF, animated GIFs below/above 8 MB, animated WebP, APNG and an HEICS sample supported by macOS.
- Apple HEIC/JPEG + MOV pairs, edited `IMG_E…` pairs, a MOV without a still, unrelated same-name files, and duplicates that make pairing ambiguous.
- Pixel Motion Photos with legacy offsets and Container metadata; Samsung SEFT and `mpv2` samples.
- GIFs with scene cuts and people appearing in different frames; Live Photos with a person visible only in the motion.

Add the folder through Settings and wait for indexing. Keep the original copies outside the scanned folder for comparison.

## Check one feature at a time

| Check | Actions | Expected result |
|---|---|---|
| Existing JPEG behavior | Browse JPEG controls, rotate/zoom visually, open People. | Thumbnails, orientation, labels and capture dates remain correct. Existing JPEG thumbnail cache names and face input revisions retain their previous scheme. |
| Format sniffing | Add HEIC renamed to `.JPG`; browse alongside normal HEIC/HEIF/HIF/AVIF. | Native decoding follows file contents; each supported file has a usable thumbnail and viewer image. Unsupported codecs produce an error rather than a crash. |
| HDR and orientation | Compare portrait, mirrored and HDR/P3 files with macOS Preview. | Thumbnails are upright, SDR display colors are reasonable, Info reports profile/HDR when available. Zoom above 1.5× on an unedited still uses the native original. |
| Progressive viewer | Open a tile whose thumbnail has loaded; immediately move forward/backward several times. | The cached thumbnail appears first, followed by a sharper derivative. A delayed request must never replace a different selected photo. |
| Video metadata | Add a MOV with capture date/GPS and a different filesystem modification date. | Capture date determines ordering when metadata is available; dimensions, duration and location appear. |
| Animated classification | Compare a one-frame GIF with GIF/WebP/APNG animations. | The one-frame GIF has no animation badge; animations have GIF/ANIMATED badges, frame counts and durations. |
| Thumbnails and placeholders | Delete nothing; launch after upgrading and index a library. Scroll a fresh library before indexing finishes, then again after. | `thumbnails/` in the cache dir is removed and `thumbs-v3/` holds `.webp` files (about 512px, smaller than the old JPEGs). Tiles show a blurred ThumbHash colour preview and fade to the sharp thumbnail; after the background warmer finishes the grid refreshes once and every tile has a preview. Videos and HEIC thumbnails are WebP too; edited files regenerate their hash. |
| Grid settings | Try Always / On hover / Never, scroll away while previews play, then return. | Only intersecting tiles play. No more than six animation previews and one Live preview are admitted; leaving a tile/viewport releases playback. |
| Animated viewer | Open a small GIF, then one larger than 8 MB. | Small files use native image animation. Large files switch to the cached H.264 derivative; Space pauses/resumes, comma/period step while paused, and speed changes work. Zoom/pan remain usable. |
| Live pairing | Add still first, then its MOV; repeat in reverse order. | Reconciliation produces one LIVE tile and one counted library item. The MOV remains in the portable DB. Info includes the combined pair size. |
| Pair precision | Add same-stem MOV longer than 4 seconds, a timestamp mismatch, differing content IDs and multiple ambiguous stills. | Those files stay separate. No partner is used twice. |
| Unpair/re-pair | Move one component out of the scanned directory, wait, then restore it. | The remaining component becomes a regular still/video, and pairing is restored when valid. |
| Live playback | Open a LIVE tile, toggle autoplay, hold Space, hold the image without dragging, release, then try hover and mute. | Autoplay follows settings. Holding starts motion; release/end returns to the still. The video appears after a presented frame, without a deliberate black placeholder. Hover is muted. |
| Live effects | Select Loop, Bounce, Long Exposure, Off and Live in turn. Reopen/restart. | Choices persist. Loop repeats with an endpoint blend; Bounce reverses smoothly; Long Exposure shows an aligned average; Off stays still. Compare stabilized borders and moving subjects for artifacts. |
| Render failures | Temporarily make a motion component unavailable before selecting an effect. | The UI shows an actionable error. Failed pre-rendering does not commit the effect selection. |
| Key photo | Open Key photo, choose a filmstrip frame, then use the time slider. Restart and finally choose Original. | Chosen frame persists, grid/viewer refresh, and face work is re-queued. Original restores the source still. Choosing a frame outside the clip is rejected. |
| Exports | Save still, video and GIF with distinct new filenames. Open them outside GPhotos. | JPEG is readable; Live video retains the original motion/audio; Loop/Bounce export their rendered MP4; GIF reflects Loop/Bounce when selected. Source files do not change. Existing export destinations are rejected rather than overwritten. |
| Pair rename | Rename through Info; try a name whose corresponding MOV already exists. | Both components acquire the new stem. A conflicting destination fails before renaming. Labels and portable IDs survive. |
| Pair delete | First remove a pair from the library. Separately, delete a disposable pair from disk. | Confirmation identifies both files. Library removal excludes both; disk deletion stages both before removing them. No orphan companion becomes a visible library item. |
| Embedded motion | Open Pixel and Samsung samples; seek near the end. | One item appears. Playback reads the validated embedded MP4 slice; effects/key extraction can create disposable cached MP4s. Invalid offsets must not read outside the file. |
| HEIC faces | Compare HEIC/JPEG exports, including 48MP HEIC. | HEIC inference uses a native reduced-size input with boxes mapped to original dimensions. Face crops align. Previously failed HEIF/AVIF rows retry on decoder upgrade; completed JPEGs do not rerun merely because of the upgrade. |
| Animated faces | Scan a GIF containing recurring people and cuts. | Up to eight frames are sampled, with endpoints and scene-change candidates. Repeated faces are associated into tracks. Inspect crops from later frames. |
| Deep Live faces | Scan with Deep scan off; enable it in Settings, wait, and inspect People. | Default scans the still/key frame. Deep scan adds quarter/half/three-quarter samples. Faces already represented by the still are not intentionally duplicated. |
| People frame opening | Click an item containing a motion/animation-sourced face. | The viewer opens the corresponding clip at the stored timestamp and pauses. Face cards/review crops use the recorded frame source. |
| Label preservation | Name a person; change a key frame, replace motion with a modified copy, rescan. | Confident frame-compatible matches retain labels. Uncertain/manual records remain available for review rather than silently being reassigned. |
| Cache | Change quota in Settings, browse/render enough media to exceed it, wait over ten minutes. | The minute sweeper removes least-recently-used eligible derivatives, keeping a ten-minute grace period. Reopening evicted content regenerates it. Edits remain in `.data/library.db`. |
| Failures | Add a truncated GIF, corrupt HEIC and invalid embedded offset. Navigate away during rendering. | Browsing remains usable; stale async results do not appear on another photo; errors are visible. |
| Raw HEIC faces | Rebuild the face runtime, then scan HEIC and 48MP HEIC. | All HEIF/AVIF rows re-run once and JPEG rows do not. Face crops line up with the photo. `cache/face-raw` is empty afterwards. |
| xxh3 skip | Re-run the scan, or touch a HEIC without changing its pixels. | Unchanged HEIC files finish without a worker call and their stored digest starts with `x3:`. |
| Deep Live + HEIC | Turn on deep scan with a HEIC Live Photo. | Faces appear from the still and from motion frames, and a second scan is a no-op. |
| Hardware faces | Turn on the hardware face-recognition toggle, wait, then turn it off. | Faces are re-embedded under `-coreml`. With the toggle off, CPU results return. If CoreML is missing, a clear "CoreML unavailable" error appears and no vectors are stored. |

## Performance measurements

Launch with `GPHOTOS_PERF=1`. Rust writes NDJSON spans named `decode`, `thumb`, `display`, `render`, `pair`, and `detect` under the application's log directory as `perf.ndjson`; motion-start is reported by the viewer. Use Activity Monitor and Web Inspector for memory, frame rate and browser timing.

Repeat cold and warm runs separately. Record p50/p95/p99 and corpus size, machine, OS, display scale, and whether face indexing ran simultaneously. Compare with §9 of the supplied proposal. **No latency, 60-fps, accuracy, memory or parity target is certified by the build checks.**

## Remaining proposal items

- Windows/Linux dynamic libheif and a verified LGPL ffmpeg sidecar, including packaging and platform validation.
- Full Samsung HEIF `sefd` variants and generalized multi-item Container padding/offset layouts; current Samsung support is the bounded SEFT directory path and `mpv2` indirection.
- Embedded-thumbnail selection, small animated WebP derivatives and a single shared grid video element. Animations currently use H.264 derivatives in the grid.
- OS-level autoplay/mute detection and a full shared zoom-action/viewer-shell refactor. Mute is remembered per browser profile; ±1 and idle ±2 still display images are prefetched.
- Proactive background regeneration of evicted selected effects. Regeneration currently occurs on demand.
- Reference-corpus runs, platform/codec coverage, face-parity statistics and every performance/precision target in §9.

The implementation is a substantial macOS delivery, but the proposal's full definition of done remains open until these items and the manual corpus checks are completed.

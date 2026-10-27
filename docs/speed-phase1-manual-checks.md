# Phase 1: implementation and manual checks

Only Phase 1 of `speed_and_features_plan.md` is implemented. No automated tests were added. Later phases and the feature wishlist are outside this change.

## Implemented

- macOS reduced-size still previews use ImageIO with orientation applied and a 512-pixel thumbnail limit. If native decoding fails, a usable embedded EXIF thumbnail (long edge at least 256 pixels) is tried before the existing decoder. Full-resolution JPEG decoding used for faces remains unchanged.
- Folder writes commit together after approximately 200 files or 250 ms, and before publishing a projection. SQLite uses WAL with `synchronous=NORMAL`.
- Initial results publish after approximately 25 indexed files or 300 ms; subsequent projections use 500 files or 2 seconds. Thresholds are checked between files, so a slow individual file can delay publication. Projections schedule thumbnail warming during scanning.
- Preview concurrency defaults to available CPU parallelism minus one, with a minimum of one worker.
- Indexing passes the detected media kind into parsing and reuses one rich-metadata probe, including video duration.
- Newly generated thumbnails produce ThumbHash from the decoded image before WebP encoding. A cached image with missing hash metadata has a recovery fallback.

Thumbnail cache version is now `thumb-webp512-v4`. Old disposable preview metadata is cleared once; face parser/cache versions are unchanged. Pairing remains at folder finalization, when same-stem candidates have been enumerated.

## Manual checklist

Use a disposable copy of representative media for interruption, rename, and file-change checks. Record the app version, Mac model, storage type, file count, and whether caches are cold when comparing timings.

1. **Early results and overlapping work:** Add a fresh folder with several thousand JPEGs. Confirm photos appear while indexing is still running, and thumbnails begin filling before the scan finishes. Scroll while indexing; confirm the app remains usable. Record time to first results, first filled viewport, and completed scan. Timing improvements have not yet been benchmarked.
2. **Small folders:** Add folders containing 1, 24, 25, and 201 supported files. Confirm every file appears after completion, including the final partial batch. Repeat with nested folders and unrelated files.
3. **Warm rescan:** Rescan an unchanged folder. Confirm unchanged files are reported as unchanged and existing thumbnails remain visible. Modify one copied image and rescan; confirm its metadata and thumbnail refresh without refreshing unrelated previews.
4. **Interrupted batches:** Quit or force-quit during a large import, reopen, and rescan. Confirm all source files eventually appear once, previously committed results remain usable, and incomplete work is retried. Repeat near the beginning and later in the scan. WAL NORMAL preserves database consistency, but a power failure can lose recent commits; rescanning recovers derived indexing data.
5. **Unavailable entries:** On disposable data, make a subfolder unavailable during scanning. Confirm an error is surfaced and files are not incorrectly treated as removed merely because enumeration failed. Restore access and rescan.
6. **Orientation and faces:** Import JPEGs covering EXIF orientations 1–8, plus rotated HEICs. Compare grid and viewer orientation. Open existing face results and confirm boxes/crops still align, without a forced face reindex caused by the preview cache upgrade.
7. **Still formats:** Import JPEG, HEIC, AVIF, PNG, WebP, TIFF, and BMP examples supported by the machine. Confirm thumbnails render and retain aspect ratio. Inspect transparent PNG/WebP examples for preserved transparency. Confirm generated small previews have no edge larger than 512 pixels. Where available, use an image rejected by ImageIO but containing a valid embedded EXIF thumbnail to exercise fallback; mark this case unverified if no such fixture is available.
8. **Video metadata:** Import MOV and MP4 clips with known duration, dimensions, creation date, and location. Check displayed metadata and playback against the originals. Include a Live Photo still/video pair and ambiguous same-stem copies; verify pairing after scan completion remains correct.
9. **Cache migration:** Launch with an existing v3 preview cache. Confirm thumbnails regenerate in `thumbs-v4`, with no loss of people assignments or face results. Relaunch and revisit the same photos; confirm the new cache is reused. Old application thumbnail cache directories may be removed automatically.
10. **Placeholders:** On a fresh import, allow warming to complete, then scroll away and back. Confirm soft placeholders transition to correct thumbnails without blank/stale images. Change a copied source during preview generation and rescan; confirm a stale result is not saved for the new revision.
11. **Concurrency:** On a multicore Mac, scroll a cold library while observing Activity Monitor. Confirm preview work progresses without an unbounded increase in workers. The configured limit is `max(available_parallelism - 1, 1)`; the single-core minimum requires a suitable constrained environment to verify manually.
12. **Regression pass:** Restart, search, open media, scroll multiple folders, rename a copied file, and rescan. Confirm metadata, availability, pairing, and existing face assignments are retained.

## Build validation

Rust offline checking/building and the frontend production build pass. Svelte checking reports zero errors and one existing warning in `Sidebar.svelte`. These compilation checks do not establish runtime correctness or performance gains; complete the checklist above on representative local media.

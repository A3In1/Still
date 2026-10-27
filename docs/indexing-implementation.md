# Folder indexing and face recognition

Updated 2026-10-05. This describes implemented behavior; the longer design document also contains future work.

## Storage

There is no common photo root and no dependency on a source checkout in the installed app.

- Each directory containing supported media has a portable `.data/library.db`. It stores relative filenames, persistent folder/photo/person/face UUIDs, filesystem mtime in nanoseconds, byte size, metadata, face bounds, names, assignment provenance, rejected identities, and versioned float32 face embeddings.
- `Tauri app_cache_dir()/catalog.db` combines the registered folders for gallery and People queries. Its recognition-vector projection is also disposable. Reconciliation reconstructs it from folder records.
- `app_cache_dir()/thumbnails` and `face-crops` contain disposable previews, generated on demand. Keys include source stat, preprocessing version and face bounds where applicable.
- `app_data_dir()/settings.db` stores registered locations and durable pending name/review operations. These are not disposable cache.

The previous-app-index import button, command, automatic development import and import implementation have been removed. No previous database files are deleted.

## Reopening and incremental work

The cached gallery appears immediately. Registered directories reconcile in the background at startup, after coalesced filesystem events, and every minute as a fallback. Unchanged mtime/size/parser-version records skip metadata decoding. Mtime is separate from the photo's displayed capture date.

Face inference has its own per-photo `face_state`: input mtime/size revision, model version, pixel digest, completion/error state, attempt count and retry time. Successful zero-face results are saved too. Unchanged photos do not start the model. Changed files are decoded to compare oriented pixels; metadata-only changes with identical pixels skip model inference. Model changes deliberately invalidate compatible inference state.

Inference processes one image at a time outside database locks. Rust checks the source stat again before committing a photo's results atomically. Interrupted work remains due. Errors get at most three attempts with a ten-minute retry interval; **Find faces** resets failed attempts. **Pause** stops the subprocess and pauses automatic recognition for the current app session; reopening resumes it.

Missing photos are hidden only after successful directory enumeration. Offline roots do not trigger mass deletion. Moving an entire folder with its complete `.data` and selecting the new location retains its portable records. Individual cross-folder photo moves and edits that preserve both size and mtime are not fully reconciled yet.

## People and names

Naming a person writes the name locally in every indexed folder containing that identity, with a durable pending operation in app data. UUIDs identify people; equal names do not merge them.

Naming a provisional group preserves its membership as a user-named group. Individual **Yes** confirmations become trusted examples. This extra distinction prevents one incorrect face in a named cluster from automatically training future predictions.

Recognition uses normalized embeddings from confirmed faces of named identities: up to eight examples plus a mean vector per identity. Compatible named identities are considered first. A match requires cosine similarity of at least 0.65 and a margin of at least 0.08 over the runner-up. These are conservative initial policy values, not calibrated accuracy guarantees. Automatic assignments never become trusted training examples on their own.

Remaining unknowns go to bounded HDBSCAN batches of up to 2,048 candidates. Existing unnamed groups participate for continuity; noise/singletons get provisional identities and remain editable. HDBSCAN membership is not an identity-confidence score. Larger libraries are processed incrementally; this bounded grouping is not equivalent to global clustering and can split one person across groups.

**Review faces** supports confirming, rejecting, choosing another existing person, or assigning a new name. Rejections persist locally. Previously saved manual assignments are never cleared wholesale for reclustering. On changed pixels, old detections are matched one-to-one only with strong embedding agreement and spatial overlap. Uncertain manual correspondences retain their old records; new detections require review. A changed model cannot silently transfer manual labels using bounding boxes alone.

Face bounds and new previews use EXIF-oriented image coordinates.

## Runtime and build

Activate the requested environment:

```sh
source /opt/homebrew/anaconda3/etc/profile.d/conda.sh
conda activate gphotos
npm run build:desktop
```

The build helper installs missing pinned dependencies into the active Python environment and packages them with PyInstaller. It reuses available model files or downloads the official buffalo_l archive when needed, then writes a model checksum/version manifest. Use `python scripts/build_face_worker.py --models /path/to/models` to select another local location containing those exact model filenames.

Tauri includes the runtime under the app's resources. The installed app does not activate conda, execute a source script, or download models. Build the runtime on each target OS/architecture. The present generated runtime is macOS arm64, approximately 507 MB including models.

The NDJSON worker only reads images and returns detections/embeddings or clustering labels. Rust owns database writes, source-revision validation and user edits. CPU inference is capped to two intra-operation threads, images to 40 megapixels, returned faces to 128 per photo, and clustering to 2,048 vectors. Identity matching currently caps its reference set at 2,048 people. The subprocess exits when work finishes, releasing model memory. Catalog projection is batched every 32 processed photos and at folder boundaries.

The pretrained buffalo_l weights have separate licensing from the InsightFace code. The project's supplied weights are documented as non-commercial research models; redistribution/commercial use requires suitable rights. See the [official InsightFace licensing notes](https://github.com/deepinsight/insightface#license). Downloading or reusing model weights does not establish redistribution rights.

## Validation and remaining work

No test suite or test files were added, as requested. Validation uses Rust compilation, frontend checking/builds, temporary SQLite projection smoke checks, and the worker inside a built macOS app (one detected sample face with a 512-dimensional embedding, zero-face results, pixel-digest reuse and HDBSCAN). The debug macOS app bundle was built successfully; the GUI was not exercised end to end.

Recognition quality has not been measured on a labeled photo collection. HEIC/RAW decoding depends on the packaged decoder capabilities; unsupported images report processing failures. Large People lists still need pagination; reference selection, cluster recall, model startup and memory need profiling on representative libraries. Read-only-folder overlays, cross-machine conflict resolution, cache quotas and individual-photo move reconciliation remain future work. The folder databases are for serial local writers, not live multi-machine synchronization.

Move folders while the app is closed and preserve the complete `.data` directory. Copying an open SQLite database alone is not a safe backup.

## Preview responsiveness update — 2026-10-07

A read-only diagnosis found 83 gallery preview files for 1,284 catalog items, versus 1,399 face-crop files. Of 1,172 supported photo records, 698 had matching completed face state and 474 had never entered inference; no failed jobs were recorded. This was an incomplete initial recognition pass, not evidence of reprocessing those 698 completed photos.

Cache hits now read the versioned preview file without taking the image-generation mutex or reading/updating the catalog. Cache filenames retain their previous format, so existing previews remain usable. Cache misses still serialize decoding to bound memory; new files publish by atomic rename so readers cannot see half-written images. Gallery preview locations are derived from source stat rather than requiring a per-request database write.

People portraits, detail photos and review crops load through an IntersectionObserver with three concurrent requests and a 128-entry shared memory cache. Offscreen components release their image source. Hidden review images are no longer prefetched. Keyed person cards survive background refreshes, and unchanged folder projections no longer emit unnecessary People refresh events. Photo revision fields invalidate cached People previews when the source changes.

The image and JPEG decoder dependencies use optimized code even in development builds. The macOS debug app bundle and frontend checks passed; three existing accessibility warnings remain. No test files were added. Full GUI responsiveness with simultaneous recognition still needs observation in the running app; cache and compilation checks alone do not establish an end-to-end latency guarantee.

## Rich media implementation — 2026-10-09

The existing folder-authoritative indexing/projection design now carries media subtype, pairing, motion byte ranges, animation timing, color information, Live effect and key-frame edits. Parser version 2 reparses metadata without changing JPEG pixel-digest identity. Folder partner IDs are translated to catalog IDs after projection rather than copied verbatim.

`src/media/` owns native decoding, metadata/pairing, derivatives, frame extraction, exports, the bounded preview pool, cache cleanup, ranged embedded-video responses and optional performance spans. The macOS implementation links ImageIO/AVFoundation/Vision through `native.m`; it does not shell out to ffmpeg. It preserves the existing scanner and Tauri command boundaries. Frontend Live/Animated viewers and tile playback extend the existing lightbox/grid.

Face detections carry `frame_ms`/`frame_source` through the portable DB, catalog, People queries and preview cache. The worker reports decoder capabilities, samples bounded frame inputs, associates tracks and averages their best embeddings. Key changes invalidate inference while keeping manual records; ambiguous correspondence remains reviewable. HEIF inputs can be downscaled natively before inference, with coordinates mapped back to the original still. JPEG digest construction is unchanged.

Samsung SEFT bounds use the format layout documented by [AndroidX Media3's SefReader](https://github.com/androidx/media/blob/release/libraries/extractor/src/main/java/androidx/media3/extractor/mp4/SefReader.java); MotionPhoto payload and `mpv2` fields are described by [ExifTool's Samsung implementation](https://github.com/exiftool/exiftool/blob/master/lib/Image/ExifTool/Samsung.pm). The reader validates lengths and MP4 magic before exposing a range.

See [the manual checklist and remaining scope](rich-media-manual-checks.md). No automated tests were added. Build/type checks do not establish corpus accuracy or performance; no fabricated measurements are included.

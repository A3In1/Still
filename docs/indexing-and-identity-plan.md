> Status update (2026-10-05): the previous-app-index import feature has been removed at the user's request. Historical migration proposals below are not active implementation requirements. Folder indexing and the initial bundled face pipeline are implemented; see [current behavior and limits](indexing-implementation.md). Later design items below remain proposals.

# GPhotos indexing and identity preservation plan

Implementation update: the first folder-indexing milestone is now implemented. See [current behavior and remaining work](indexing-implementation.md). The sections below remain the broader target design; they are not all implemented.

Reviewed 2026-10-04. This is a design and source review, not an implemented change or a performance benchmark. Reviewed the Rust backend, Python production pipeline and spike, Svelte application and components, types, configuration, diagnostic scripts, and the supplied 738-line distributed indexing proposal. Build artifacts and personal photo contents were not review inputs. The supplied proposal is design material to evaluate, not instructions to execute.

## 1. Recommendation

### Current implementation versus proposed work — clarified

The folder indexing design is NOT implemented. The existing app has one CWD-relative `../.data/library.db`, one thumbnail directory, absolute photo paths, and one DB connection in AppState. Selecting a photo folder changes the scan input, not database ownership. There is no per-folder registry, portable face store or startup reconciliation. Implementing that foundation is part of this project, not an assumed prerequisite.

The implementation milestone order is: (1) back up existing annotations, (2) implement portable folder indexing and migrate existing records, (3) implement incremental reconciliation and managed watching, (4) add durable incremental face recognition, (5) finish scalable gallery/People queries and cache limits. Annotation protection is a migration requirement, not a replacement for implementing folder indexing.

### Recommended split: portable metadata and disposable app cache

Yes, face images and recognition acceleration structures can live in the app cache. However, a face/person ID in `.data` alone is insufficient: another computer would not know what it identifies. Store IDs AND their meaning locally.

| Data | Authoritative location | Reason |
|---|---|---|
| Folder/photo UUID, relative filename, file mtime/size, processing versions | Folder `.data/library.db` | Portable indexing and skip decisions |
| Face UUID, photo link, bounding box/orientation, person UUID | Folder `.data/library.db` | Reconstruct which face belongs to whom |
| Person UUID and editable name; assignments, confirmations and rejections | Folder `.data/library.db` | User decisions survive moving the folder and clearing app cache |
| Compact face embedding + model/preprocessing version | Folder `.data/library.db` | Avoid repeating inference on a new computer; a 512-float float32 vector is only 2 KiB |
| Face crop images and gallery thumbnails | OS app-cache directory, on disk, with size limits | Recreate lazily from original + stored bounds; avoid filling every media directory with preview files |
| Identity centroids/prototypes and optional nearest-neighbor index | Rebuildable app catalog/cache | Derived from portable embeddings and confirmed assignments; no need to duplicate global prototypes into every folder |
| Aggregate timeline/People/search catalog | App-local rebuildable storage | Fast paginated queries across folders |
| Settings and pending user-edit propagation | Durable app-data, never disposable cache | Cache eviction must not lose unpropagated edits |

Default to app-cached preview images. Optional portable preview export can be added later for immediate offline browsing on another machine. Clearing cache preserves names, bounds and embeddings; thumbnails regenerate on demand when originals are accessible. If an original is offline and its cache was removed, show a placeholder rather than claiming its preview can be recovered.

On a new computer, moving the complete folder with `.data` restores recorded people and compatible embeddings without face inference. Preview generation may still be necessary. Excluding embeddings would preserve names if the remaining metadata is complete, but would require new inference for recognition; that tradeoff conflicts with the user's request to avoid repeated work.

Separate four lifecycles: discovering filesystem changes, extracting media metadata, detecting faces, and assigning identities. Persist each result independently. A normal reopen checks files in the background; it must not decode unchanged photos, load the face model without pending work, or recluster all faces.

Use portable per-directory records for photos, face detections, and user decisions, plus a rebuildable machine-local catalog for fast cross-folder queries. Stable identity UUIDs and explicit assignment provenance preserve names. Centroids/prototypes are derived recognition aids, never the source of a person's identity.

Prioritize preserving existing names and corrections before changing storage or clustering. Do not run the current destructive face pipeline as part of migration.

## 2. Confirmed problems in the current source

References below are repository-relative file locations and line numbers at review time.

| Priority | Evidence | Consequence |
|---|---|---|
| Critical | `src-tauri/face_pipeline.py:16–80,142–168`: fetch every photo, detect every face, cluster everything, delete all faces/people, insert replacements | Repeated expensive inference; unstable IDs; no durable identity boundary. Named faces that become noise lose their assignment; conflicting names are reduced to a majority vote. |
| Critical | `src-tauri/src/people.rs:94–105,140–150`; pipeline preservation reads names/bboxes only | Rename is only a central person-row change. Explicit confirmation is reduced to confidence=1 and its provenance is discarded by the next scan. Renaming a generated `Person #…` label also confuses the name-prefix heuristic. |
| High | `src/lib/components/PeopleView.svelte:95–103` | “Not them” removes the item from the UI but makes no backend mutation. Rejection is forgotten. |
| High | `src-tauri/face_pipeline.py:67–73,166–168`; `src-tauri/src/db.rs:43–57` | Embeddings are duplicated in Python memory, including Python float lists, but never persisted. No face-processing status, zero-face success, model version, or resumable work. |
| High | `src/routes/+page.svelte:41–58`; `src-tauri/src/lib.rs:67–97` | Startup loads existing rows and starts a create-only watcher. It never reconciles offline edits/deletes. Watcher errors disappear; watcher lifecycle and folder switching are unmanaged. |
| High | `src-tauri/src/scanner.rs:232–260`; `src-tauri/src/lib.rs:48–63` | All paths then all parsed records are collected in memory; DB publication waits until the entire scan finishes. Existing records are not compared before EXIF/video work. |
| High | `src-tauri/src/scanner.rs:275–280`; `src-tauri/src/lib.rs:121–128,307–318` | Filesystem mtime is seconds-only; the API calls the display date “mtime”; editing the capture date writes both DB mtime and the actual filesystem modification time. Change tracking and presentation are conflated. |
| High | `src-tauri/src/db.rs:86–98` | Conflict update omits `size`, leaving stale sizes after edits. |
| High | `src-tauri/src/scanner.rs:31–47,65–81` | Thumbnail keys use paths and cache reuse checks only existence. Changed content at the same path keeps an obsolete thumbnail. Renames create unnecessary cache identities. |
| High | `src-tauri/src/lib.rs:29–40,383–390`; `src-tauri/src/people.rs:158–170` | DB, thumbnails, and Python script depend on process working directory. Selecting a folder does not isolate its database. Packaged execution is not reliably configured. |
| High | `src-tauri/src/people.rs:154–176`; `SettingsView.svelte:59–63` | Python spawn returns success immediately; child exit, failures, progress, cancellation and duplicate runs are not tracked. Another scan can start while face indexing continues. |
| High | `src/routes/+page.svelte:24`; `SettingsView.svelte:31`; `PhotoGrid.svelte:25,124` | Up to one million records requested, even to count photos; thumbnail cache has no eviction. Grid virtualization limits DOM nodes but not dataset/layout/cache memory. |
| Medium | `src-tauri/src/lib.rs:159–210,222–244` | Every search reads/filter/sorts the full library. Missing thumbnail paths can cause full originals to be sent as base64; a stale cached path fails rather than regenerating. |
| Medium | `src-tauri/src/people.rs:30,63–67,112`; `PeopleView.svelte:66–77` | Independent MIN aggregates can select a path/bbox/crop from different faces. Person detail loads all face rows and thumbnails. Review excludes unassigned/noise faces. |
| Medium | `src-tauri/src/db.rs`; `src-tauri/src/lib.rs:366–378` | No explicit foreign-key enablement, secondary query indexes, versioned migrations, atomic multi-statement delete, or persistent “remove from library” exclusion. Rescan can restore removed-only photos. |

Other relevant gaps: scan errors are discarded by `filter_map(Result::ok)`; extension lists do not prove installed image decoders support every format; Python attempts videos through `cv2.imread`; image orientation conventions are not shared between Rust and Python. Current small diagnostic scripts do not test identity survival, reconciliation, or migration.

## 3. Corrections to the proposed architecture

1. **A name is not an identity.** Two people may both be Alice; one person may be renamed. Use an immutable UUID, with mutable display name. Never merge on equal names.
2. **Naming a cluster does not validate every member.** Preserve its current memberships as `user_named_group`, distinct from `user_confirmed_face`. A wrongly clustered face must not become a trusted training example merely because its group was named.
3. **One mean vector is not sufficient in every case.** Profiles, age, lighting and bad detections can pull a mean away from good matches. Start with normalized means plus a bounded set of representative confirmed embeddings; validate usefulness on the actual library.
4. **No automatic name inheritance for a whole mixed cluster.** Conflicting known people require review/splitting. Even one known label is insufficient to bless every other face. Classify individual faces conservatively; uncertain faces remain unknown.
5. **Do not feed guesses back into trusted prototypes.** Build trusted prototypes from explicitly confirmed high-quality faces. With only a named group, require representative confirmation before enabling trusted automatic classification; group members still retain their named association.
6. **IoU is not an identity mechanism, but geometry remains useful.** Ordinary scans reuse stable face rows. Redetection after crop/rotation/model changes still needs correspondence evidence. Use geometry, compatible embeddings, orientation transforms and one-to-one matching; uncertain transfers retain old annotations for review instead of silently losing or misapplying them.
7. **mtime+size is a fast heuristic, not proof.** Copies can preserve timestamps, clocks and timestamp resolutions differ, and same-size edits can preserve mtime. Parent-directory mtime does not reliably expose edits to existing descendants. Offer deeper content verification; do not promise a universal 200 ms scan.
8. **Global pagination in the original proposal is incorrect.** Applying an offset within every child database and again after merging skips valid results. Query one indexed aggregate catalog instead of loading/sorting the union for each UI page.
9. **Copies preserve UUID and created_at.** The older-copy rule cannot distinguish them. Keep separate local location-instance IDs; do not rewrite a copied folder's portable UUID and trigger inference merely to resolve a path collision.
10. **Portable does not mean indestructible.** Deleting the only `.data` destroys its annotations without a backup. Moving only an image does not carry its folder DB. Simultaneous SQLite writes/sync from multiple computers need a separate supported protocol, not a “no conflicts” assumption.
11. **Storage options are tradeoffs.** A central SQLite catalog is not inherently too slow for 100k photos; sidecars can coexist with a catalog; hashes can be computed selectively. Per-folder databases buy subfolder portability at the cost of file handles, transactions, and recovery complexity. Avoid creating databases in every empty directory.

The claim that this is exactly Google Photos' internal implementation is not established by this repository or proposal and is not a premise of this design.

## 4. Storage ownership and concrete schema contract

Example:

```text
Pictures/Vacation/
  beach.jpg
  .data/
    library.db           durable local records, annotations and pending work

<Tauri app-data>/
  settings.json          selected roots and machine-specific configuration
  catalogs/<library-id>.db
                         aggregate gallery/person cache, source revisions,
                         location registry and durable propagation outbox

<Tauri app-cache>/
  previews/              replaceable photo thumbnails and face crops
  recognition/           optional derived vector-search acceleration files
```

Use Tauri's resolved application-data directory, never the process CWD. Catalog rows are rebuildable; the outbox is durable operational state and must survive a catalog rebuild. Store it separately if the whole catalog file is replaced. A root database containing portable direct-file records must never be deleted to rebuild the aggregate cache.

Create a folder DB when it has supported direct media, existing portable metadata, or annotations to preserve. Parent-only directories can be tracked in the catalog. Each DB owns only direct files. Resolve filename against its current registered directory and validate containment. Skip `.data` explicitly at traversal and watcher boundaries; default to not following symlinks. Do not exclude user media solely because an unrelated directory is hidden without a documented policy.

Proposed logical tables (implementation must add explicit types, constraints and migrations):

| Local table | Required fields / purpose |
|---|---|
| `folder_meta` | singleton `folder_uuid`, `schema_version`, `local_revision`; UUID authoritative here, no competing mutable JSON copy |
| `photos` | `photo_uuid` PK, unique `filename`, `size_bytes`, `fs_mtime_ns`, optional filesystem file ID hint, `source_revision`, `last_seen_generation`, availability/exclusion flags, metadata including separate `taken_at` and timezone/offset information where available |
| `processing_state` | `(photo_uuid, stage)` PK; `input_revision`, `pipeline_version`, `status`, attempt count, error, retry-after, completed-at; success includes zero faces |
| `local_faces` | `face_uuid` PK, `photo_uuid` FK, detection revision, orientation/coordinate contract, bbox, detection score, quality, crop key, retired flag |
| `face_embeddings` | `(face_uuid, model_version, preprocessing_version)` PK; float32 BLOB, dimension, normalization/encoding version |
| `local_people` | `person_uuid` PK, `display_name`, name revision, originating operation ID, status/alias info; display name is not unique |
| `face_assignments` | `face_uuid` PK, nullable `person_uuid`, provenance (`user_confirmed_face`, `user_named_group`, `automatic`, `unassigned`), decision revision/op ID, score and classifier version for automatic assignments |
| `face_rejections` | `(face_uuid, person_uuid)` PK, operation/revision; negative decisions persist |
| `identity_operations` | idempotent operations for rename, merge, split, assign, reject; enough payload to replay and detect conflicts |
| `jobs` | unique `(photo_uuid, source_revision, stage, pipeline_version)`, queued/running/succeeded/failed/unsupported, lease, retries and cancellation state |
| `change_log` | monotonic local revision plus changed IDs/tombstones for catalog ingestion; compact only after consumers can recover through a snapshot |
| `scan_runs` | generation, start/end, completion state, per-directory failures; generations are operational and resettable when an index is adopted on a new machine |

Catalog tables: root/library and location registrations; projected photos and face memberships; person summaries; name/merge conflicts; `known_identity_prototypes(person_uuid, model_version, preprocessing_version, prototype_id, vector, source_revision, sample_count)`; applied source revisions. Do not duplicate every embedding or base64 crop into gallery rows.

Global keys include location-instance ID plus portable photo/face UUID, because folder copies preserve UUIDs. Derived prototypes must deduplicate identical confirmed exemplars from copies. Portable person UUID stays shared across copies unless the user explicitly separates identities. Same UUID with divergent edits produces a conflict; equal names with different UUIDs stay separate.

Index local filename lookups, faces by photo, assignments by person, pending jobs by status/retry time. Index the catalog on `(library_id, sort_timestamp DESC, location_instance_id, photo_uuid)` and person membership. Use a deterministic tie-breaker and cursor pagination. Use SQL count queries. Add search indexes/FTS as appropriate; preserve existing location-radius semantics with candidate filtering followed by exact distance ranking. Validate with query plans.

Enable foreign keys on every connection, use short transactions and busy timeouts, version migrations explicitly, and use a bounded connection cache. Prefer Rust as the sole writer; Python receives jobs and streams results over a bounded protocol instead of independently rewriting databases. SQLite WAL can help readers coexist with a writer, but is not cross-machine synchronization.

## 5. Saving names, confirmations and corrections

When naming a group Alice:

1. Allocate/reuse its persistent person UUID. Rename that identity, preserving UUID and current memberships.
2. Persist an operation and targets before acknowledging the edit. Include the identity name snapshot in each affected folder's `local_people` and maintain each assignment there.
3. Apply changes in folder-local transactions with operation IDs for idempotent retry; increment the local revision/change log in the same transaction. A cross-folder edit is not one atomic SQLite transaction.
4. Track the propagation outbox as `saved`, `pending folders`, or `conflict`. Do not claim fully portable completion before all affected writable folders acknowledge. Persist edits for offline/read-only folders locally and show their pending portability status.
5. Project committed changes into the catalog. Rebuild only affected derived prototypes. Renaming alone does not require decoding images or recomputing vectors.

“Yes” creates explicit face confirmation. “Not them” records a rejection, clears that automatic assignment, and excludes the rejected identity from later automatic matching. Reassign/merge/split/unassign must be durable operations with undo history. User-confirmed assignments win over classifiers and cluster runs. Merge aliases and split decisions must travel with all affected folders; preserve unresolved offline changes in the outbox.

Concurrent operation conflicts use operation ancestry/revisions, not wall-clock last-writer-wins across machines. Detect incompatible names/assignments and surface resolution. V1 supports serial local ownership of portable DBs, not live multi-computer database sync.

When a subfolder arrives on a new computer, ingest its UUIDs, names, assignments, embeddings and versions first. Recreate known profiles from records; no embedding guessing is needed to recover recorded identities. Prototype caches may be rebuilt from those embeddings without decoding the original photos.

## 6. mtime and reopening: exact behavior

**Where stored:** `Vacation/.data/library.db → photos.fs_mtime_ns` and `photos.size_bytes`, one row per direct photo. These are the latest successfully observed file stat, not the capture date. Each processing stage separately stores its completed input revision, so advancing an observed timestamp cannot accidentally mark failed face work complete. The catalog mirrors stat values for display/queries but is not their portable authority. `scan_runs` records scan completion, not file modification time.

Read mtime from filesystem metadata and preserve the precision the filesystem actually exposes. Nanosecond storage cannot create nanosecond accuracy on coarse filesystems. Keep `taken_at`, filesystem mtime, and indexing timestamps separate; serialize ns values as strings or safely represented structured values across JavaScript boundaries, where large integers lose precision.

Startup sequence:

1. Resolve configured library and display the first cached page immediately. Offline roots show their status and cached contents without deletion.
2. Start one managed watcher per active root and wait for registration; buffer events while reconciling. Watcher failure triggers visible degraded status and reconciliation/polling fallback.
3. Discover directories and existing folder databases in the background. Compare DB-local revisions to catalog watermarks; if a change log is unavailable, reproject that folder from a bounded snapshot.
4. Enumerate/stat supported direct files in bounded batches. Lookup previous records by filename. New or changed `(mtime,size)` creates a new observed revision and idempotent jobs. Unchanged files skip completed compatible stages but still recover pending/failed work or missing caches.
5. Record seen entries using a disk-backed generation/staging table if needed, avoiding whole-library sets. On successful complete enumeration of a directory only, mark unseen entries unavailable. Permission failure, cancellation or unmounted disk never means every file was deleted.
6. Reconcile buffered events and drain/coalesce new watcher activity through the same job queue. On overflow or ambiguous events, mark directories dirty and repeat reconciliation. Periodic low-priority checks cover missed events while open.
7. Publish changed pages/counts in batches. Gallery readiness and face-indexing completion are distinct states.

Before processing an input, stat it; stat again before committing. If it changes mid-read, discard results and requeue. Debounce repeated events and partially copied files, but never treat elapsed debounce alone as proof that copying is complete. Use revision checks in commits to prevent stale worker output overwriting a newer file or decision.

| Change | Work |
|---|---|
| Nothing changed, all stages complete | Directory/stat checks; no EXIF read, image decode, inference or clustering |
| New photo | Metadata + thumbnail + one face detection job; classification; optionally unknown grouping |
| New video | Metadata/video thumbnail; video face extraction is explicitly unsupported in V1 |
| In-app name/review edit | Durable annotation transaction + catalog/prototype update; no source decode |
| In-app date/GPS-only edit | Save metadata, restat size/mtime, preserve visual revision if the writer is validated to leave decoded pixels/orientation unchanged |
| External changed file | Read/hash as needed, reparse metadata; if canonical decoded pixels/orientation match the prior visual fingerprint, reuse faces; otherwise redetect and conservatively reconcile annotations |
| Same-volume rename | Match trustworthy filesystem ID plus context, update filename/location; reuse detections |
| Cross-volume move or ambiguous replacement | Selective content hash against retained records; ambiguous matches need review; filename alone is insufficient |
| Folder moved with `.data` | Register new location, reuse portable rows; reconcile real changes only |
| Missing file after successful enumeration | Mark unavailable/tombstone; retain annotations for recovery/undo; garbage collect caches separately |
| Remove from library while keeping file | Persist exclusion so future scans do not immediately reimport it |
| New model or preprocessing version | Explicit staged migration of affected embeddings/detections; never compare incompatible vector spaces |

Maintain a stable photo UUID independently of any hash. A full content hash detects byte equality; an optional canonical visual fingerprint distinguishes metadata-only changes and requires a versioned decoding/orientation contract. A fingerprint must not be a lossy thumbnail hash used as proof that all faces are unchanged. Hash new/changed/ambiguous candidates, not every original on every reopen. Offer an explicit deep verification action to detect same-stat changes; document that the fast path can miss timestamp-preserving edits.

Ordinary **Check for changes** uses this same incremental reconciliation, including first-time libraries. Separate actions: **Repair thumbnails**, **Retry failed items**, **Rebuild catalog**, **Reprocess faces/model upgrade**, and **Verify file contents**. None may discard user decisions. Moving a complete indexed folder is not a reason for a full face rescan.

## 7. Incremental face pipeline

1. Select only due face jobs for supported still images. Record unsupported formats explicitly rather than retrying each launch. Standardize EXIF orientation, decoding, coordinate mapping and model preprocessing across thumbnails and inference.
2. Lazy-start one supervised model worker. Load only required detection/recognition models; validate providers and package dependencies/model assets. Capture progress, exit codes and actionable errors; pin model and preprocessing versions. Unload after a configurable idle interval.
3. Decode within pixel/memory limits; use reduced decoding where supported, and map detection coordinates correctly to the original oriented image. Cap pathological image sizes. Persist successful detections, compact float32 embeddings, face crop references and zero-face results atomically per photo/revision.
4. Preserve existing manual decisions. Classify only eligible unassigned faces against compatible trusted prototypes using normalized cosine similarity, a calibrated acceptance threshold, and a sufficient margin over the next-best identity. Rejections exclude candidates. Poor quality, insufficient exemplars or close competitors go to review.
5. Thresholds must be measured using confirmed positives and representative hard negatives, including similar-looking people, ages, pose and lighting. Do not copy a detector score, HDBSCAN probability, or arbitrary similarity constant into an identity probability.
6. Persist automatic predictions with provenance, scores and model/prototype revision; do not turn them into trusted training labels. Batch the face-by-prototype comparison matrix to bound memory. Add approximate search only after exact batched search becomes a measured bottleneck.
7. Cluster remaining unknowns as optional scheduled work, not as part of every reopen. Include suitable previously unknown embeddings so small imports can join earlier groups; preserve previous review groups and use membership overlap for provisional group continuity. Group labels are not person IDs.
8. For small unknown sets, run HDBSCAN under an explicit size/memory limit. For larger sets, defer or partition candidate neighborhoods and reconcile conservatively; this is an approximation with recall tradeoffs, not equivalent to full global HDBSCAN. Do not silently drop leftover/noise faces. Keep an Unknown view and allow manual assignment even for singletons.
9. Redetection writes a new generation before retiring old detections. Transfer annotations only with strong one-to-one correspondence; otherwise keep old labeled detections in unresolved review. A new model may require recomputing embeddings for confirmed reference crops before any classification; preserve old vectors until migration is verified.

HDBSCAN membership strength measures persistence within its clusters; it is not a calibrated probability of being Alice. Keep detector quality, cluster strength, identity score, and user confirmation as different fields. See [scikit-learn HDBSCAN reference](https://scikit-learn.org/stable/modules/generated/sklearn.cluster.HDBSCAN.html).

## 8. Performance and memory controls

Starting defaults below are tuning candidates, not measured guarantees:

- Stream traversal into a bounded queue (e.g. 256 records); commit 100–500 metadata changes at a time. Avoid collecting whole scans before insertion.
- Start with 1–2 image decode workers, one video subprocess, and one inference worker. A memory-budget semaphore controls concurrent decoded pixels; thread count alone cannot bound memory for huge images. Limit nested OpenCV/ONNX/Rayon parallelism.
- Give visible thumbnail requests priority over background work. Pause/cancel at job boundaries and resume from durable stage state. Coalesce duplicate manual/watcher/startup requests.
- Keep only a bounded number of folder DB connections open (e.g. 16), with bounded page caches. Benchmark total overhead on many tiny folders.
- Store JPEG/WebP crop files and compact vector BLOBs, not base64 in rows and JSON float arrays. A 512-dimensional float32 vector is 2,048 bytes: 100k vectors are about 195 MiB before indexes, Python objects and clustering workspace. Avoid loading them all merely to display people.
- Use versioned thumbnail keys `(photo_uuid, visual_revision, thumbnail_version)` and face crop keys; write temporary files then atomically publish. Evict orphan caches only after references are safely retired.
- Serve thumbnails through the asset protocol with correct MIME and scoped paths. Never use a full original as the gallery-thumbnail fallback; enqueue repair and show a placeholder.
- Page gallery and person results (e.g. 200 rows), use cursor tie-breakers, keep only a bounded window plus lightweight date/count summaries for navigation. Preserve grid virtualization and remove unbounded layout/data arrays.
- Use a byte-limited frontend cache (initial candidate 64 MiB), bounded requests and cancellation/request-generation checks on navigation/search/library switching. Remove image references from offscreen layout state on eviction too, otherwise an LRU alone cannot free memory.
- Debounce/batch catalog notifications, update affected rows, and use `COUNT(*)` or maintained transactional counts. Do not reload a million rows for every new photo.
- Select person cover photo, bbox and crop from the same deterministic face row; distinguish number of faces from number of distinct photos.
- Instrument stage timings, original bytes read, decodes, model loads, queue depth, WAL size, connections, Rust/Python/webview RSS and cancellation latency. Optimize measured bottlenecks.

## 9. Portability, failure and recovery

Folder moves retain portable identities only when `.data` is included and its records are committed. For app-managed moves, pause relevant jobs, finish local writes, close/checkpoint connections, perform the move and re-register watchers. Arbitrary live external copies cannot be guaranteed consistent. Provide a snapshot/export path using SQLite's backup API; do not copy only a live `.db` while ignoring its WAL. SQLite documents that WAL is part of database state and does not support clients on different hosts over a network filesystem: [SQLite WAL](https://www.sqlite.org/wal.html).

Read-only directories use a machine-local overlay and display “changes saved on this device; portable write pending.” Do not claim edits travel until exported/written successfully. Unsupported network/cloud live-write modes should use local working state and explicit snapshot import/export, not live multi-host SQLite writes.

Track copies through distinct local instances without re-inference. A moved single photo without its `.data` can preserve annotations when the app can reconcile it against retained records; a new machine cannot recover annotations that were never copied. Offer managed move/export if per-photo portability is required.

Keep deletions recoverable long enough to reconcile moves. A missing identity's last photo does not erase its manual name/history. Back up annotations separately from regenerable caches. Rebuilding the catalog must reimport local records and replay durable pending operations; losing the only local annotation DB without backup is unrecoverable.

## 10. Migration and implementation sequence

| Phase | Main files/modules | Deliverable and gate |
|---|---|---|
| 0: protect current data | `db.rs`, `people.rs`, `face_pipeline.py`, `PeopleView.svelte` | Snapshot current DB; disable destructive face replacement; stable IDs/provenance, durable rejection; ensure renamed/confirmed faces survive repeated jobs |
| 1: incremental job core | `scanner.rs`, new `jobs.rs`, `lib.rs`, `db.rs` | Separate observed stats/stage completion, bounded scan batches, revision-checked commits, managed watcher, resumable jobs; unchanged reopen causes zero inference |
| 2: portable schema and migration | new `folder_store.rs`, `migration.rs`, `catalog.rs` | Per-folder records + aggregate query catalog + propagation outbox; verified import preserves identities across a folder move |
| 3: durable face inference | `face_pipeline.py`, `people.rs`, new worker supervisor | Persist embeddings/zero-face status, process only due jobs, bounded IPC, errors/cancellation, compatible model versions |
| 4: identity classification | new identity service, pipeline, `PeopleView.svelte` | Confirmed exemplars/prototypes, conservative classification, rejection constraints, unknown review, safe staged redetection |
| 5: UI and query scaling | `+page.svelte`, `PhotoGrid.svelte`, `PeopleView.svelte`, `SettingsView.svelte`, `types.ts`, `InfoPanel.svelte` | Cursor pages/counts, bounded image cache, distinct dates, accurate progress; verify memory plateaus during long scrolling |
| 6: hardening | all above plus integration fixtures | Crash recovery, copies/offline/read-only libraries, migration rollback, performance profiling and threshold evaluation |

Migration details:

1. Locate the actually active legacy DB by configured/runtime resolution; do not choose a stray `src-tauri/library.db` or `data.db` just because it exists. Preview counts and roots; take a consistent backup.
2. Import legacy photo/person/face IDs using an explicit mapping journal to new UUIDs. Preserve all existing names, assignments and bboxes before inference. Do not merge people with equal names or treat every name as an explicitly confirmed face.
3. Legacy confirmation provenance cannot reliably be inferred from confidence=1, because model scores can also reach 1. Preserve the value as legacy provenance, requiring confirmation for trusted exemplars.
4. Existing schema contains no persisted embeddings, so one-time extraction is necessary to build prototypes. Keep legacy labeled face records while this happens. Where detection correspondence is uncertain, surface it rather than dropping labels.
5. Copy existing preview assets into versioned app-cache storage; keep the legacy assets until counts, relationships, names and representative image checks pass. Handle duplicate basenames with UUID keys. Portable local records refer to logical cache keys, never a machine's absolute cache paths.
6. Checkpoint each migrated folder in a resumable journal. Switch active catalog only after validation; keep source backup and rollback path. Unknown-format or unavailable originals must not block preservation of their annotations.

## 11. Acceptance tests and benchmarks

Use synthetic file trees, mock inference and fixture images; never run destructive verification on the user's live library.

| Test | Required result |
|---|---|
| Reopen unchanged indexed library repeatedly | Zero source decode, EXIF parse, model load or HDBSCAN calls once all requested stages are complete |
| Add 10 supported still photos | Only those photos enter inference; prior person/face UUIDs and user decisions unchanged |
| Photo with zero detected faces | Recorded success; no repeat inference on unchanged reopen |
| Rename, confirm, reject, then scan/restart | Names and both positive/negative decisions survive; rejected identity not immediately reassigned |
| Two people named Alice / cluster mixes Alice and Bob | No name-based merge; no majority-vote overwrite of confirmed labels |
| Named group contains a wrong face | Association remains correctable; it does not automatically contaminate trusted prototypes |
| Date/GPS-only in-app edit | Stat refreshed; capture date separate; face inference skipped when visual preservation is established |
| Change content at same path | Versioned thumbnail refreshed; changed visual content processed; stale results cannot commit |
| Same size and preserved mtime edit | Deep verification detects it; fast-path limitation documented |
| Move/rename folder with `.data` onto a new machine | Names and compatible detections imported; no inference required solely because paths changed |
| Copy same folder twice | Both locations represented without UUID corruption or duplicate prototype weighting |
| Offline disk / denied directory / interrupted enumeration | No mass deletion; availability/degraded state recorded |
| Crash during job/local rename propagation/catalog update | Retry resumes idempotently; pending portability visible; no loss of acknowledged operation |
| Watcher burst, overflow, rename, root switch | Bounded/coalesced work, reconciliation fallback, old watcher stopped, no writes to wrong library |
| Failed decode, unsupported format, unavailable model | Distinct states and actionable error; bounded retry; no false completion |
| Delete from library only, reopen | Exclusion survives; file stays on disk and is not reimported |
| Redetect rotated/cropped photo or upgrade model | Old annotations remain until confidently reconciled or explicitly reviewed |
| Migrate fixture legacy DB, interrupt, resume, rollback | Counts/names/relationships preserved; repeated migration does not duplicate records |
| Browse/search 10k and 100k records; many tiny folders | First page does not wait for full traversal; cache/connection/queue limits hold; scrolling memory plateaus |

Benchmark cold and warm startup separately on SSD and external storage. Record time to first page, reconciliation duration, changed-file throughput, thumbnail latency, classification quality, peak combined process RSS and disk writes. Start with proposed resource caps, measure, then set regression budgets. Do not promise numerical startup/inference targets before measuring the actual machine, codecs and model runtime.

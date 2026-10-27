"""Bounded NDJSON inference worker. No database writes and no implicit downloads."""
import os
os.environ.setdefault('OMP_NUM_THREADS', '2')
os.environ.setdefault('OPENBLAS_NUM_THREADS', '2')
import sys
import json
import hashlib
from pathlib import Path
from contextlib import redirect_stdout

PREPROCESS = 'oriented-rgb-max2048-v1'
MAX_PIXELS = 40_000_000
MAX_CLUSTER = 2048


def serve(model_dir):
    import numpy as np
    from PIL import Image, ImageOps
    import pillow_heif
    pillow_heif.register_heif_opener()
    import cv2
    cv2.setNumThreads(2)
    Image.MAX_IMAGE_PIXELS = MAX_PIXELS
    manifest = json.loads((model_dir / 'manifest.json').read_text())
    # Check bundled files, never silently mix model spaces or download replacement weights.
    for filename, expected in manifest['sha256'].items():
        if hashlib.file_digest((model_dir / filename).open('rb'), 'sha256').hexdigest() != expected:
            raise RuntimeError('Face model checksum mismatch: ' + filename)
    model = None
    # Hardware (CoreML) vectors live under a distinct model version, so Rust never mixes them with CPU ones.
    coreml = os.environ.get('GPHOTOS_FACE_COREML') == '1'
    if coreml:
        import onnxruntime as ort
        if 'CoreMLExecutionProvider' not in ort.get_available_providers():
            raise RuntimeError('CoreML unavailable')
    emit({'ready': True, 'model': manifest['version'] + ('-coreml' if coreml else ''), 'provider': 'coreml' if coreml else 'cpu',
          'capabilities': {'heif': True, 'animated': True, 'video': True, 'raw': True}, 'decoder_version': 3})
    for line in sys.stdin:
        try:
            if len(line) > 32_000_000:
                raise ValueError('Worker request too large')
            req = json.loads(line)
            if req['op'] == 'detect':
                sources = req.get('frames') or [{'path': req['path'], 'frame_ms': None, 'frame_source': 'still'}]
                if len(sources) > 8:
                    raise ValueError('Frame sampling exceeds eight frames')
                prepared = []
                raw_digests = []
                hasher = None
                for source in sources:
                    if 'raw' in source:
                        # Rust already decoded, oriented and downscaled this frame and hashed it with xxh3.
                        w, h = int(source['width']), int(source['height'])
                        if w * h > MAX_PIXELS or os.path.getsize(source['raw']) != w * h * 3:
                            raise ValueError('Raw frame has an invalid size')
                        pixels = np.fromfile(source['raw'], dtype=np.uint8).reshape(h, w, 3)
                        raw_digests.append(source['digest'])
                        prepared.append((cv2.cvtColor(pixels, cv2.COLOR_RGB2BGR), 1.0, source))
                        continue
                    with Image.open(source['path']) as original:
                        if original.width * original.height > MAX_PIXELS:
                            raise ValueError('Image exceeds the 40 megapixel face-processing limit')
                        oriented = ImageOps.exif_transpose(original).convert('RGB')
                        pixels = np.asarray(oriented)
                        if hasher is None:
                            hasher = hashlib.sha256(str(pixels.shape).encode())
                        else:
                            hasher.update(str(pixels.shape).encode())
                        hasher.update(memoryview(pixels))
                        if source.get('frame_ms') is not None:
                            hasher.update(str((source['frame_ms'], source['frame_source'])).encode())
                        w, h = oriented.size
                        scale = min(1.0, 2048 / max(w, h))
                        if scale < 1:
                            oriented = oriented.resize((round(w * scale), round(h * scale)))
                        prepared.append((cv2.cvtColor(np.asarray(oriented), cv2.COLOR_RGB2BGR), scale, source))
                if hasher is None:
                    digest = raw_digests[0] if len(raw_digests) == 1 else hashlib.sha256(''.join(raw_digests).encode()).hexdigest()
                else:
                    for item in raw_digests:
                        hasher.update(item.encode())
                    digest = hasher.hexdigest()
                if digest == req.get('pixel_digest'):
                    emit({'unchanged': True, 'digest': digest, 'faces': []})
                    continue
                if model is None:
                    import onnxruntime as ort
                    ort.disable_telemetry_events()
                    with redirect_stdout(sys.stderr):
                        from insightface.app import FaceAnalysis
                    options = ort.SessionOptions()
                    options.intra_op_num_threads = 2
                    options.inter_op_num_threads = 1
                    providers = ['CPUExecutionProvider']
                    if coreml:
                        providers = [('CoreMLExecutionProvider', {'ModelCacheDirectory': os.environ.get('GPHOTOS_FACE_CACHE', '')}), 'CPUExecutionProvider']
                    with redirect_stdout(sys.stderr):
                        model = FaceAnalysis(name=str(model_dir), allowed_modules=['detection', 'recognition'],
                                             providers=providers, sess_options=options)
                        model.prepare(ctx_id=-1, det_size=(640, 640), det_thresh=0.65)
                    # ONNX Runtime silently falls back to CPU; refuse to label those vectors as hardware ones.
                    if coreml and any(m.session.get_providers()[0] != 'CoreMLExecutionProvider' for m in model.models.values()):
                        model = None
                        raise RuntimeError('CoreML unavailable')
                tracks = []
                for bgr, scale, source in prepared:
                    with redirect_stdout(sys.stderr):
                        detections = model.get(bgr)
                    used = set()
                    for face in detections[:128]:
                        vector = np.asarray(face.normed_embedding, dtype=np.float32)
                        if vector.shape != (512,) or not np.isfinite(vector).all():
                            continue
                        bbox = (face.bbox / scale).tolist()
                        coordinate_width = source.get('coordinate_width')
                        coordinate_height = source.get('coordinate_height')
                        if coordinate_width and coordinate_height:
                            sx = coordinate_width / (bgr.shape[1] / scale)
                            sy = coordinate_height / (bgr.shape[0] / scale)
                            bbox = [bbox[0]*sx, bbox[1]*sy, bbox[2]*sx, bbox[3]*sy]
                        area = max(0, bbox[2]-bbox[0]) * max(0, bbox[3]-bbox[1])
                        points = face.kps
                        frontal = 1.0
                        if points is not None:
                            eye_mid = (points[0] + points[1]) / 2
                            frontal = 1 / (1 + abs(float(points[2][0]-eye_mid[0])) / max(1.0, float(np.linalg.norm(points[0]-points[1]))))
                        quality = float(face.det_score) * area * frontal
                        record = {'bbox': bbox, 'score': float(face.det_score), 'embedding': vector.tolist(),
                                  'frame_ms': source.get('frame_ms'), 'frame_source': source.get('frame_source', 'still'),
                                  'quality': quality if np.isfinite(quality) else None,
                                  'blur': sharpness(cv2, bgr, face.bbox),
                                  'face_px': max(0.0, min(bbox[2]-bbox[0], bbox[3]-bbox[1]))}
                        candidates = []
                        for i, track in enumerate(tracks):
                            if i in used:
                                continue
                            cosine = float(np.dot(vector, track['vector']))
                            a = track['face']['bbox']
                            intersection = max(0, min(a[2],bbox[2])-max(a[0],bbox[0])) * max(0, min(a[3],bbox[3])-max(a[1],bbox[1]))
                            union = (a[2]-a[0])*(a[3]-a[1]) + area - intersection
                            overlap = intersection / max(union, 1)
                            if cosine >= .70 or cosine >= .55 and overlap >= .3:
                                candidates.append((cosine, i))
                        if candidates:
                            _, i = max(candidates)
                            track = tracks[i]
                            used.add(i)
                            track['samples'].append((quality, vector))
                            track['samples'].sort(key=lambda x: x[0], reverse=True)
                            track['samples'] = track['samples'][:3]
                            mean = np.mean([v for _, v in track['samples']], axis=0)
                            track['vector'] = mean / max(np.linalg.norm(mean), 1e-8)
                            # Default still faces retain their crop when deep motion matches them.
                            if quality > track['quality'] and track['face']['frame_source'] != 'still':
                                track['face'], track['quality'] = record, quality
                        elif len(tracks) < 128:
                            used.add(len(tracks))
                            tracks.append({'face': record, 'quality': quality, 'vector': vector, 'samples': [(quality, vector)]})
                result = []
                for track in tracks:
                    track['face']['embedding'] = track['vector'].tolist()
                    result.append(track['face'])
                emit({'unchanged': False, 'digest': digest, 'faces': result})
            elif req['op'] == 'cluster':
                vectors = np.asarray(req['vectors'], dtype=np.float32)
                if len(vectors) > MAX_CLUSTER:
                    raise ValueError('Clustering batch exceeds limit')
                if len(vectors) < 2:
                    emit({'labels': [-1] * len(vectors)})
                    continue
                from sklearn.cluster import HDBSCAN
                labels = HDBSCAN(min_cluster_size=2, min_samples=2, metric='euclidean',
                                 n_jobs=1, copy=True).fit_predict(vectors)
                emit({'labels': labels.tolist()})
            else:
                raise ValueError('Unknown operation')
        except Exception as exc:
            emit({'error': str(exc)})


def sharpness(cv2, bgr, box):
    """Variance of the Laplacian on the grayscale face crop; None when the crop is empty."""
    h, w = bgr.shape[:2]
    x1, y1 = max(0, int(box[0])), max(0, int(box[1]))
    x2, y2 = min(w, int(round(box[2]))), min(h, int(round(box[3])))
    if x2 - x1 < 4 or y2 - y1 < 4:
        return None
    gray = cv2.cvtColor(bgr[y1:y2, x1:x2], cv2.COLOR_BGR2GRAY)
    # Compare faces at a common scale; large crops otherwise look artificially smooth.
    side = min(gray.shape)
    if side > 160:
        gray = cv2.resize(gray, (round(gray.shape[1] * 160 / side), round(gray.shape[0] * 160 / side)),
                          interpolation=cv2.INTER_AREA)
    value = float(cv2.Laplacian(gray, cv2.CV_64F).var())
    return value if value == value and value != float('inf') else None


def emit(value):
    print(json.dumps(value, allow_nan=False, separators=(',', ':')), flush=True)


if __name__ == '__main__':
    try:
        serve(Path(sys.argv[1]))
    except Exception as exc:
        emit({'error': str(exc)})
        sys.exit(1)

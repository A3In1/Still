# Third-Party Notices

Still's own source code is not yet under an open-source license (all rights reserved). The components below keep their own licenses.

## Bundled face-detection runtime

The face worker is packaged with PyInstaller (see `scripts/build_face_worker.py`,
`requirements-face.txt`).

| Component | Version | License | Link |
|---|---|---|---|
| insightface (code) | 2.0 | MIT | https://github.com/deepinsight/insightface |
| onnxruntime | 1.30.0 | MIT | https://github.com/microsoft/onnxruntime/blob/main/LICENSE |
| opencv-python (packaging scripts) | 5.0.0.93 | MIT | https://github.com/opencv/opencv-python/blob/4.x/LICENSE.txt |
| OpenCV (bundled in opencv-python) | 5.0 | Apache-2.0 | https://github.com/opencv/opencv/blob/4.x/LICENSE |
| FFmpeg (bundled in opencv-python wheels) | — | LGPL-2.1 | https://github.com/opencv/opencv-python#licensing |
| numpy | 2.4.6 | BSD-3-Clause | https://github.com/numpy/numpy/blob/main/LICENSE.txt |
| scikit-learn | 1.9.1 | BSD-3-Clause | https://github.com/scikit-learn/scikit-learn/blob/main/COPYING |
| Pillow | 12.3.0 | MIT-CMU (HPND) | https://github.com/python-pillow/Pillow/blob/main/LICENSE |
| pillow-heif | 1.1.1 | BSD-3-Clause | https://github.com/bigcat88/pillow_heif/blob/master/LICENSE.txt |
| libheif, libde265 (bundled in pillow-heif) | — | LGPL-3.0 | https://github.com/bigcat88/pillow_heif/blob/master/LICENSES_bundled.txt |
| x265 (bundled in pillow-heif and opencv-python) | — | GPL-2.0 | https://github.com/bigcat88/pillow_heif/blob/master/LICENSES_bundled.txt |
| PyInstaller bootloader | 6.16.0 | GPL-2.0 with bootloader exception | https://github.com/pyinstaller/pyinstaller/blob/develop/COPYING.txt |
| Python runtime | 3.11–3.13 | PSF-2.0 | https://docs.python.org/3/license.html |

## InsightFace pretrained models (buffalo_l) — NOT MIT

The bundled models `det_10g.onnx` and `w600k_r50.onnx` come from the InsightFace
`buffalo_l` model pack and are **not** covered by Still's MIT license or by the
InsightFace code's MIT license. Per the
[InsightFace README](https://github.com/deepinsight/insightface#license) (checked 2026-10-27):

> The training data containing the annotation (and the models trained with these data) are
> available for non-commercial research purposes only.

> Both manual-downloading models from our github repo and auto-downloading models with our
> python-library follow the above license policy(which is for non-commercial research purposes only).

> For open-sourced face recognition models (e.g., buffalo_l package), please contact
> recognition-oss-pack@insightface.ai for licensing.

Commercial use of these models requires a separate license from InsightFace.

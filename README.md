# GPhotos

## Run from source on another computer

Install Node.js, Rust and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). Use Python 3.11–3.13; Python 3.11 is recommended. Activate your Python/conda environment first if you use one.

From this project's folder:

```sh
npm install
python scripts/build_face_worker.py
npm run tauri dev
```

The Python script installs missing or mismatched packages from `requirements-face.txt` into the active Python environment, downloads missing face models, and packages the worker for the current computer. Initial setup needs internet and can take several minutes. Downloaded models are reused on subsequent runs. No username-specific path or preinstalled InsightFace model folder is required.

Existing models in `~/.insightface/models/buffalo_l` are reused if available. You can optionally supply another existing folder with `--models /path/to/models`.

## Share the installed app

Run `npm run build:desktop` to build an application package. The package includes the face runtime and models, so the recipient does not need Node, Python, conda, Rust or a source checkout. Build for the recipient's OS and CPU architecture; distributing signed/notarized releases is a separate step.

The source checkout excludes generated model/runtime files intentionally. They are recreated by the setup script. The packaged app does not download models at runtime.

The pretrained models have separate licensing: review the [InsightFace license notes](https://github.com/deepinsight/insightface#license) before redistribution or commercial use.

See [indexing implementation and limits](docs/indexing-implementation.md) for storage and recognition behavior.

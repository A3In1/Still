"""Install worker dependencies, fetch missing models, and bundle the face worker."""
import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
MODEL_NAMES = ("det_10g.onnx", "w600k_r50.onnx")
MODEL_URL = "https://github.com/deepinsight/insightface/releases/download/model-zoo/buffalo_l.zip"


def prepare_models(models, source):
    models.mkdir(parents=True, exist_ok=True)
    for name in MODEL_NAMES:
        target = models / name
        if not target.is_file() and (source / name).is_file():
            shutil.copy2(source / name, target)

    missing = [name for name in MODEL_NAMES if not (models / name).is_file()]
    if missing:
        print("Downloading face models (first setup only)…", flush=True)
        # Extract only the two required files; publish completed files atomically.
        with tempfile.TemporaryFile(dir=models.parent) as archive:
            with urllib.request.urlopen(MODEL_URL, timeout=60) as response:
                shutil.copyfileobj(response, archive)
            archive.seek(0)
            with zipfile.ZipFile(archive) as bundle:
                for name in missing:
                    member = next(item for item in bundle.infolist()
                                  if Path(item.filename).name == name and not item.is_dir())
                    temporary = models / (name + ".pending")
                    with bundle.open(member) as src, temporary.open("wb") as dst:
                        shutil.copyfileobj(src, dst)
                    temporary.replace(models / name)

    hashes = {}
    for name in MODEL_NAMES:
        with (models / name).open("rb") as model:
            hashes[name] = hashlib.file_digest(model, "sha256").hexdigest()
    version = "buffalo-l-" + hashlib.sha256(
        json.dumps(hashes, sort_keys=True).encode()
    ).hexdigest()[:16] + "-oriented-rgb-max2048-v1"
    (models / "manifest.json").write_text(
        json.dumps({"version": version, "sha256": hashes}, indent=2)
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--models", type=Path,
                        default=Path.home() / ".insightface/models/buffalo_l",
                        help="Optional existing models to reuse; missing models download automatically.")
    args = parser.parse_args()
    if not (3, 11) <= sys.version_info[:2] <= (3, 13):
        raise RuntimeError("Use Python 3.11–3.13 for the pinned worker dependencies (3.11 recommended).")

    requirements = ROOT / "requirements-face.txt"
    for line in requirements.read_text().splitlines():
        name, version = line.strip().split("==")
        try:
            installed = importlib.metadata.version(name)
        except importlib.metadata.PackageNotFoundError:
            installed = None
        if installed != version:
            print("Installing face-worker packages into the active Python environment…", flush=True)
            subprocess.run([sys.executable, "-m", "pip", "install", "-r", str(requirements)], check=True)
            break

    runtime = ROOT / "src-tauri/resources/face-runtime"
    prepare_models(runtime / "models", args.models.expanduser())
    work = ROOT / ".face-build"
    work.mkdir(exist_ok=True)
    print("Packaging the face worker…", flush=True)
    subprocess.run([
        sys.executable, "-m", "PyInstaller", "--noconfirm", "--onedir",
        "--name", "gphotos-face", "--distpath", str(runtime),
        "--workpath", str(work / "work"), "--specpath", str(work),
        "--collect-all", "pillow_heif", "--collect-all", "insightface", "--collect-all", "onnxruntime",
        "--collect-submodules", "sklearn.cluster._hdbscan",
        str(ROOT / "src-tauri/face_pipeline.py"),
    ], env=dict(os.environ, PYINSTALLER_CONFIG_DIR=str(work / "cache")), check=True)
    print("Ready. Run: npm run tauri dev")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, StopIteration, zipfile.BadZipFile,
            subprocess.CalledProcessError) as error:
        sys.exit(f"Face worker setup failed: {error}\nFix the issue and rerun; completed model downloads are reused.")

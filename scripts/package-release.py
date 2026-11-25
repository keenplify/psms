"""Package all native CI artifacts, preserving POSIX executable permissions."""
import hashlib
import os
from pathlib import Path
import tarfile
import tomllib
import zipfile

version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
if os.environ["GITHUB_REF_NAME"] != "v" + version:
    raise SystemExit("Tag must match Cargo.toml")
out = Path("dist")
out.mkdir(exist_ok=True)
targets = {"x86_64-pc-windows-msvc": "windows-x64", "x86_64-unknown-linux-gnu": "linux-x64",
           "aarch64-unknown-linux-gnu": "linux-arm64", "x86_64-apple-darwin": "macos-intel",
           "aarch64-apple-darwin": "macos-arm64"}
checksums = []
for target, label in targets.items():
    windows = label.startswith("windows")
    folder = Path("binaries") / ("papamo-sms-" + target)
    ext = ".exe" if windows else ""
    files = [folder / (name + ext) for name in ("psms", "psms-desktop")]
    docs = list(map(Path, ["README.md", "SYNC.md", "LICENSE"]))
    archive = out / ("psms-" + label + (".zip" if windows else ".tar.gz"))
    if windows:
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as z:
            for file in files + docs:
                z.write(file, file.name)
    else:
        with tarfile.open(archive, "w:gz") as tar:
            for file in files + docs:
                info = tar.gettarinfo(str(file), arcname=file.name)
                info.mode = 0o755 if file in files else 0o644
                with file.open("rb") as stream:
                    tar.addfile(info, stream)
    with archive.open("rb") as stream:
        checksums.append(hashlib.file_digest(stream, "sha256").hexdigest() + "  " + archive.name)
(out / "SHA256SUMS").write_text("\n".join(checksums) + "\n")

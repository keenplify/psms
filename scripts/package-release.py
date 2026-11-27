"""Package all native CI artifacts, preserving POSIX executable permissions."""
import json
import hashlib
import subprocess
import tempfile
import os
import shutil
from pathlib import Path
import tarfile
import tomllib
import zipfile

version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
if (os.environ.get("RELEASE_TAG") or os.environ["GITHUB_REF_NAME"]) != "v" + version:
    raise SystemExit("Tag must match Cargo.toml")
out = Path("dist")
out.mkdir(exist_ok=True)
targets = {"x86_64-pc-windows-msvc": "windows-x64", "x86_64-unknown-linux-gnu": "linux-x64",
           "aarch64-unknown-linux-gnu": "linux-arm64", "aarch64-apple-darwin": "macos-arm64"}
checksums = []
for target, label in targets.items():
    windows = label.startswith("windows")
    folder = Path("binaries") / ("papamo-sms-" + target)
    ext = ".exe" if windows else ""
    files = [folder / (name + ext) for name in ("psms", "psms-desktop")]
    docs = list(map(Path, ["README.md", "SYNC.md", "LICENSE"]))
    for name in ("psms", "psms-desktop"):
        shutil.copyfile(folder / (name + ext), out / (name + "-" + label + ext))
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
    installers = []
    if windows:
        installers = [f"psms-{label}-setup.exe"]
    elif label.startswith("linux-"):
        installers = [f"psms-{label}.deb", f"psms-{label}.rpm"]
    elif label.startswith("macos-"):
        installers = [f"psms-{label}.dmg"]
    for name in installers:
        shutil.copyfile(folder / name, out / name)
for browser in ("chrome","firefox"):
    source = Path("extension")
    manifest = json.loads((source/"manifest.json").read_text())
    if browser == "firefox":
        manifest["background"] = {"scripts":["match.js","background.js"]}
        manifest["browser_specific_settings"] = {"gecko":{"id":"autofill@papamo.dev","strict_min_version":"128.0"}}
    with zipfile.ZipFile(out/f"psms-{browser}.zip","w",zipfile.ZIP_DEFLATED) as z:
        z.writestr("manifest.json", json.dumps(manifest,indent=2))
        for file in sorted(source.iterdir()):
            if file.name != "manifest.json":
                z.write(file,file.name)
# Include the authoritative schema with its compatible server build.
shutil.copyfile("hosted/target/release/psms-hosted", out/"psms-hosted-linux-x64")
shutil.copyfile("hosted/schema.sql",out/"psms-hosted-schema.sql")
for file in sorted(out.iterdir()):
    with file.open("rb") as stream:
        checksums.append(hashlib.file_digest(stream, "sha256").hexdigest() + "  " + file.name)
(out / "SHA256SUMS").write_text("\n".join(checksums) + "\n")
# Sign the exact version and every downloadable digest as one release.
manifest = {"version":version,"files":dict((line[66:],line[:64]) for line in checksums)}
path = out / "update.json"
path.write_bytes(json.dumps(manifest, sort_keys=True, separators=(",",":")).encode())
with tempfile.TemporaryDirectory() as temp:
    key = Path(temp) / "key.pem"
    key.write_text(os.environ.pop("PSMS_RELEASE_SIGNING_KEY").strip() + "\n")
    subprocess.run(["openssl","pkeyutl","-sign","-rawin","-inkey",str(key),"-in",str(path),"-out",str(out/"update.sig")],check=True)
subprocess.run(["openssl","pkeyutl","-verify","-rawin","-pubin","-inkey","packaging/update-public.pem","-in",str(path),"-sigfile",str(out/"update.sig")],check=True)

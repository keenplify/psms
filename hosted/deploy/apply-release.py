#!/usr/bin/python3
"""Privileged fixed command for release-only SSH keys; reads one stable tag from stdin."""
import fcntl
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

BASE = "https://github.com/keenplify/psms/releases/download"
STATE = Path("/var/lib/psms-release")
BIN = Path("/usr/local/bin")

def run(*args):
    subprocess.run(args, check=True, timeout=120)

def version(tag):
    if not re.fullmatch(r"v(0|[1-9][0-9]{0,8})(\.(0|[1-9][0-9]{0,8})){2}", tag):
        raise ValueError("Expected a stable release tag")
    return tuple(map(int, tag[1:].split(".")))

def download(stage, tag, name, size):
    path = stage / name
    run("curl","--fail","--silent","--show-error","--location","--proto","=https",
        "--proto-redir","=https","--max-time","100","--max-filesize",str(size),
        "--output",str(path), f"{BASE}/{tag}/{name}")
    return path

def health():
    for port, path in [(47832,"/readyz"),(47833,"/free/v1/readyz")]:
        run("curl","-fsS","--retry","5","--retry-all-errors","--max-time","5",
            f"http://127.0.0.1:{port}{path}")

def main():
    os.umask(0o077)
    tag = sys.stdin.readline(64).strip()
    ver = version(tag)
    STATE.mkdir(mode=0o700,exist_ok=True)
    with (STATE/"lock").open("a") as lock, tempfile.TemporaryDirectory(dir=STATE) as directory:
        fcntl.flock(lock,fcntl.LOCK_EX)
        marker=STATE/"version"
        if marker.exists():
            installed=version(marker.read_text().strip())
            if ver<installed:
                raise ValueError("Refusing release downgrade")
            if ver==installed:
                health()
                return
        stage=Path(directory)
        manifest=download(stage,tag,"update.json",65536)
        signature=download(stage,tag,"update.sig",64)
        run("openssl","pkeyutl","-verify","-rawin","-pubin","-inkey",
            "/etc/psms/update-public.pem","-in",str(manifest),"-sigfile",str(signature))
        metadata=json.loads(manifest.read_bytes())
        if metadata["version"]!=tag[1:]:
            raise ValueError("Signed release version mismatch")
        for name in ["psms-linux-x64","psms-hosted-linux-x64","psms-hosted-schema.sql"]:
            path=download(stage,tag,name,300*1024*1024)
            with path.open("rb") as stream:
                if hashlib.file_digest(stream,"sha256").hexdigest()!=metadata["files"][name]:
                    raise ValueError("Asset digest mismatch")
        region=Path("/etc/psms/rollout-region").read_text().strip()
        if region not in ("na","sg","ws","sa"):
            raise ValueError("Unconfigured rollout region")
        if region=="na":
            with (stage/"psms-hosted-schema.sql").open("rb") as sql:
                subprocess.run(["runuser","-u","psms_hosted","--","psql","-Xq","-v",
                                "ON_ERROR_STOP=1","-d","psms_accounts"],stdin=sql,check=True,timeout=120)
        names=["psms","psms-hosted"]
        for name in names:
            shutil.copy2(BIN/name,stage/(name+".old"))
        try:
            for name in names:
                run("install","-m755",str(stage/(name+"-linux-x64")),str(BIN/(name+".next")))
                os.replace(BIN/(name+".next"),BIN/name)
            run("systemctl","restart",*names)
            health()
        except BaseException:
            for name in names:
                run("install","-m755",str(stage/(name+".old")),str(BIN/(name+".rollback")))
                os.replace(BIN/(name+".rollback"),BIN/name)
            run("systemctl","restart",*names)
            raise
        (stage/"version").write_text(tag+"\n")
        os.replace(stage/"version",marker)
        print(region,tag,"ready")

if __name__=="__main__":
    main()

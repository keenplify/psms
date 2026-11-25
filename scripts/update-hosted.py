"""Roll out a prebuilt Linux binary to the provisioned pSMS hosts."""
import argparse
import hashlib
from pathlib import Path
import re

import subprocess

ROOT = Path(__file__).resolve().parents[1]
REGIONS = ("na", "sg", "ws", "sa")

def ssh(host, command, data=None):
    return subprocess.run(["ssh", "-o", "BatchMode=yes", host, command], input=data,
                          capture_output=False, check=True)

def deploy(host, binary, digest, publish):
    stage = subprocess.check_output(["ssh", host, "mktemp -d /var/tmp/psms-release.XXXXXXXX"], text=True).strip()
    if not re.fullmatch(r"/var/tmp/psms-release\.[A-Za-z0-9]{8}", stage):
        raise ValueError("unexpected staging directory")
    try:
        subprocess.run(["scp", "-q", str(binary), str(ROOT / "hosted/deploy/psms-hosted.service"),
                        host + ":" + stage + "/"], check=True)
        script = """
set -eu
cd "$1"
echo "$2  psms-hosted" | sha256sum -c -
exec 9>/run/psms-hosted-deploy.lock
flock -n 9
if test -x /usr/local/bin/psms-hosted; then cp -p /usr/local/bin/psms-hosted ./previous; fi
install -m755 psms-hosted /usr/local/bin/psms-hosted.new
mv -f /usr/local/bin/psms-hosted.new /usr/local/bin/psms-hosted
install -m644 psms-hosted.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable psms-hosted
systemctl restart psms-hosted
if ! curl -fsS --retry 6 --retry-all-errors --max-time 5 http://127.0.0.1:47833/free/v1/readyz; then
 if test -x previous; then install previous /usr/local/bin/psms-hosted && systemctl restart psms-hosted; fi
 exit 1
fi
"""
        ssh(host, "sh -s -- "+stage+" "+digest, script.encode())
        if publish:
            ssh(host, "python3 -", (ROOT / "hosted/deploy/publish.py").read_bytes())
            ssh(host, "curl -fsS --retry 3 --max-time 10 https://"+host+".sms.papamo.dev/free/v1/readyz")
        print(host, "ready", flush=True)
    finally:
        ssh(host,"rm -f "+stage+"/psms-hosted "+stage+"/psms-hosted.service "+stage+"/previous; rmdir "+stage)

if __name__ == "__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("binary",type=Path)
    p.add_argument("--publish",action="store_true",help="enable public HTTPS routes after smoke tests")
    args=p.parse_args()
    binary=args.binary.resolve(strict=True)
    ssh("na","runuser -u psms_hosted -- psql -X -v ON_ERROR_STOP=1 -d psms_accounts",(ROOT/"hosted/schema.sql").read_bytes())
    for region in REGIONS:
        deploy(region,binary,hashlib.file_digest(binary.open("rb"),"sha256").hexdigest(),args.publish)

"""Roll out prebuilt Linux binaries to the provisioned pSMS hosts over SSH."""
import argparse
import hashlib
from pathlib import Path
import re

import subprocess

ROOT = Path(__file__).resolve().parents[1]
REGIONS = ("na", "sg", "ws", "sa")
# Service name, binary and the local readiness URL checked after a restart.
HEALTH = {"psms": "http://127.0.0.1:47832/readyz",
          "psms-hosted": "http://127.0.0.1:47833/free/v1/readyz"}

def ssh(host, command, data=None):
    return subprocess.run(["ssh", "-o", "BatchMode=yes", host, command], input=data,
                          capture_output=False, check=True)

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def deploy(host, binaries, publish):
    stage = subprocess.check_output(["ssh", host, "mktemp -d /var/tmp/psms-release.XXXXXXXX"], text=True).strip()
    if not re.fullmatch(r"/var/tmp/psms-release\.[A-Za-z0-9]{8}", stage):
        raise ValueError("unexpected staging directory")
    names = list(binaries)
    try:
        subprocess.run(["scp", "-q", *map(str, binaries.values()),
                        str(ROOT / "hosted/deploy/psms-hosted.service"), host + ":" + stage + "/"],
                       check=True)
        for name, path in binaries.items():
            if path.name != name:
                ssh(host, f"mv {stage}/{path.name} {stage}/{name}")
        checks = "\n".join(f'echo "{digest(p)}  {n}" | sha256sum -c -' for n, p in binaries.items())
        installs = "\n".join(f"""if test -x /usr/local/bin/{n}; then cp -p /usr/local/bin/{n} ./{n}.previous; fi
install -m755 {n} /usr/local/bin/{n}.new
mv -f /usr/local/bin/{n}.new /usr/local/bin/{n}""" for n in names)
        rollback = "\n".join(f"if test -x {n}.previous; then install {n}.previous /usr/local/bin/{n}; fi" for n in names)
        probes = " && ".join(f"curl -fsS --retry 6 --retry-all-errors --max-time 5 {HEALTH[n]}" for n in names)
        script = f"""
set -eu
cd "$1"
{checks}
exec 9>/run/psms-hosted-deploy.lock
flock -n 9
{installs}
install -m644 psms-hosted.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable psms-hosted
systemctl restart {" ".join(names)}
if ! {{ {probes}; }}; then
 {rollback}
 systemctl restart {" ".join(names)}
 exit 1
fi
"""
        ssh(host, "sh -s -- " + stage, script.encode())
        if publish:
            ssh(host, "python3 -", (ROOT / "hosted/deploy/publish.py").read_bytes())
            ssh(host, "curl -fsS --retry 3 --max-time 10 https://"+host+".sms.papamo.dev/free/v1/readyz")
        print(host, "ready", flush=True)
    finally:
        ssh(host, "rm -rf -- " + stage)

if __name__ == "__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("binary",type=Path,help="psms-hosted built for x86_64 Linux")
    p.add_argument("--psms",type=Path,help="also roll out this psms archive binary")
    p.add_argument("--regions",default=",".join(REGIONS),help="comma-separated, in rollout order")
    p.add_argument("--publish",action="store_true",help="enable public HTTPS routes after smoke tests")
    args=p.parse_args()
    binaries={"psms-hosted":args.binary.resolve(strict=True)}
    if args.psms:
        binaries["psms"]=args.psms.resolve(strict=True)
    regions=args.regions.split(",")
    if not regions or any(r not in REGIONS for r in regions):
        p.error("regions must come from "+",".join(REGIONS))
    ssh("na","runuser -u psms_hosted -- psql -X -v ON_ERROR_STOP=1 -d psms_accounts",(ROOT/"hosted/schema.sql").read_bytes())
    for region in regions:
        deploy(region,binaries,args.publish)

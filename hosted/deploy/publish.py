"""Install hosted API locations in the existing pSMS regional HTTPS server."""
from pathlib import Path
import subprocess

site = Path("/etc/nginx/sites-enabled/psms-regional").resolve(strict=True)
old = site.read_text()
anchor = "    location ^~ /internal/sync/"
zones = Path("/etc/nginx/conf.d/psms-hosted-limits.conf")
limits = """limit_req_zone $binary_remote_addr zone=psms_auth_ip:1m rate=10r/m;
limit_req_zone $server_name zone=psms_signup_global:1m rate=10r/m;
"""
routes = """    location = /free/v1/auth/register {
        limit_req zone=psms_auth_ip burst=5 nodelay;
        limit_req zone=psms_signup_global burst=10 nodelay;
        limit_req_status 429;
        proxy_pass http://127.0.0.1:47833;
    }
    location = /free/v1/auth/login {
        limit_req zone=psms_auth_ip burst=8 nodelay;
        limit_req_status 429;
        proxy_pass http://127.0.0.1:47833;
    }
    location ^~ /free/v1/ { proxy_pass http://127.0.0.1:47833; }
"""
# Contact cards carry a small photo, so they get a larger body limit than the rest of the API.
cards = "    location ^~ /free/v1/contacts/ { client_max_body_size 512k; proxy_pass http://127.0.0.1:47833; }\n"
general = "    location ^~ /free/v1/ { proxy_pass http://127.0.0.1:47833; }\n"
if routes not in old and old.count(anchor) != 1:
    raise SystemExit("unexpected pSMS nginx layout")
if zones.exists() and zones.read_text() != limits:
    raise SystemExit("conflicting pSMS limit zones")
existed = zones.exists()
try:
    zones.write_text(limits)
    text = old if routes in old else old.replace(anchor,routes+anchor)
    if cards not in text:
        if text.count(general) != 1:
            raise SystemExit("unexpected pSMS nginx layout")
        text = text.replace(general,cards+general)
    if text != old:
        site.write_text(text)
    subprocess.run(["nginx","-t"],check=True)
    subprocess.run(["systemctl","reload","nginx"],check=True)
except Exception:
    site.write_text(old)
    if not existed:
        zones.unlink(missing_ok=True)
    raise

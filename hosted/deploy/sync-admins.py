#!/usr/bin/env python3
"""Refresh the Chicago entitlement snapshot from the Singapore identity database."""
import subprocess
from uuid import UUID

psql = ["psql", "-X", "-w", "-qAt", "-v", "ON_ERROR_STOP=1"]
subjects = subprocess.check_output(
    [*psql, "-d", "competitive", "-c", "SELECT id FROM psms_admin_subjects"],
    timeout=10, text=True,
).splitlines()
# Canonical UUIDs are the only data sent across the tunnel.
rows = "".join(str(UUID(subject)) + "\n" for subject in subjects)
refresh = """BEGIN;
CREATE TEMP TABLE current_admins(id uuid) ON COMMIT DROP;
COPY current_admins FROM STDIN;
""" + rows + """\\.
INSERT INTO admin_role_snapshot(singleton,subjects,expires_at)
SELECT true, coalesce(jsonb_agg(id), '[]'::jsonb), clock_timestamp() + interval '5 minutes'
FROM current_admins
ON CONFLICT(singleton) DO UPDATE SET subjects=excluded.subjects, expires_at=excluded.expires_at;
COMMIT;
"""
subprocess.run(
    [*psql, "-d", "host=127.0.0.1 port=55432 user=psms_roles dbname=psms_accounts connect_timeout=5"],
    input=refresh, text=True, timeout=15, check=True,
)

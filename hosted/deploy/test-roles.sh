#!/bin/sh
# Run as an operator on the Chicago primary with a candidate binary and this directory.
set -eu
binary=$(realpath "$1")
deploy=$(dirname "$(realpath "$0")")
stage=$(mktemp -d /var/tmp/psms-quota.XXXXXXXX)
name="psms_qa_$(date +%s)_$$"
runuser -u postgres -- createdb -O psms_hosted "$name"
pid=''
cleanup() {
 if test -n "$pid"; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
 runuser -u postgres -- dropdb "$name"
 rm -f "$stage/pgpass" "$stage/psms-hosted"
 rmdir "$stage"
}
trap cleanup EXIT
test -z "$(ss -Hltn 'sport = :47839')"
chmod 711 "$stage"
install -m755 "$binary" "$stage/psms-hosted"
runuser -u psms_hosted -- psql -Xq -v ON_ERROR_STOP=1 -d "$name" <"$deploy/../schema.sql"
umask 077
sed "s/:psms_accounts:/:$name:/" /etc/psms/hosted-db/pgpass >"$stage/pgpass"
chown psms_hosted:psms_hosted "$stage/pgpass"
export PSMS_HOSTED_PORT=47839 PSMS_HOSTED_PGPASS="$stage/pgpass" PSMS_TEST_DATABASE="$name"
runuser -u postgres -- psql -Xq -v ON_ERROR_STOP=1 -d "$name" -c 'GRANT SELECT, INSERT, UPDATE, DELETE ON accounts,sessions,archived_messages TO psms_runtime; GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO psms_runtime'
runuser -u psms_hosted -- "$stage/psms-hosted" &
pid=$!
curl -sf --retry 5 --retry-all-errors http://127.0.0.1:47839/free/v1/readyz
python3 "$deploy/smoke.py"

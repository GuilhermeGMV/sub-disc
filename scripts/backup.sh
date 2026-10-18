#!/usr/bin/env bash
# Backup consistente com breve interrupção apenas do backend.
set -euo pipefail
cd "$(dirname "$0")/.."
subdisc_backup_dir=${1:-$PWD/backups}
mkdir -p "$subdisc_backup_dir"
subdisc_backup_dir=$(cd "$subdisc_backup_dir" && pwd)
subdisc_archive="subdisc-$(date -u +%Y%m%dT%H%M%SZ).tar.gz"
trap 'docker compose start app >/dev/null' EXIT
docker compose stop app
docker compose run --rm --no-deps --user root -v "$subdisc_backup_dir:/backup" --entrypoint sh app -c 'tar -czf "/backup/$1" -C /data . && chmod 600 "/backup/$1"' -- "$subdisc_archive"
printf 'Backup: %s/%s\n' "$subdisc_backup_dir" "$subdisc_archive"

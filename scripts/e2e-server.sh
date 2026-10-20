#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
subdisc_test_dir=$(mktemp -d /tmp/subdisc-e2e.XXXXXX)
export APP_ENV=development APP_ORIGIN=http://127.0.0.1:18080 BIND=127.0.0.1:18080
export DATABASE_URL="sqlite://$subdisc_test_dir/test.db" SETUP_KEY=a-safe-setup-test-key STATIC_DIR="$PWD/frontend/dist"
export MEDIA_ICE_SERVERS='[]' MEDIA_PUBLIC_IP='' MEDIA_UDP_MIN=50000 MEDIA_UDP_MAX=51999
printf 'E2E logs: %s\n' "$subdisc_test_dir"
exec "${SUBDISC_BINARY:-$PWD/backend/target/debug/subdisc}" > "$subdisc_test_dir/backend.log" 2>&1

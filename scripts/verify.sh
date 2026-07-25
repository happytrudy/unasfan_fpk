#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SUPERIO_BINARY="${ROOT_DIR}/app/bin/superio-fanctl"
DAEMON_BINARY="${ROOT_DIR}/app/bin/fan-daemon"

bash -n "${ROOT_DIR}/cmd/main"
for json in "${ROOT_DIR}/config/privilege" "${ROOT_DIR}/config/resource" "${ROOT_DIR}/wizard/install" "${ROOT_DIR}/wizard/config"; do
    python3 -m json.tool "${json}" >/dev/null
done
[ -x "${SUPERIO_BINARY}" ] || { printf 'Missing executable: %s\n' "${SUPERIO_BINARY}" >&2; exit 1; }
superio_info=$(file -b "${SUPERIO_BINARY}")
case "${superio_info}" in *ELF*64-bit*x86-64*) ;; *) printf 'Expected x86-64 Rust helper, got: %s\n' "${superio_info}" >&2; exit 1 ;; esac
! readelf -l "${SUPERIO_BINARY}" | grep -q 'INTERP' || { printf 'Super I/O helper must be statically linked.\n' >&2; exit 1; }
"${SUPERIO_BINARY}" --help >/dev/null 2>&1
[ -x "${DAEMON_BINARY}" ] || { printf 'Missing Rust fan daemon: %s\n' "${DAEMON_BINARY}" >&2; exit 1; }
daemon_info=$(file -b "${DAEMON_BINARY}")
case "${daemon_info}" in *ELF*64-bit*x86-64*) ;; *) printf 'Expected x86-64 Rust daemon, got: %s\n' "${daemon_info}" >&2; exit 1 ;; esac
! readelf -l "${DAEMON_BINARY}" | grep -q 'INTERP' || { printf 'Rust daemon must be statically linked.\n' >&2; exit 1; }
printf 'Verification passed: Super I/O fan controller and Rust daemon are x86-64.\n'

#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
FANCTL_BINARY="${ROOT_DIR}/app/bin/superio-fanctl"
DAEMON_BINARY="${ROOT_DIR}/app/bin/fan-daemon"

bash -n "${ROOT_DIR}/cmd/main"
for json in "${ROOT_DIR}/config/privilege" "${ROOT_DIR}/config/resource" "${ROOT_DIR}/wizard/install" "${ROOT_DIR}/wizard/config"; do
    python3 -m json.tool "${json}" >/dev/null
done
[ -x "${FANCTL_BINARY}" ] || { printf 'Missing executable: %s\n' "${FANCTL_BINARY}" >&2; exit 1; }
fanctl_info=$(file -b "${FANCTL_BINARY}")
case "${fanctl_info}" in *ELF*64-bit*x86-64*) ;; *) printf 'Expected x86-64 Rust helper, got: %s\n' "${fanctl_info}" >&2; exit 1 ;; esac
! readelf -l "${FANCTL_BINARY}" | grep -q 'INTERP' || { printf 'I2C fan helper must be statically linked.\n' >&2; exit 1; }
"${FANCTL_BINARY}" --help >/dev/null 2>&1
[ -x "${DAEMON_BINARY}" ] || { printf 'Missing Rust fan daemon: %s\n' "${DAEMON_BINARY}" >&2; exit 1; }
daemon_info=$(file -b "${DAEMON_BINARY}")
case "${daemon_info}" in *ELF*64-bit*x86-64*) ;; *) printf 'Expected x86-64 Rust daemon, got: %s\n' "${daemon_info}" >&2; exit 1 ;; esac
! readelf -l "${DAEMON_BINARY}" | grep -q 'INTERP' || { printf 'Rust daemon must be statically linked.\n' >&2; exit 1; }
printf 'Verification passed: I801 I2C fan controller and Rust daemon are x86-64.\n'

#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
OUTPUT_DIR=${OUTPUT_DIR:-"${ROOT_DIR}/dist"}
PACKAGE_VERSION=${VERSION:-$(sed -n 's/^version=//p' "${ROOT_DIR}/manifest" | head -n 1)}
if [ -n "${VERSION:-}" ] && [[ ! "${PACKAGE_VERSION}" =~ ^[0-9]{14}$ ]]; then
    printf 'VERSION must be a 14-digit UTC timestamp (YYYYMMDDHHMMSS).\n' >&2
    exit 1
fi
TOOL_TMP=""
STAGING_DIR=$(mktemp -d)
SOURCE_TMP=$(mktemp -d)
PACKAGE_SOURCE="${SOURCE_TMP}/unasfan_fpk"
cleanup() { rm -rf "${STAGING_DIR}" "${SOURCE_TMP}"; [ -z "${TOOL_TMP}" ] || rm -f "${TOOL_TMP}"; }
trap cleanup EXIT

native_dir="${ROOT_DIR}/native/superio-fanctl"
native_target="x86_64-unknown-linux-musl"
cargo build --target "${native_target}" --release --manifest-path "${native_dir}/Cargo.toml"
install -m 0755 "${native_dir}/target/${native_target}/release/superio-fanctl" "${ROOT_DIR}/app/bin/superio-fanctl"
install -m 0755 "${native_dir}/target/${native_target}/release/fan-daemon" "${ROOT_DIR}/app/bin/fan-daemon"
"${ROOT_DIR}/scripts/verify.sh"
find_fnpack() {
    local machine candidate
    machine=$(uname -m)
    case "${machine}" in x86_64|amd64) machine=amd64 ;; aarch64|arm64) machine=arm64 ;; *) return 1 ;; esac
    [ -n "${FNPACK:-}" ] && { printf '%s\n' "${FNPACK}"; return; }
    for candidate in "${ROOT_DIR}/../buildbot/fnpack/fnpack-1.2.1-linux-${machine}" "${ROOT_DIR}/../fnpack/fnpack-1.2.1-linux-${machine}" "${ROOT_DIR}/../fnpack-1.2.1-linux-${machine}"; do
        [ -f "${candidate}" ] && { printf '%s\n' "${candidate}"; return; }
    done
    command -v fnpack
}
source_tool=$(find_fnpack)
if [ -x "${source_tool}" ]; then fnpack="${source_tool}"; else TOOL_TMP=$(mktemp); install -m 0755 "${source_tool}" "${TOOL_TMP}"; fnpack="${TOOL_TMP}"; fi
mkdir -p "${OUTPUT_DIR}" "${PACKAGE_SOURCE}"
cp -a "${ROOT_DIR}/." "${PACKAGE_SOURCE}/"
rm -rf "${PACKAGE_SOURCE}/dist" "${PACKAGE_SOURCE}/.git"
sed -i "s/^version=.*/version=${PACKAGE_VERSION}/" "${PACKAGE_SOURCE}/manifest"
(cd "${STAGING_DIR}" && "${fnpack}" build --directory "${PACKAGE_SOURCE}")
mapfile -t outputs < <(find "${STAGING_DIR}" -maxdepth 1 -type f -name '*.fpk' -print)
[ "${#outputs[@]}" -eq 1 ] || { printf 'Expected one FPK, found %s\n' "${#outputs[@]}" >&2; exit 1; }
package_path="${OUTPUT_DIR}/unasfan_fpk-${PACKAGE_VERSION}-x86.fpk"
install -m 0644 "${outputs[0]}" "${package_path}"
sha256sum "${package_path}" >"${package_path}.sha256"
printf 'Built: %s\n' "${package_path}"

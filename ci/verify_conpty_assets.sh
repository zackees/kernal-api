#!/usr/bin/env bash
set -euo pipefail

asset_dir="${1:?usage: verify_conpty_assets.sh <asset-directory>}"
manifest="${asset_dir}/conpty-sidecar.sha256.toml"

[[ -f "${manifest}" ]] || {
  echo "ConPTY asset manifest is missing: ${manifest}" >&2
  exit 1
}

for arch in x64 arm64 x86; do
  archive="${asset_dir}/conpty-sidecar-${arch}.tar.zst"
  [[ -s "${archive}" ]] || {
    echo "ConPTY archive is missing or empty: ${archive}" >&2
    exit 1
  }

  expected_sha="$(awk -v arch="${arch}" '
    $0 == "[asset." arch "]" { found = 1; next }
    found && /^sha256 = / { value = $0; sub(/^sha256 = "/, "", value); sub(/"$/, "", value); print value; exit }
  ' "${manifest}")"
  expected_size="$(awk -v arch="${arch}" '
    $0 == "[asset." arch "]" { found = 1; next }
    found && /^size_bytes = / { print $3; exit }
  ' "${manifest}")"
  actual_sha="$(sha256sum "${archive}" | awk '{ print $1 }')"
  actual_size="$(wc -c < "${archive}" | tr -d ' ')"
  [[ "${actual_sha}" == "${expected_sha}" && "${actual_size}" == "${expected_size}" ]] || {
    echo "ConPTY manifest does not match ${archive}" >&2
    exit 1
  }

  contents="$(tar --zstd -tf "${archive}")"
  for entry in conpty.dll OpenConsole.exe VERSION.txt; do
    grep -Eq "^(\\./)?${entry}$" <<<"${contents}" || {
      echo "ConPTY archive ${archive} is missing ${entry}" >&2
      exit 1
    }
  done
done

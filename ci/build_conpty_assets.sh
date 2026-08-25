#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:-${repo_root}/target/conpty-assets}"
epoch="${SOURCE_DATE_EPOCH:-0}"
version="$(awk '$1 == "Microsoft.Windows.Console.ConPTY" { print $2 }' "${repo_root}/WINDOWS_CONPTY_VERSION.txt")"

if [[ -z "${version}" ]]; then
  echo "WINDOWS_CONPTY_VERSION.txt has no package version" >&2
  exit 1
fi

mkdir -p "${output_dir}"
manifest="${output_dir}/conpty-sidecar.sha256.toml"
: > "${manifest}"

for arch in x64 arm64 x86; do
  source_dir="${repo_root}/vendor/win32/conpty/${arch}"
  stage="${output_dir}/stage-${arch}"
  archive="${output_dir}/conpty-sidecar-${arch}.tar.zst"
  rm -rf "${stage}"
  mkdir -p "${stage}"
  cp "${source_dir}/conpty.dll" "${stage}/conpty.dll"
  cp "${source_dir}/OpenConsole.exe" "${stage}/OpenConsole.exe"
  printf '%s\n' "${version}" > "${stage}/VERSION.txt"
  tar \
    --sort=name \
    --mtime="@${epoch}" \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    -cf - \
    -C "${stage}" . | zstd -q -19 -T1 -f -o "${archive}"
  sha256="$(sha256sum "${archive}" | awk '{ print $1 }')"
  size="$(wc -c < "${archive}" | tr -d ' ')"
  {
    printf '[asset.%s]\n' "${arch}"
    printf 'sha256 = "%s"\n' "${sha256}"
    printf 'size_bytes = %s\n\n' "${size}"
  } >> "${manifest}"
done

rm -rf "${output_dir}"/stage-*
printf 'wrote %s\n' "${manifest}"

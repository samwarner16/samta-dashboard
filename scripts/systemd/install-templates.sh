#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${1:-${HOME}/.config/systemd/user}"
mkdir -p "${TARGET_DIR}"

for template in "${REPO_DIR}"/scripts/systemd/*.template; do
  unit_name="$(basename "${template}" .template)"
  case "$unit_name" in
    api|api.service)
      unit_name="go-ahead-and-call-api.service"
      ;;
    worker|worker.service)
      unit_name="go-ahead-and-call-worker.service"
      ;;
  esac
  destination="${TARGET_DIR}/${unit_name}"

  sed "s#<REPO_DIR>#${REPO_DIR}#g" "${template}" > "${destination}"
  echo "installed ${destination}"

done

echo "Enable with:"
echo "systemctl --user daemon-reload"
echo "systemctl --user enable --now go-ahead-and-call-api.service go-ahead-and-call-worker.service"

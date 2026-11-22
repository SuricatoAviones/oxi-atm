#!/usr/bin/env bash
# Trío de calidad dentro de WSL. Smart App Control no evalúa binarios Linux,
# así que el runner de tests vive acá cuando Windows los bloquea.
#
# Uso desde Windows:
#   wsl -e bash /mnt/c/Users/luisa/Desktop/oxi-atm/scripts/check-wsl.sh
set -euo pipefail

# Raíz del repo: este script vive en scripts/.
cd "$(dirname "$0")/.."

# No pisar los artefactos de Windows (target/): dir propio para Linux, en el
# filesystem NATIVO de WSL (compilar sobre /mnt/c es un orden de magnitud
# más lento por el puente 9P).
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/oxi-atm-target}"

# rustup vive en ~/.cargo/bin; el shell no-login de wsl -e no carga ~/.profile.
export PATH="${HOME}/.cargo/bin:${PATH}"

./scripts/check.sh

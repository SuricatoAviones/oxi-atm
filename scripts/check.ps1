# Trío de calidad del DoD (ROADMAP §1). Ejecutar antes de cada commit.
# Uso: .\scripts\check.ps1
$ErrorActionPreference = "Stop"

Write-Host "== cargo fmt --check =="
cargo fmt --check
if ($LASTEXITCODE -ne 0) { Write-Error "fmt falló"; exit 1 }

Write-Host "== cargo clippy --all-targets --all-features --locked -- -D warnings =="
cargo clippy --all-targets --all-features --locked -- -D warnings
if ($LASTEXITCODE -ne 0) { Write-Error "clippy falló"; exit 1 }

Write-Host "== cargo test =="
cargo test --quiet
if ($LASTEXITCODE -ne 0) { Write-Error "test falló"; exit 1 }

Write-Host "== TODO VERDE =="

#!/usr/bin/env bash
# Extension-contract §5 for the payment↔billing settlement seam: prove the cross-module ACL/consumer
# wiring survives a regeneration of BOTH modules. Snapshots the seam files, regenerates payment AND
# billing with --force, asserts byte-identical, and re-runs the end-to-end seam test green.
# Usage: DATABASE_URL=... bash scripts/settlement_seam_roundtrip.sh
set -euo pipefail
cd "$(dirname "$0")/.."

PAY_FILES=(
  src/application/service/payment_write_service.rs
  src/application/service/payment_events.rs
  src/application/service/payment_gl.rs
  src/presentation/http/guarded_routes.rs
  tests/settlement_seam.rs
)
BILL_FILES=(
  ../backbone-billing/src/application/service/billing_write_service.rs
)

echo "→ snapshot seam consumer/ACL files (both modules)"
before=$(shasum -a 256 "${PAY_FILES[@]}" "${BILL_FILES[@]}")

echo "→ regenerate BOTH modules (§5) — billing then payment"
( cd ../backbone-billing && metaphor schema schema generate --force >/dev/null )
metaphor schema schema generate --force >/dev/null

echo "→ verify every seam file is byte-identical after regen"
after=$(shasum -a 256 "${PAY_FILES[@]}" "${BILL_FILES[@]}")
if [ "$before" != "$after" ]; then
  echo "✗ FAIL: a seam file changed during regen"; diff <(echo "$before") <(echo "$after") || true; exit 1
fi
echo "  ✓ all ${#PAY_FILES[@]}+${#BILL_FILES[@]} seam files unchanged"

echo "→ re-run the end-to-end settlement seam post-regen"
cargo test --test settlement_seam -- --test-threads=1 >/dev/null
echo "  ✓ billing→payment→accounting→billing seam still green after regenerating both modules"
echo "✓ §5 round-trip proven for the settlement seam."

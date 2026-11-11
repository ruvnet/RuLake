#!/usr/bin/env bash
#
# ruLake — live public IPFS gateway reachability probe.
#
# Scheduled, non-blocking companion to the hermetic IPFS fixture used by
# ui/scripts/smoke.sh (issue #19). Public gateways rate-limit CI runners
# and change behaviour without notice (w3s.link now 301s to dweb.link,
# which answers 429; ipfs.io serves an interstitial page), so this check
# is kept out of the PR path and runs from
# .github/workflows/ipfs-gateway-probe.yml instead.
#
# For each gateway it fetches the Console's canary CID with retry and
# exponential backoff, then verifies the body's sha256 against the
# digest embedded in the CID. A 200 that carries an interstitial page
# does not count. Exits 0 when at least MIN_OK gateways serve the right
# bytes, 1 otherwise.
#
# Usage:
#   ./scripts/ipfs-gateway-probe.sh
#   GATEWAYS="https://a https://b" MIN_OK=2 ./scripts/ipfs-gateway-probe.sh

set -uo pipefail

CID="${CID:-bafkreigh2akiscaildcqabsyg3dfr6chu3fgpregiymsck7e7aqa4s52zy}"
GATEWAYS="${GATEWAYS:-https://w3s.link https://dweb.link https://ipfs.io https://gateway.pinata.cloud https://ipfs.filebase.io https://4everland.io}"
MIN_OK="${MIN_OK:-1}"
ATTEMPTS="${ATTEMPTS:-4}"

# Expected sha256 = the 32-byte digest inside the CIDv1 raw multihash
# (prefix 0x01 0x55 0x12 0x20). Decoded rather than hard-coded so the
# check stays honest if CID is overridden.
EXPECTED=$(node -e '
  const s = process.argv[1];
  if (!s.startsWith("b")) { process.exit(2); }
  const A = "abcdefghijklmnopqrstuvwxyz234567";
  let bits = 0, v = 0; const out = [];
  for (const ch of s.slice(1)) {
    const i = A.indexOf(ch); if (i < 0) process.exit(2);
    v = (v << 5) | i; bits += 5;
    if (bits >= 8) { out.push((v >> (bits - 8)) & 255); bits -= 8; }
  }
  const b = Buffer.from(out);
  if (b[0] !== 1 || b[1] !== 0x55 || b[2] !== 0x12 || b[3] !== 0x20) process.exit(2);
  process.stdout.write(b.subarray(4, 36).toString("hex"));
' "$CID") || { echo "not a CIDv1 raw/sha256 CID: $CID" >&2; exit 2; }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

ok_count=0
for gw in $GATEWAYS; do
  url="${gw%/}/ipfs/${CID}"
  result="FAIL"
  detail=""
  delay=2
  for attempt in $(seq 1 "$ATTEMPTS"); do
    code=$(curl -sSL -m 30 -o "$TMP/body" -w '%{http_code}' \
      -H 'Origin: https://ruvnet.github.io' "$url" 2>"$TMP/err") || code="000"
    if [[ "$code" == "200" ]]; then
      got=$(sha256sum "$TMP/body" | cut -d' ' -f1)
      if [[ "$got" == "$EXPECTED" ]]; then
        result="OK"
        detail="attempt ${attempt}, $(stat -c %s "$TMP/body") B, sha256 verified"
        break
      fi
      detail="attempt ${attempt}: 200 but sha256 ${got:0:12}… != ${EXPECTED:0:12}…"
    else
      detail="attempt ${attempt}: HTTP ${code} $(head -c 120 "$TMP/err" | tr '\n' ' ')"
    fi
    if [[ "$attempt" -lt "$ATTEMPTS" ]]; then
      sleep "$delay"
      delay=$((delay * 2))
    fi
  done
  printf '%-4s %-32s %s\n' "$result" "$gw" "$detail"
  [[ "$result" == "OK" ]] && ok_count=$((ok_count + 1))
done

echo
if [[ "$ok_count" -ge "$MIN_OK" ]]; then
  echo "ipfs-gateway-probe: ${ok_count} gateway(s) served ${CID} with a verified hash (need ${MIN_OK})"
  exit 0
fi
echo "ipfs-gateway-probe: only ${ok_count} gateway(s) served ${CID} with a verified hash (need ${MIN_OK})" >&2
exit 1

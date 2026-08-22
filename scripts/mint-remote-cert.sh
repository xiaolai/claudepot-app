#!/usr/bin/env bash
# Mint the TLS material for Claudepot's remote surface.
#
# Claudepot terminates TLS itself. A self-hosted tailnet control server
# cannot issue certificates, so `tailscale serve` is not available and
# there is no public CA that will sign a `.internal` name. This script
# creates a small private CA and a leaf certificate for this machine.
#
# Install the CA (printed at the end) on each phone or laptop that will
# reach Claudepot. On iOS that is TWO steps and the second is the one
# everybody forgets:
#   1. AirDrop / open the .crt  ->  Settings > Profile Downloaded > Install
#   2. Settings > General > About > Certificate Trust Settings
#      -> enable full trust for the root
# Without step 2 Safari rejects it and the failure looks like a bug.
#
# Idempotent: an existing CA is reused so previously-trusted devices
# keep working. Delete the CA files to start over — every device then
# has to install the new one.
set -euo pipefail

DATA_DIR="${CLAUDEPOT_DATA_DIR:-$HOME/.claudepot}"
CA_KEY="$DATA_DIR/remote-ca-key.pem"
CA_CRT="$DATA_DIR/remote-ca.crt"
LEAF_KEY="$DATA_DIR/remote-key.pem"
LEAF_CRT="$DATA_DIR/remote-cert.pem"

# Safari refuses a leaf valid for more than 398 days (Apple's policy
# since Sep 2020), and the resulting error says nothing about duration.
# 397 keeps a day of headroom. The CA itself is exempt from that rule.
LEAF_DAYS=397
CA_DAYS=3650

host="${1:-}"
if [ -z "$host" ]; then
  host="$(hostname -s).$(tailscale status --json 2>/dev/null \
    | sed -n 's/.*"MagicDNSSuffix":"\([^"]*\)".*/\1/p')"
  host="${host%.}"
fi
[ -z "${host#*.}" ] && { echo "usage: $0 <hostname>  (could not derive one)" >&2; exit 1; }

# Both a DNS name and the tailnet IP: a user may reach either, and a
# certificate without a matching SAN fails with a message that blames
# the connection rather than the certificate.
ts_ip="$(tailscale ip -4 2>/dev/null | head -1 || true)"

mkdir -p "$DATA_DIR"
chmod 700 "$DATA_DIR"

if [ ! -f "$CA_KEY" ] || [ ! -f "$CA_CRT" ]; then
  echo "==> creating a new CA (devices will need to trust this)"
  openssl req -x509 -newkey rsa:4096 -sha256 -days "$CA_DAYS" -nodes \
    -keyout "$CA_KEY" -out "$CA_CRT" \
    -subj "/CN=Claudepot Remote CA/O=Claudepot" \
    -addext "basicConstraints=critical,CA:TRUE,pathlen:0" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
  chmod 600 "$CA_KEY"
  chmod 644 "$CA_CRT"
else
  echo "==> reusing the existing CA (already-trusted devices keep working)"
fi

san="DNS:$host"
[ -n "$ts_ip" ] && san="$san,IP:$ts_ip"
echo "==> minting a leaf for $san (${LEAF_DAYS}d)"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

openssl req -newkey rsa:2048 -sha256 -nodes \
  -keyout "$LEAF_KEY" -out "$tmp/leaf.csr" \
  -subj "/CN=$host" 2>/dev/null

# extendedKeyUsage=serverAuth is required by Safari; without it the
# certificate is structurally valid and still refused.
openssl x509 -req -in "$tmp/leaf.csr" -sha256 -days "$LEAF_DAYS" \
  -CA "$CA_CRT" -CAkey "$CA_KEY" -CAcreateserial \
  -out "$LEAF_CRT" \
  -extfile <(printf 'subjectAltName=%s\nextendedKeyUsage=serverAuth\nbasicConstraints=critical,CA:FALSE\n' "$san") \
  2>/dev/null

# The private key authenticates this machine to every paired device.
# `remote::tls` refuses to start the server if this is not owner-only.
chmod 600 "$LEAF_KEY"
chmod 644 "$LEAF_CRT"

echo
echo "cert : $LEAF_CRT"
echo "key  : $LEAF_KEY  (0600)"
echo
echo "Install this on every device that will connect:"
echo "  $CA_CRT"
echo
echo "iOS: install the profile, THEN enable full trust under"
echo "     Settings > General > About > Certificate Trust Settings."

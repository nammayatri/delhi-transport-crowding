#!/usr/bin/env bash
# Apply the crowding patch to an OpenTripPlanner 2.5 checkout (upstream v2.5.0 or a fork of it).
# Usage: ./apply-patch.sh /path/to/OpenTripPlanner
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target="${1:?path to an OpenTripPlanner checkout}"
cd "$target"
git apply --3way "$here"/patches/*.patch
echo "Applied. Build with: mvn -Dprettier.skip=true -DskipTests package"
echo "Enable with otp-config.json: {\"otpFeatures\": {\"Crowding\": true}}"
echo "and point OTP at the crowding service: OTP_CROWDING_INDEX_URL=http://crowd-service:8090/v1/otp/crowding-index"

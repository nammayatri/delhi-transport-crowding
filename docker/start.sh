#!/usr/bin/env bash
# Run the crowding service and, unless OTP_ENABLED=False, OpenTripPlanner in one container.
# OTP reads the crowding index from the service; the service plans journeys through OTP. If either
# process stops, the container stops, so the orchestrator restarts both.
set -uo pipefail

pids=()
if [[ "${OTP_ENABLED:-True}" == "True" ]]; then
  # shellcheck disable=SC2086
  java ${OTP_JAVA_OPTS:--Xmx3g} -jar /opt/otp/otp.jar --load --serve /opt/otp/graph &
  pids+=($!)
fi
crowd-service &
pids+=($!)

trap 'kill -TERM "${pids[@]}" 2>/dev/null' TERM INT
wait -n
status=$?
kill -TERM "${pids[@]}" 2>/dev/null
wait
exit "$status"

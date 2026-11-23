# Common tasks. Install `just` or run the commands by hand.
set shell := ["bash", "-cu"]

data := "data"
pipeline := "crowding/target/release/crowd-pipeline"

# Download the ticket data and unpack the GTFS feed.
fetch:
    mkdir -p {{data}}/raw/tickets {{data}}/raw/gtfs/delhi-bus
    curl -fL -o {{data}}/raw/tickets/October-Delhi.csv.zip https://pub-091bd9e794bb4877b10b29e6df80f065.r2.dev/2026-10/October-Delhi.csv.zip
    cd {{data}}/raw/tickets && unzip -o -q October-Delhi.csv.zip October-Delhi.csv
    test -f {{data}}/raw/gtfs/delhi-bus.gtfs.zip && cd {{data}}/raw/gtfs/delhi-bus && unzip -o -q ../delhi-bus.gtfs.zip

build:
    cd crowding && cargo build --release
    cd web && npm install && npx vite build

test:
    cd crowding && cargo test
    cd otp-crowding/otp-src && mvn -q -Dprettier.skip=true -Dtest=CrowdingCostCalculatorTest -Dsurefire.failIfNoSpecifiedTests=false test

# Build the crowding model from all of October.
profiles:
    {{pipeline}} --config crowding/config/model.dhall --gtfs {{data}}/raw/gtfs/delhi-bus --tickets {{data}}/raw/tickets/October-Delhi.csv --out {{data}}/out/full

# Every model variant plus the validation suite (week-to-week, sensitivity, patterns).
validate:
    scripts/run_all_models.sh

serve:
    cd crowding && DHALL_CONFIG=config/service.dhall CROWD_WEB_DIR=../web/dist target/release/crowd-service

web-dev:
    cd web && npx vite

# Replay Wednesday 28 October from 07:00 at 120x as live tickets.
replay date="2026-10-28" from="07:00" speed="120":
    crowding/target/release/crowd-replay --tickets {{data}}/raw/tickets/October-Delhi.csv --date {{date}} --from {{from}} --speed {{speed}}

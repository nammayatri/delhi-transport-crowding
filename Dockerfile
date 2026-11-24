# One image: the crowding service and web app, the model, and the crowding-aware OpenTripPlanner.
#
# Build context: the repository root, after `just profiles` has written data/out/full (the model
# snapshot) and `just validate` the live trees (data/out/full/live_trees.json), with the
# OpenTripPlanner inputs in data/raw (delhi-bus.gtfs.zip, delhi.osm.pbf).
#
#   docker build -t delhi-transport-crowding .                          # this machine's platform
#   docker buildx build --platform linux/amd64 -t ... .                 # e.g. from an Apple Silicon Mac
#
# Everything is compiled on the build machine's own platform (Rust cross-compiles); only the last
# stage is the target platform.

# ---- Rust: service, pipeline, replay ------------------------------------------------------------
FROM --platform=$BUILDPLATFORM rust:1.95-bookworm AS rust
ARG TARGETARCH
WORKDIR /src
COPY crowding ./crowding
RUN set -eux; \
    case "$TARGETARCH" in \
      amd64) T=x86_64-unknown-linux-gnu; G=x86-64-linux-gnu; M=x86_64-linux-gnu ;; \
      arm64) T=aarch64-unknown-linux-gnu; G=aarch64-linux-gnu; M=aarch64-linux-gnu ;; \
      *) echo "unsupported $TARGETARCH"; exit 1 ;; \
    esac; \
    if [ "$(uname -m)" != "${T%%-*}" ]; then \
      dpkg --add-architecture "$TARGETARCH"; \
      apt-get update; \
      apt-get install -y --no-install-recommends "gcc-$G" "libc6-dev-$TARGETARCH-cross" "libssl-dev:$TARGETARCH"; \
      rustup target add "$T"; \
      U=$(echo "$T" | tr 'a-z-' 'A-Z_'); L=$(echo "$T" | tr '-' '_'); \
      export "CARGO_TARGET_${U}_LINKER=$M-gcc" "CC_${L}=$M-gcc" "AR_${L}=$M-ar" \
             PKG_CONFIG_ALLOW_CROSS=1 "PKG_CONFIG_PATH=/usr/lib/$M/pkgconfig"; \
    fi; \
    cd crowding; \
    cargo build --release --target "$T" -p crowd-service -p crowd-pipeline -p crowd-replay; \
    mkdir -p /out; \
    cp "target/$T/release/crowd-service" "target/$T/release/crowd-pipeline" "target/$T/release/crowd-replay" /out/

# ---- Web app ------------------------------------------------------------------------------------
FROM --platform=$BUILDPLATFORM node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json* ./
RUN npm install --no-audit --no-fund
COPY web ./
RUN npx vite build

# ---- OpenTripPlanner 2.5 with the crowding patch, and the Delhi graph ------------------------
FROM --platform=$BUILDPLATFORM maven:3.9-eclipse-temurin-21 AS otp
WORKDIR /src
RUN git clone --depth 1 --branch v2.5.0 https://github.com/opentripplanner/OpenTripPlanner.git otp
COPY otp-crowding/patches /patches
RUN cd otp && git apply /patches/*.patch \
 && mvn -q -Dprettier.skip=true -DskipTests -Dmaven.javadoc.skip=true package \
 && cp target/otp-*-shaded.jar /otp.jar
WORKDIR /graph
COPY otp-crowding/delhi/*.json ./
COPY data/raw/gtfs/delhi-bus.gtfs.zip data/raw/delhi.osm.pbf ./
RUN java -Xmx5g -jar /otp.jar --build --save /graph && rm delhi-bus.gtfs.zip delhi.osm.pbf

# ---- Runtime --------------------------------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl libssl3 tini \
 && rm -rf /var/lib/apt/lists/*
COPY --from=eclipse-temurin:21-jre /opt/java/openjdk /opt/java/openjdk
COPY --from=rust /out/ /usr/local/bin/
COPY --from=web /web/dist /app/web
COPY --from=otp /otp.jar /opt/otp/otp.jar
COPY --from=otp /graph /opt/otp/graph
COPY crowding/config /app/config
COPY data/out/full/snapshot.msgpack.zst data/out/full/live_trees.json /data/out/full/
COPY docker/start.sh /usr/local/bin/start.sh
WORKDIR /app
ENV PATH=/opt/java/openjdk/bin:$PATH \
    DHALL_CONFIG=/app/config/service.dhall \
    CROWD_SNAPSHOT=/data/out/full/snapshot.msgpack.zst \
    LIVE_TREES=/data/out/full/live_trees.json \
    CROWD_WEB_DIR=/app/web \
    CROWD_CLOCK=wall \
    OTP_ENABLED=True \
    OTP_GRAPHQL_URL=http://localhost:8080/otp/gtfs/v1 \
    OTP_CROWDING_INDEX_URL=http://localhost:8090/v1/otp/crowding-index \
    OTP_CROWDING_TIMEZONE=Asia/Kolkata \
    OTP_JAVA_OPTS=-Xmx3g \
    RUST_LOG=info
EXPOSE 8090 8080
HEALTHCHECK CMD curl -sf localhost:8090/health || exit 1
ENTRYPOINT ["tini", "--"]
CMD ["start.sh"]

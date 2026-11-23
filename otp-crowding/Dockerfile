# Patched OpenTripPlanner 2.5 with the Delhi bus graph.
# Build context: repository root (needs data/raw/gtfs/delhi-bus.gtfs.zip and data/raw/delhi.osm.pbf).
FROM maven:3.9-eclipse-temurin-21 AS build
WORKDIR /src
RUN git clone --depth 1 --branch v2.5.0 https://github.com/opentripplanner/OpenTripPlanner.git otp
COPY otp-crowding/patches /patches
RUN cd otp && git apply /patches/*.patch \
 && mvn -q -Dprettier.skip=true -DskipTests -Dmaven.javadoc.skip=true package \
 && cp target/otp-*-shaded.jar /otp.jar

FROM eclipse-temurin:21-jre AS graph
COPY --from=build /otp.jar /otp.jar
WORKDIR /var/otp
COPY otp-crowding/delhi/*.json ./
COPY data/raw/gtfs/delhi-bus.gtfs.zip ./delhi-bus.gtfs.zip
COPY data/raw/delhi.osm.pbf ./delhi.osm.pbf
RUN java -Xmx10g -jar /otp.jar --build --save /var/otp

FROM eclipse-temurin:21-jre
COPY --from=build /otp.jar /otp.jar
COPY --from=graph /var/otp/graph.obj /var/otp/graph.obj
COPY otp-crowding/delhi/*.json /var/otp/
ENV OTP_CROWDING_INDEX_URL=http://crowd-service:8090/v1/otp/crowding-index \
    OTP_CROWDING_TIMEZONE=Asia/Kolkata
EXPOSE 8080
CMD ["java", "-Xmx8g", "-jar", "/otp.jar", "--load", "--serve", "/var/otp"]

# syntax=docker/dockerfile:1
FROM rust:1.97-trixie AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake pkg-config libasound2-dev libfontconfig1-dev libx11-dev libxcursor-dev \
    libxext-dev libxfixes-dev libxft-dev libxinerama-dev libxrandr-dev libxrender-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY media/icons/screacher-radio-icon-512.png ./media/icons/
COPY src ./src
ARG SCREACHER_RELEASE_TAG
ARG SCREACHER_COMMIT_SHA
RUN cargo test --release --locked && cargo build --release --locked

FROM scratch AS binary
COPY --from=build /app/target/release/screacher-radio /screacher-radio
COPY media/icons/screacher-radio-icon-512.png /screacher-radio.png
COPY media/linux/screacher-radio.desktop /screacher-radio.desktop

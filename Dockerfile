# Capivara em modo web — acesse de qualquer navegador (celular, tablet, outro PC).
#   docker build -t capivara .
#   docker run -p 8080:8080 -e CAPIVARA_TOKEN=minhasenha -v ./downloads:/downloads -v capivara-data:/data capivara
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY ui ./ui
RUN cargo build --release

FROM debian:bookworm-slim
ARG TARGETARCH
RUN apt-get update \
 && apt-get install -y --no-install-recommends ffmpeg ca-certificates curl unzip \
 && rm -rf /var/lib/apt/lists/*
RUN case "$TARGETARCH" in \
      arm64) YT=yt-dlp_linux_aarch64; DN=aarch64-unknown-linux-gnu ;; \
      *)     YT=yt-dlp_linux;         DN=x86_64-unknown-linux-gnu ;; \
    esac \
 && curl -fsSL -o /usr/local/bin/yt-dlp "https://github.com/yt-dlp/yt-dlp/releases/latest/download/$YT" \
 && chmod +x /usr/local/bin/yt-dlp \
 && curl -fsSL -o /tmp/deno.zip "https://github.com/denoland/deno/releases/latest/download/deno-$DN.zip" \
 && unzip -q /tmp/deno.zip -d /usr/local/bin && rm /tmp/deno.zip
COPY --from=build /src/target/release/capivara /usr/local/bin/capivara
ENV CAPIVARA_DATA=/data \
    CAPIVARA_OUTPUT=/downloads \
    CAPIVARA_HOST=0.0.0.0 \
    CAPIVARA_PORT=8080
VOLUME ["/data", "/downloads"]
EXPOSE 8080
CMD ["capivara", "serve"]

# syntax=docker/dockerfile:1
#
# lanthorn in a container — build once, serve it however you want.
#
#   docker build -t lanthorn .
#
# Two ways to run the result:
#
#   1. In YOUR terminal (full fidelity — kitty graphics pass straight through):
#        docker run -it --rm -v ~/if-games:/stories -v lanthorn-data:/data lanthorn
#
#   2. As a WEB SERVER (ttyd wraps lanthorn; point a browser at port 7681;
#      7682 carries the game's sound to the browser):
#        docker run -d -p 7681:7681 -p 7682:7682 -v ~/if-games:/stories -v lanthorn-data:/data lanthorn serve
#
# `/stories` is the game library (the story picker opens on it by default) and
# `/data` is $HOME — saves, config, and map archives live in /data/.lanthorn.
# See docs/features/docker.md for the full story, docker-compose.yml for an
# example deployment, and docker/entrypoint.sh for the serve-mode knobs
# (LANTHORN_WEB_PORT, LANTHORN_WEB_CREDENTIAL).

FROM rust:1-slim-trixie AS builder

# ALSA headers are the one native build dependency (audio's default features
# pull in rodio/cpal — same requirement as CI). git is for buildinfo's build
# script, which stamps the binary with its own short hash.
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libasound2-dev git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

# rustup reads rust-toolchain.toml and fetches the repo's pinned toolchain, so
# the container compiles with the same rustc the repo gates on.
#
# CARGO_BUILD_JOBS: `.cargo/config.toml` caps jobs at 8 for the developer's
# interactive machine; a throwaway build container should use every core it
# has (the env var wins over the config file — same override CI uses).
#
# The cache mounts keep the registry, the toolchain download, and incremental
# build artifacts across image rebuilds; the binaries are copied out because
# a cache mount's contents are not part of the image.
# The repo builds into `target.noindex` on a developer Mac; the image wants the
# conventional path so the `cp target/release/...` below holds.
ENV CARGO_TARGET_DIR=target
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/rustup \
    --mount=type=cache,target=/src/target \
    CARGO_BUILD_JOBS="$(nproc)" \
    cargo build --release --locked -p lanthorn -p lanthorn-zvm-cli -p lanthorn-gvm-cli -p lanthorn-scott-cli -p audio-relay \
    && mkdir -p /out \
    && cp target/release/lanthorn target/release/zvm-cli \
          target/release/gvm-cli target/release/scott-cli \
          target/release/lanthorn-audio-relay /out/

# ttyd (the web-terminal server behind `serve` mode) is not packaged by
# Debian; its releases ship static per-arch binaries, so fetch a pinned one.
FROM debian:trixie-slim AS ttyd-fetch
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
ARG TTYD_VERSION=1.7.7
RUN arch="$(dpkg --print-architecture)" \
    && case "$arch" in \
         amd64) t=x86_64 ;; \
         arm64) t=aarch64 ;; \
         armhf) t=armhf ;; \
         *) echo "no ttyd release binary for $arch" >&2; exit 1 ;; \
       esac \
    && curl -fsSL -o /ttyd \
         "https://github.com/tsl0922/ttyd/releases/download/${TTYD_VERSION}/ttyd.${t}" \
    && chmod +x /ttyd
# ttyd's page is compiled into its binary. Serve it once here and save the
# result: the entrypoint injects the browser-audio script into this copy and
# hands it back to ttyd with --index.
RUN sh -c '/ttyd -p 7999 true & pid=$!; sleep 1; curl -fsS -o /ttyd-index.html http://127.0.0.1:7999/; kill $pid' \
    && grep -q "</head>" /ttyd-index.html

# The browser page ships its own font, so a visitor's default monospace stops
# deciding whether lanthorn's Nerd Font icons and the map's Legacy Computing
# half-diagonals (U+1FBA0-1FBA3) render at all. IosevkaTerm Nerd Font Mono is
# the one Nerd Font that carries those diagonals. Iosevka itself is OFL-1.1;
# the Nerd Fonts patch is MIT; both redistributable, and the licence text
# ships in the image below. Fetched and pinned the same way as ttyd above —
# a release asset, verified by SHA-256 before anything unpacks it.
FROM debian:trixie-slim AS font-fetch
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl unzip woff2 \
    && rm -rf /var/lib/apt/lists/*
ARG NERD_FONTS_VERSION=3.5.1
ARG NERD_FONTS_SHA256=b0dfd98968b7c7743080431257bfd082c5ef7dd4d1c674d3ce16bc5520c10cdc
# Only the two static weights the page needs (Regular/Bold) are pulled out of
# the release zip — it ships every weight and both upright and oblique/italic
# variants, ~1.1 GB unpacked. Converted to woff2 here (the `woff2` Debian
# package's woff2_compress) so the served page embeds a few MB, not ~14 MB
# per face of raw TTF.
RUN curl -fsSL -o /tmp/IosevkaTerm.zip \
         "https://github.com/ryanoasis/nerd-fonts/releases/download/v${NERD_FONTS_VERSION}/IosevkaTerm.zip" \
    && printf '%s  /tmp/IosevkaTerm.zip\n' "${NERD_FONTS_SHA256}" > /tmp/IosevkaTerm.zip.sha256 \
    && sha256sum -c /tmp/IosevkaTerm.zip.sha256 \
    && mkdir -p /fonts \
    && unzip -o -j /tmp/IosevkaTerm.zip \
         IosevkaTermNerdFontMono-Regular.ttf IosevkaTermNerdFontMono-Bold.ttf LICENSE.md \
         -d /fonts \
    && woff2_compress /fonts/IosevkaTermNerdFontMono-Regular.ttf \
    && woff2_compress /fonts/IosevkaTermNerdFontMono-Bold.ttf \
    && rm /fonts/IosevkaTermNerdFontMono-Regular.ttf /fonts/IosevkaTermNerdFontMono-Bold.ttf \
          /tmp/IosevkaTerm.zip /tmp/IosevkaTerm.zip.sha256

# trixie-slim to match the builder's glibc (rust:1-slim-trixie above).
FROM debian:trixie-slim

# libasound2t64 (trixie's name for libasound2): the release binaries link ALSA
# (harmless without a sound device — the app degrades to silent).
# ca-certificates: HTTPS for the picker's built-in IFDB story downloads.
RUN apt-get update \
    && apt-get install -y --no-install-recommends libasound2t64 ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# A container has no sound card. ALSA's default device is the `file` plugin
# writing to the path in LANTHORN_AUDIO_OUT, or /dev/null when unset: silent
# and clean in a terminal, and in serve mode the session wrapper points it at
# a FIFO that lanthorn-audio-relay streams to the browser. See the file.
COPY docker/asound.conf /etc/asound.conf

COPY --from=builder /out/ /usr/local/bin/
COPY --from=ttyd-fetch /ttyd /usr/local/bin/ttyd
COPY --from=ttyd-fetch /ttyd-index.html /usr/local/share/lanthorn/ttyd-index.html
COPY --from=font-fetch /fonts/ /usr/local/share/lanthorn/fonts/
COPY docker/web-audio.js /usr/local/share/lanthorn/web-audio.js
COPY docker/entrypoint.sh /usr/local/bin/lanthorn-entrypoint
COPY docker/serve-session.sh /usr/local/bin/lanthorn-serve-session

# /data is $HOME (saves, config, archives under /data/.lanthorn); /stories is
# the library the picker opens on. Both are meant to be volume-mounted.
RUN useradd --uid 1000 --create-home --home-dir /data lanthorn \
    && mkdir -p /stories \
    && chown lanthorn:lanthorn /stories \
    && chmod +x /usr/local/bin/lanthorn-entrypoint /usr/local/bin/lanthorn-serve-session

USER lanthorn
ENV HOME=/data \
    TERM=xterm-256color \
    LANTHORN_WEB_PORT=7681 \
    LANTHORN_WEB_AUDIO_PORT=7682

VOLUME ["/data", "/stories"]
EXPOSE 7681 7682

ENTRYPOINT ["/usr/local/bin/lanthorn-entrypoint"]
# Default: open the story picker on the library mount. Replace with `serve`
# (plus optional lanthorn args) for the browser-facing web terminal, or with
# any lanthorn arguments (a story path, --help, ...) for direct play.
CMD ["/stories"]

# Corten build and test container
#
# Builds Corten from source and provides an environment for testing.
# Must be run with --privileged for container operations.
#
# Build:  docker build -t corten .
# Run:    docker run --privileged -it corten
# Test:   docker run --privileged corten ./scripts/smoke-test.sh

FROM rust:1.85-bookworm AS builder

WORKDIR /build
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    iproute2 iptables procps \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/corten /usr/local/bin/corten
COPY --from=builder /build/scripts/ /opt/corten/scripts/
COPY --from=builder /build/examples/ /opt/corten/examples/

# Create data directories
RUN mkdir -p /var/lib/corten/images /var/lib/corten/containers

# Set capabilities
RUN setcap 'cap_sys_admin,cap_net_admin,cap_sys_chroot,cap_dac_override,cap_fowner,cap_setuid,cap_setgid,cap_mknod+eip' /usr/local/bin/corten || true

WORKDIR /opt/corten
ENV CORTEN=/usr/local/bin/corten

CMD ["/bin/bash"]

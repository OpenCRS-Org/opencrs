# Container image for the opencrs CLI. FROM scratch: a static musl binary plus
# CA certificates (needed by `opencrs fetch`). The release workflow stages the
# binaries as ctx/amd64/opencrs and ctx/arm64/opencrs.

# CA certificates, from a pinned Alpine build stage (runs on the build host).
FROM --platform=$BUILDPLATFORM alpine:3.22@sha256:2e0b81d60597f449baa56dc25d355367869d2dca4cd57e866f1e2b52af3eb55f AS certs
RUN apk add --no-cache ca-certificates && mkdir /work

FROM scratch
ARG TARGETARCH
LABEL org.opencontainers.image.source="https://github.com/OpenCRS-Org/opencrs" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.title="opencrs" \
      org.opencontainers.image.description="Tooling for OpenCRS, the Open Creative Requirements Specification" \
      org.opencontainers.image.url="https://opencrs.dev"
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY ${TARGETARCH}/opencrs /opencrs
# The working directory, owned by the unprivileged user the CLI runs as.
# Mount files to check there; to write into a bind mount (fetch -o), run
# with --user "$(id -u):$(id -g)" so the files are yours.
COPY --from=certs --chown=65532:65532 /work /work
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
WORKDIR /work
USER 65532:65532
ENTRYPOINT ["/opencrs"]

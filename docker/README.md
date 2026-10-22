# Tari Ootle Docker image

This directory contains the Dockerfile that produces two container images:

- `ghcr.io/tari-project/ootle` (default target `runtime`): `tari_validator_node` and `tari_indexer`
- `ghcr.io/tari-project/ootle-walletd` (target `walletd`): `tari_ootle_walletd`
 See [`docs/docker-build-pipeline.md`](../docs/docker-build-pipeline.md)
for the full architecture, CI workflow, and troubleshooting reference.

## Quick build

```bash
DOCKER_BUILDKIT=1 docker build \
  -f docker/ootle.Dockerfile \
  -t ootle:local .

DOCKER_BUILDKIT=1 docker build \
  -f docker/ootle.Dockerfile \
  --target walletd \
  -t ootle-walletd:local .
```

The first build takes 15-25 minutes. Subsequent builds reuse the cargo-chef
and pnpm caches and are typically 30 seconds to 5 minutes.

## Quick run

The image ships the network node binaries, `tari_validator_node` and
`tari_indexer`, with no default `CMD`. Pick one when running:

```bash
docker run --rm ootle:local tari_validator_node --help
docker run --rm ootle:local tari_indexer --help
```

The wallet CLI, swarm and operations tools are not in either image; use the release binaries.

The container runs as the non-root user `tari` (uid 1000), with `tini`
as PID 1 for signal forwarding and zombie reaping.

## Wallet daemon image

`ghcr.io/tari-project/ootle-walletd` runs `tari_ootle_walletd` as its entrypoint, so any arguments
after the image name go to the wallet daemon. It is configured entirely through environment
variables; no config file needs to be mounted.

```bash
docker run -d --name walletd \
  -v walletd-data:/data \
  -p 127.0.0.1:5100:5100 \
  -e TARI_NETWORK=esmeralda \
  -e JRPC_LISTEN_ON=0.0.0.0:5100 \
  -e OOTLE_WALLETD_INDEXER_API_URL=https://ootle-indexer-a.tari.com/,https://ootle-indexer-b.tari.com/ \
  -e OOTLE_WALLETD_KEYRING_PASSWORD=<password> \
  ghcr.io/tari-project/ootle-walletd:vX.Y.Z

curl -fsS http://127.0.0.1:5100/health
```

| Variable | Flag | Default in the image | Meaning |
|---|---|---|---|
| `TARI_NETWORK` | `--network` | `esmeralda` | Network to join |
| `TARI_BASE_DIR` | `--base-path` / `-b` | `/data` | Data directory; the wallet uses `<base>/<network>/` |
| `JRPC_LISTEN_ON` | `--listen-on` | `127.0.0.1:5100` | JSON-RPC, web UI and `/health` listener. Set `0.0.0.0:<port>` only to publish the port outside the container's network namespace (`docker run -p`); containers in one ECS `awsvpc` task share loopback |
| `OOTLE_WALLETD_INDEXER_API_URL` | `--indexer-api-url` / `-i` | the network preset (Esmeralda: the two public Tari indexers) | Comma-separated indexer URLs; one is picked at random, the rest are failovers |
| `OOTLE_WALLETD_KEYRING_PASSWORD` | `--override-keyring-password` | none | Password that encrypts the wallet seed. **Required in a container** |
| `OOTLE_WALLETD_AUTHENTICATION` | `--authentication` | `webauthn` | `webauthn` or `none` |
| `OOTLE_WALLETD_AUTO_CLAIM_BURNS` | `--auto-claim-burns` | `true` | Claim burn proofs found in the burn proof directory |
| `OOTLE_WALLETD_MAX_FEE_LIMIT` | `--max-fee-limit` | `100000000` (100 TARI) | Largest `max_fee`, in microtari, the wallet will submit |

Any other `[ootle_wallet_daemon]` setting can be set with `-p <key>=<value>` after the image
name, or with an environment variable `TARI_<SECTION>__<KEY>` (double underscore between
levels), e.g. `TARI_ESMERALDA__OOTLE_WALLET_DAEMON__JWT_EXPIRY=15m`. Use the network-scoped
key (`esmeralda.ootle_wallet_daemon.<key>`): a value in the network section of the config
file wins over the unscoped `ootle_wallet_daemon.<key>`. The flags and `OOTLE_WALLETD_*`
variables above win over the config file. One exception: indexer URLs saved over JSON-RPC with
`settings.set` are stored in the wallet database and win over `OOTLE_WALLETD_INDEXER_API_URL`.

### First start

On first start the daemon writes a default `config.toml` to `/data/<network>/config/`, creates
the wallet database at `/data/<network>/data/wallet.sqlite` and generates a new seed. Logs go to
`/data/<network>/log/` and stdout. Keep `/data` on a persistent volume: losing it loses the
wallet unless its seed words were backed up.

The container runs as uid/gid 1000 and never as root, so `/data` must be writable by uid 1000.
A Docker named volume inherits the image's ownership and works as is. A host bind mount or an
attached EBS volume is usually owned by root: `chown 1000:1000` the directory on the host (or in
an ECS init container) before the first start. Otherwise the daemon exits immediately with a
`Permission denied` error while creating `/data/<network>/config/config.toml`.

There is no OS keyring inside a container, so `OOTLE_WALLETD_KEYRING_PASSWORD` must be set;
without it the daemon cannot store the seed password and exits with a keyring access error. The seed is encrypted
with this password, so the same password must be given on every later start.

### Running headless in ECS

- Inject `OOTLE_WALLETD_KEYRING_PASSWORD` through the task definition's `secrets` (`valueFrom` a
  Secrets Manager secret or SSM SecureString), not `environment`: plain environment values show
  in the task definition and in `docker inspect`. Put `/data` on an encrypted EBS volume.
- `webauthn` needs a browser to register a passkey. A headless wallet can use
  `OOTLE_WALLETD_AUTHENTICATION=none`, but then anything that reaches the port can spend the
  wallet's funds, including a web page through DNS rebinding when the port is published on a
  desktop. Keep it on loopback inside the task (the image default).
- For a cold and a hot wallet in one task, give each its own `JRPC_LISTEN_ON` port (e.g.
  `127.0.0.1:5100` and `127.0.0.1:5101`) and its own health-check port. If the port is taken the
  daemon silently binds a random port instead, so two wallets on 5100 conflict.

Burn proofs are read from `/data/share/tari/<network>/burn_proofs` (the image sets
`XDG_DATA_HOME=/data/share`), so pending proofs survive a container replacement.

### Health check

`GET /health` on the JSON-RPC port returns `200 OK` without authentication once the wallet has
started. The image has a `HEALTHCHECK` that calls it with `curl` on the port in
`JRPC_LISTEN_ON` (5100 when unset). ECS ignores the image `HEALTHCHECK`; set it in the task definition instead:

```json
"healthCheck": {
  "command": ["CMD-SHELL", "curl -fsS http://127.0.0.1:5100/health || exit 1"],
  "interval": 30,
  "timeout": 5,
  "retries": 3,
  "startPeriod": 60
}
```

## Pulling from GHCR

```bash
docker pull ghcr.io/tari-project/ootle:latest
docker pull ghcr.io/tari-project/ootle:development
docker pull ghcr.io/tari-project/ootle:nightly
docker pull ghcr.io/tari-project/ootle-walletd:development
```

Both images get the same tags. Released tags follow `vX.Y.Z` semver.

## Verifying signatures

CI signs each pushed image by digest with cosign keyless signing (GitHub OIDC). Verify a release
image against the release tag ref:

```bash
cosign verify ghcr.io/tari-project/ootle-walletd:vX.Y.Z \
  --certificate-identity-regexp '^https://github\.com/tari-project/tari-ootle/\.github/workflows/build_dockers\.yml@refs/tags/v' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

and a `development` image against the branch:

```bash
cosign verify ghcr.io/tari-project/ootle-walletd:development \
  --certificate-identity https://github.com/tari-project/tari-ootle/.github/workflows/build_dockers.yml@refs/heads/development \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

Tags can be moved; in ECS, pin the image by the verified digest (`ootle-walletd@sha256:...`).

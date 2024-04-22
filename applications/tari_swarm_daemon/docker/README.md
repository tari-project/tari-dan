# Docker Build Notes

Create a folder ```sources``` and build a docker image.

```bash
mkdir sources
cd sources
git clone https://github.com/tari-project/tari.git
git clone https://github.com/tari-project/tari-dan.git
git clone https://github.com/tari-project/tari-connector.git
cp -v applications/tari_swarm/docker/cross-compile-aarch64.sh .
docker build -f applications/tari_swarm/docker/tari_swarm.Dockerfile \
  -t local/tari-swarm .
```

# Targeted testing and cross platform builds

```bash
docker build -f tari_swarm/docker_rig/tari_swarm.Dockerfile \
  -t local/tari-swarm-tari-dan --target=builder-tari-dan .
```

or

```bash
docker buildx build -f tari_swarm/docker_rig/tari_swarm.Dockerfile \
  -t local/tari-swarm-tari-dan-arm64 --target=builder-tari-dan \
  --platform linux/arm64 .
```

# Docker Testing Notes

Launching the docker image with local ports redirected to docker container ports 18000 to 19000

```bash
docker run --rm -it -p 18000-19000:18000-19000 \
  quay.io/tarilabs/tari-swarm
```

Using the folder ```sources```, builds can be done with
the docker image.

```bash
docker run --rm -it -p 18000-19000:18000-19000 \
  -v $PWD/sources/:/home/tari/sources-build \
  quay.io/tarilabs/tari-swarm:development_20230704_790dbea \
  /bin/bash
```

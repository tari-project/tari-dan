# Docker Build Notes
Create a folder ```sources``` and build a docker image.
```bash
mkdir -p sources
cd sources
git clone https://github.com/tari-project/tari-dan.git
cd tari-dan
docker build -f docker_rig/tari-dan.Dockerfile \
  -t local/tari-dan:testing .
```

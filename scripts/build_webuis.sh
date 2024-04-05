#!/usr/bin/env bash

set -e

#
# Builds all webuis
#

function usage() {
  echo "Usage: $0 [-h|--help] [-t|--check-typescript] [-k|--skip-bindings]"
  echo "  -h|--help    This help"
  echo "  -t|--check-typescript    Check that typescript compiles without building"
  echo "  -k|--skip-bindings     Skip building bindings"
  exit 1
}

# Git base dir
base_path=$(git rev-parse --show-toplevel)

# Parse arguments
while [[ $# -gt 0 ]]; do
  key="$1"
  case $key in
    -h|--help)
      usage
      ;;
    -kt|-tk)
      check_typescript=true
      skip_bindings=true
      shift
      ;;
    -k|--skip-bindings)
      skip_bindings=true
      shift
      ;;
    -t|--check-typescript)
      check_typescript=true
      shift
      ;;
    *)
      echo "Unknown option: $key"
      usage
      ;;
  esac
done

# Build bindings
if [ -z "${skip_bindings}" ]; then
  echo "Building Bindings..."
  pushd $base_path/bindings > /dev/null
  npm install
  npm run build
  popd > /dev/null
fi

function build() {
  pushd $base_path/applications/$1 > /dev/null
  npm install > /dev/null
  if [ -z ${check_typescript+x} ]; then
    npm run build
  else
    npx tsc
    echo "✅ Typescript compiled successfully"
  fi
  popd > /dev/null
}

# Build webuis
echo "Building Validator Node Web UI..."
build tari_validator_node_web_ui


echo "Building Wallet Web UI..."
build tari_dan_wallet_web_ui


echo "Building Indexer Web UI..."
build tari_indexer_web_ui
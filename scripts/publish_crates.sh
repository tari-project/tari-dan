#!/usr/bin/env bash
# NB: The order these are listed in is IMPORTANT! Dependencies must go first

packages=${@:-'
dan_layer/template_lib
'}
p_arr=($packages)

function build_package {
    list=($@)
    for p in "${list[@]}"; do
      echo "************************  Building $path/$p package ************************"
      cargo publish --manifest-path=./${p}/Cargo.toml
      sleep 30 # Wait for crates.io to register any dependent packages
    done
    echo "************************  $path packages built ************************"
}

# You need a token with write access to publish these crates
#cargo login
build_package ${p_arr[@]}

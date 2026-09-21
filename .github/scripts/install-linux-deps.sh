#!/usr/bin/env bash
set -euo pipefail

# Zig 0.16 uses libc++ 21 headers; Ubuntu's default runtime is older.
curl --fail --silent --show-error --location https://apt.llvm.org/llvm-snapshot.gpg.key \
  | sudo tee /usr/share/keyrings/apt.llvm.org.asc >/dev/null
echo 'deb [signed-by=/usr/share/keyrings/apt.llvm.org.asc] https://apt.llvm.org/noble/ llvm-toolchain-noble-21 main' \
  | sudo tee /etc/apt/sources.list.d/llvm-21.list >/dev/null
sudo apt-get update
sudo apt-get install -y \
  clang cmake pkg-config libclang-dev libc++-21-dev libc++abi-21-dev \
  libasound2-dev libfontconfig-dev libfreetype-dev libxml2-dev \
  libegl1-mesa-dev libgl1-mesa-dev libwayland-dev \
  libx11-xcb-dev libxkbcommon-x11-dev libvulkan-dev libssl-dev

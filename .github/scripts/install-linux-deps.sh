#!/usr/bin/env bash
set -euo pipefail

# Zig 0.17 uses libc++ 22 headers; Ubuntu's default runtime is older.
curl --fail --silent --show-error --location https://apt.llvm.org/llvm-snapshot.gpg.key \
  | sudo tee /usr/share/keyrings/apt.llvm.org.asc >/dev/null
echo 'deb [signed-by=/usr/share/keyrings/apt.llvm.org.asc] https://apt.llvm.org/noble/ llvm-toolchain-noble-22 main' \
  | sudo tee /etc/apt/sources.list.d/llvm-22.list >/dev/null
sudo apt-get update
sudo apt-get install -y \
  clang cmake pkg-config libclang-dev libc++-22-dev libc++abi-22-dev \
  libasound2-dev libfontconfig-dev libfreetype-dev libxml2-dev \
  libegl1-mesa-dev libgl1-mesa-dev libwayland-dev \
  libx11-xcb-dev libxkbcommon-x11-dev libvulkan-dev libssl-dev

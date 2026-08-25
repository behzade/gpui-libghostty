{
  description = "gpui-libghostty development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    zig-overlay = {
      url = "github:mitchellh/zig-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      zig-overlay,
      ...
    }:
    let
      inherit (nixpkgs) lib;
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
    in
    {
      devShells = lib.genAttrs systems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          zig = zig-overlay.packages.${system}."0.16.0";
          linuxLibraries = with pkgs; [
            fontconfig
            freetype
            libGL
            libxkbcommon
            llvmPackages.libcxx
            wayland
          ];
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              git
              neovim
              pkg-config
              rust-analyzer
              rustc
              rustfmt
              zig
            ];
            nativeBuildInputs = [ pkgs.rustPlatform.bindgenHook ];
            buildInputs = lib.optionals pkgs.stdenv.hostPlatform.isLinux linuxLibraries;
            shellHook = lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              export LD_LIBRARY_PATH="${lib.makeLibraryPath linuxLibraries}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
            '';
          };
        }
      );
    };
}

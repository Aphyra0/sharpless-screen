{
  description = "Global rounded screen corners for Wayland, in pure Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/20b1ddd1aa5ace70c9468305030aa4f9ef79671b";
  };

  outputs =
    { nixpkgs, self }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems f;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          # buildRustPackage splits the build into a `cargoDeps` derivation
          # hashed only on Cargo.lock plus the app derivation itself, so
          # dependency objects stay in the store across rebuilds that only
          # touch src/. naersk's `-deps` derivation hashes the full source
          # tree, forcing a from-scratch recompile of every crate on any edit.
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "wlr-screen-corners";
            version = "0.1.0";
            src = pkgs.lib.cleanSourceWith {
              src = ./.;
              filter =
                path: type:
                !pkgs.lib.hasInfix ".crush" path
                && path != toString ./result
                && path != toString ./target;
            };

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ pkgs.wayland ];

            meta = with pkgs.lib; {
              description = "Draw black rounded corners at the edges of the screen on Wayland";
              license = licenses.mit;
              platforms = platforms.linux;
              mainProgram = "wlr-screen-corners";
            };
          };
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = [
              pkgs.cargo
              pkgs.rustc
              pkgs.clippy
              pkgs.rustfmt
              pkgs.pkg-config
              pkgs.wayland
              pkgs.wayland-protocols
            ];
          };
        }
      );
    };
}

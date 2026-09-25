{
  description = "Global rounded screen corners for Wayland, in pure Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/20b1ddd1aa5ace70c9468305030aa4f9ef79671b";
    # Binary rust distribution (rustup profiles) instead of nixpkgs' from-source
    # rustc: ~1.2 GB devshell closure instead of ~2 GB.
    rust-overlay.url = "github:oxalica/rust-overlay/84fb3c79e477d5e7ec4eff9c24dbc5d5c99b0fd1";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    { self, nixpkgs, rust-overlay }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems f;
      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          # Same binary toolchain as the dev shell (upstream dist tarballs,
          # rustc+cargo+rust-std in one prefix) wired into buildRustPackage
          # via makeRustPlatform, instead of nixpkgs' from-source rustc.
          toolchain = pkgs.rust-bin.stable.latest.minimal;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
        in
        {
          # buildRustPackage splits the build into a `cargoDeps` derivation
          # hashed only on Cargo.lock plus the app derivation itself, so
          # dependency objects stay in the store across rebuilds that only
          # touch src/.
          default = rustPlatform.buildRustPackage {
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
            auditable = false;

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
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.mkShell {
            packages = [
              # rustup "minimal" profile (rustc, cargo, rust-std) plus the two
              # dev tools; binaries come from upstream's dist tarballs.
              (pkgs.rust-bin.stable.latest.minimal.override {
                extensions = [
                  "clippy"
                  "rustfmt"
                ];
              })
              pkgs.pkg-config
              pkgs.wayland
              pkgs.wayland-protocols
            ];
          };
        }
      );
    };
}

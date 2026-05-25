{
  description = "todone — persistent work tracking TUI";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in
    {
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "todone";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        preCheck = ''
          export TZ=":${pkgs.tzdata}/share/zoneinfo/Australia/Melbourne"
        '';
        meta = {
          description = "Persistent work tracking TUI";
          mainProgram = "todone";
        };
      };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          rustc
          cargo
          rustfmt
          clippy
          rust-analyzer
          pkg-config
        ];
      };
    };
}

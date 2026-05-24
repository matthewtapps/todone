{
  description = "standup — daily dot-points TUI";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in
    {
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "standup";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        nativeBuildInputs = [ pkgs.makeWrapper ];
        postInstall = ''
          wrapProgram $out/bin/standup \
            --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.wl-clipboard ]}
        '';
        meta = {
          description = "Daily dot-points TUI for standups";
          mainProgram = "standup";
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
          wl-clipboard
        ];
      };
    };
}

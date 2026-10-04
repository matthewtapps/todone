{
  description = "todone — persistent work tracking TUI";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      web = pkgs.buildNpmPackage {
        pname = "todone-web";
        version = "1.1.0";
        src = ./web;
        npmDepsHash = "sha256-CTWvM/VTm82fJEYoUf/alPbYhBxCyHJ13u12tQHdYJw=";
        installPhase = "cp -r dist $out";
      };
      # Developer commands, on PATH in `nix develop`. Add a case to extend it.
      devtools = pkgs.writeShellApplication {
        name = "devtools";
        runtimeInputs = with pkgs; [
          cargo
          rustc
          stdenv.cc
          pkg-config
          nodejs
          git
        ];
        text = ''
          usage() {
            cat <<EOF
          Usage: devtools <command> [args]

          Commands:
            dev [todone args]  Run todone from source; the timesheet web app rebuilds on save.
                               Set TODONE_TIMESHEET_PORT to run beside another todone.
            help               Show this help.
          EOF
          }

          root() {
            cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
            if [ ! -f web/package.json ]; then
              echo "devtools: run this from the todone repo" >&2
              exit 1
            fi
          }

          # Debug builds read web/dist from disk at runtime, so a watching Vite build
          # shows web changes on a browser refresh without recompiling todone.
          dev() {
            root
            if [ ! -d web/node_modules ] || [ web/package-lock.json -nt web/node_modules ]; then
              (cd web && npm ci)
            fi
            (cd web && npm run build)

            log="$(mktemp -t todone-web.XXXXXX.log)"
            (cd web && exec npx vite build --watch >"$log" 2>&1) &
            watcher=$!
            trap 'kill "$watcher" 2>/dev/null' EXIT
            echo "devtools: web rebuilds on save (log: $log); refresh the browser to see changes"

            cargo run -- "$@"
          }

          cmd="''${1:-help}"
          shift || true
          case "$cmd" in
            dev) dev "$@" ;;
            help | -h | --help) usage ;;
            *)
              echo "devtools: unknown command '$cmd'" >&2
              usage >&2
              exit 1
              ;;
          esac
        '';
      };
    in
    {
      packages.${system} = {
        inherit web;
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "todone";
          version = "1.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          # The timesheet SPA is embedded into the binary at compile time.
          preBuild = ''
            cp -r ${web} web/dist
          '';
          preCheck = ''
            export TZ=":${pkgs.tzdata}/share/zoneinfo/Australia/Melbourne"
          '';
          meta = {
            description = "Persistent work tracking TUI";
            mainProgram = "todone";
          };
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
          nodejs
          devtools
        ];
      };
    };
}

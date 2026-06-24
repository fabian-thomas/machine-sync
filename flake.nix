{
  description = "msync: live-sync a directory to one or more machines over rsync/ssh";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        lib = pkgs.lib;

        # rsync and ssh are needed at runtime. They are added as a PATH *suffix* so
        # the user's own system rsync/ssh take precedence when present (preserving
        # their ssh config, agent, key types/FIDO support and version); the packaged
        # ones are only a fallback on systems that lack them.
        runtimeDeps = [ pkgs.rsync pkgs.openssh ];

        mkMsync = rustPlatform:
          rustPlatform.buildRustPackage {
            pname = "msync";
            version = "2.0.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            postInstall = ''
              wrapProgram $out/bin/msync \
                --suffix PATH : ${lib.makeBinPath runtimeDeps}
            '';
            meta = {
              description = "Live-sync a directory to one or more machines over rsync/ssh";
              mainProgram = "msync";
              license = lib.licenses.mit;
            };
          };

        msync = mkMsync pkgs.rustPlatform;

        # Fully static binary via the musl target.
        msync-static = mkMsync pkgs.pkgsStatic.rustPlatform;
      in
      {
        packages = {
          default = msync;
          msync = msync;
          static = msync-static;
        };

        apps.default = {
          type = "app";
          program = "${msync}/bin/msync";
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [
            pkgs.cargo
            pkgs.rustc
            pkgs.clippy
            pkgs.rustfmt
            pkgs.rust-analyzer
            pkgs.rsync
            pkgs.openssh
          ];
        };
      });
}

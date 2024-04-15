{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=release-23.11";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let pkgs = nixpkgs.legacyPackages.${system}; in
      {
        packages = rec {
          msync = (pkgs.callPackage ./default.nix { inherit rsync-notify-multiple; });
          rsync-notify-multiple = (pkgs.callPackage ./rsync-notify-multiple.nix {});
          default = msync;
        };
      }
  );
}

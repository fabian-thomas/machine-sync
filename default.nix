{ stdenv, callPackage, lib, makeWrapper, coreutils, findutils, lsyncd, ps, rsync-notify-multiple ? (callPackage ./rsync-notify-multiple.nix {}) }:

stdenv.mkDerivation {
  pname = "msync";
  version = "1.0";
  src = ./.;

  meta.mainProgram = "msync";

  installPhase = ''
    mkdir -p $out/bin
    mkdir -p $out/share
    cp $src/msync $out/bin/
    # TODO: /share/ ??? but then it doesnt work outside of derivation
    cp $src/template.lua $out/bin/
  '';

  buildInputs = [ makeWrapper ];
  postFixup = "wrapProgram $out/bin/msync --set PATH ${lib.makeBinPath [coreutils findutils lsyncd ps rsync-notify-multiple]}";
}

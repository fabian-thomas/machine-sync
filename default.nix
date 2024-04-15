{ stdenv, callPackage, lib, makeWrapper, coreutils, findutils, lsyncd, ps, rsync-notify-multiple ? (callPackage ./rsync-notify-multiple.nix {}) }:

stdenv.mkDerivation {
  pname = "msync";
  version = "1.0";
  src = ./.;

  meta.mainProgram = "msync";

  installPhase = ''
    mkdir -p $out/bin $out/share
    cp $src/msync $out/bin/
    cp $src/template.lua $out/share/
  '';

  buildInputs = [ makeWrapper ];
  postFixup = "wrapProgram $out/bin/msync --set PATH ${lib.makeBinPath [coreutils findutils lsyncd ps rsync-notify-multiple]} --set LSYNCD_TEMPLATE $out/share/template.lua";
}

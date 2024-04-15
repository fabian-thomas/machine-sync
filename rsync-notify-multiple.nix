{ stdenv, lib, makeWrapper, coreutils, libnotify, rsync, openssh }:

stdenv.mkDerivation {
  pname = "rsync-notify-multiple";
  version = "1.0";
  src = ./.;
  installPhase = ''
    mkdir -p $out/bin
    cp $src/rsync-notify-multiple $out/bin/
  '';
  buildInputs = [ makeWrapper ];
  postFixup = "wrapProgram $out/bin/rsync-notify-multiple --set PATH ${lib.makeBinPath [libnotify coreutils rsync openssh]}";
}

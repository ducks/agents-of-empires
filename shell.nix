let
  nixpkgs =
    builtins.fetchTarball {
      url =
        "https://github.com/NixOS/nixpkgs/archive/2fcb964de67fcf60b43471c55d5d99e61a9ccb5a.tar.gz";
      sha256 = "sha256-RzPPiWeUtuvymnpuEWsdtzli5w4kjZs49FqEs3/1u+I=";
    };
  pkgs = import nixpkgs { };
in
pkgs.mkShell {
  packages = with pkgs; [
    bash
    cargo
    clippy
    cmake
    git
    nix
    openssh
    openssl
    pkg-config
    python3
    qemu_kvm
    rust-analyzer
    rustc
    rustfmt
  ];

  NIX_CONFIG = "experimental-features = nix-command flakes";
  RUST_BACKTRACE = "1";

  shellHook = ''
    echo "Agents of Empires development shell"
    echo "Rust: $(rustc --version)"
  '';
}

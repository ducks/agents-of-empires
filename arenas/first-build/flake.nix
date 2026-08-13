{
  description = "Agents of Empires first build race";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      mkBuilder = name: address: mac: password: nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [ ./nix/base.nix {
          arena.territory = name;
          arena.arenaAddress = address;
          arena.arenaMac = mac;
          arena.password = password;
        } ];
      };
    in {
      nixosConfigurations = {
        builder-one = mkBuilder "builder-one" "10.78.0.10" "52:54:00:78:00:0a" "builder-one-race";
        builder-two = mkBuilder "builder-two" "10.78.0.11" "52:54:00:78:00:0b" "builder-two-race";
        builder-three = mkBuilder "builder-three" "10.78.0.12" "52:54:00:78:00:0c" "builder-three-race";
      };
      checks.${system} = {
        builder-one = self.nixosConfigurations.builder-one.config.system.build.toplevel;
        builder-two = self.nixosConfigurations.builder-two.config.system.build.toplevel;
        builder-three = self.nixosConfigurations.builder-three.config.system.build.toplevel;
      };
    };
}

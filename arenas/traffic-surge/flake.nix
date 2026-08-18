{
  description = "Agents of Empires traffic surge race";
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
        surge-one = mkBuilder "surge-one" "10.82.0.10" "52:54:00:82:00:0a" "surge-one-race";
        surge-two = mkBuilder "surge-two" "10.82.0.11" "52:54:00:82:00:0b" "surge-two-race";
        surge-three = mkBuilder "surge-three" "10.82.0.12" "52:54:00:82:00:0c" "surge-three-race";
      };
      checks.${system} = {
        surge-one = self.nixosConfigurations.surge-one.config.system.build.toplevel;
        surge-two = self.nixosConfigurations.surge-two.config.system.build.toplevel;
        surge-three = self.nixosConfigurations.surge-three.config.system.build.toplevel;
      };
    };
}

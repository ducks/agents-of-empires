{
  description = "Agents of Empires zero-downtime rollout race";
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
        rollout-one = mkBuilder "rollout-one" "10.80.0.10" "52:54:00:80:00:0a" "rollout-one-race";
        rollout-two = mkBuilder "rollout-two" "10.80.0.11" "52:54:00:80:00:0b" "rollout-two-race";
        rollout-three = mkBuilder "rollout-three" "10.80.0.12" "52:54:00:80:00:0c" "rollout-three-race";
      };
      checks.${system} = {
        rollout-one = self.nixosConfigurations.rollout-one.config.system.build.toplevel;
        rollout-two = self.nixosConfigurations.rollout-two.config.system.build.toplevel;
        rollout-three = self.nixosConfigurations.rollout-three.config.system.build.toplevel;
      };
    };
}

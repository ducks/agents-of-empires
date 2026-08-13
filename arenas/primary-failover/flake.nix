{
  description = "Agents of Empires stateful primary failover race";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      mkHost = name: address: mac: password: nixpkgs.lib.nixosSystem {
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
        failover-one = mkHost "failover-one" "10.81.0.10" "52:54:00:81:00:0a" "failover-one-race";
        failover-two = mkHost "failover-two" "10.81.0.11" "52:54:00:81:00:0b" "failover-two-race";
        failover-three = mkHost "failover-three" "10.81.0.12" "52:54:00:81:00:0c" "failover-three-race";
      };
      checks.${system} = {
        failover-one = self.nixosConfigurations.failover-one.config.system.build.toplevel;
        failover-two = self.nixosConfigurations.failover-two.config.system.build.toplevel;
        failover-three = self.nixosConfigurations.failover-three.config.system.build.toplevel;
      };
    };
}

{
  description = "Agents of Empires durable job queue race";
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
        queue-one = mkBuilder "queue-one" "10.79.0.10" "52:54:00:79:00:0a" "queue-one-race";
        queue-two = mkBuilder "queue-two" "10.79.0.11" "52:54:00:79:00:0b" "queue-two-race";
        queue-three = mkBuilder "queue-three" "10.79.0.12" "52:54:00:79:00:0c" "queue-three-race";
      };
      checks.${system} = {
        queue-one = self.nixosConfigurations.queue-one.config.system.build.toplevel;
        queue-two = self.nixosConfigurations.queue-two.config.system.build.toplevel;
        queue-three = self.nixosConfigurations.queue-three.config.system.build.toplevel;
      };
    };
}

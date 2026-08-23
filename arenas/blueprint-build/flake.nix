{
  description = "Agents of Empires blueprint build race";
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
        blueprint-one = mkBuilder "blueprint-one" "10.84.0.10" "52:54:00:84:00:0a" "blueprint-one-race";
        blueprint-two = mkBuilder "blueprint-two" "10.84.0.11" "52:54:00:84:00:0b" "blueprint-two-race";
        blueprint-three = mkBuilder "blueprint-three" "10.84.0.12" "52:54:00:84:00:0c" "blueprint-three-race";
      };
      checks.${system} = {
        blueprint-one = self.nixosConfigurations.blueprint-one.config.system.build.toplevel;
        blueprint-two = self.nixosConfigurations.blueprint-two.config.system.build.toplevel;
        blueprint-three = self.nixosConfigurations.blueprint-three.config.system.build.toplevel;
      };
    };
}

{
  description = "Agents of Empires first-contact arena";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      mkTerritory = module: nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [ ./nix/base.nix module ];
      };
    in
    {
      nixosConfigurations = {
        gatekeeper = mkTerritory ./nix/gatekeeper.nix;
        archivist = mkTerritory ./nix/archivist.nix;
        courier = mkTerritory ./nix/courier.nix;
      };

      checks.${system} = {
        gatekeeper = self.nixosConfigurations.gatekeeper.config.system.build.toplevel;
        archivist = self.nixosConfigurations.archivist.config.system.build.toplevel;
        courier = self.nixosConfigurations.courier.config.system.build.toplevel;
      };
    };
}

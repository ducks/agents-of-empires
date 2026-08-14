{ lib, pkgs, config, modulesPath, ... }:

{
  imports = [ "${modulesPath}/virtualisation/qemu-vm.nix" ];

  options.arena = {
    territory = lib.mkOption { type = lib.types.str; };
    arenaAddress = lib.mkOption { type = lib.types.str; };
    arenaMac = lib.mkOption { type = lib.types.str; };
    password = lib.mkOption { type = lib.types.str; };
  };

  config = {
    system.stateVersion = "24.11";
    documentation.enable = false;
    documentation.nixos.enable = false;
    programs.command-not-found.enable = false;
    programs.nix-ld.enable = true;
    networking.hostName = config.arena.territory;
    networking.useDHCP = false;
    networking.useNetworkd = true;
    networking.firewall.allowedTCPPorts = [ 8080 ];
    systemd.network.enable = true;
    systemd.network.networks = {
      "10-arena" = {
        matchConfig.MACAddress = config.arena.arenaMac;
        address = [ "${config.arena.arenaAddress}/24" ];
        networkConfig.LinkLocalAddressing = "no";
      };
      "20-management" = {
        matchConfig.Name = "en*";
        networkConfig.DHCP = "ipv4";
      };
    };
    services.openssh = {
      enable = true;
      settings = {
        PermitRootLogin = "yes";
        PasswordAuthentication = true;
      };
    };
    users.users.root.initialPassword = config.arena.password;
    environment.systemPackages = with pkgs; [ curl jq procps python3 vim ];
    systemd.services.arena-app = {
      description = "Agent-built application slot";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      serviceConfig = {
        ExecStart = "${pkgs.python3}/bin/python /opt/arena/app.py";
        Restart = "always";
        RestartSec = 1;
      };
    };
    virtualisation = {
      cores = 1;
      memorySize = 768;
      diskSize = 2048;
      graphics = false;
    };
  };
}

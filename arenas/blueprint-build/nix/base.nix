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
    networking.firewall = { enable = true; allowedTCPPorts = [ 8080 ]; };
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
      settings = { PermitRootLogin = "yes"; PasswordAuthentication = true; };
    };
    users.users.root.initialPassword = config.arena.password;
    environment.systemPackages = with pkgs; [ curl jq lsof procps python3 sqlite strace vim ];
    systemd.tmpfiles.rules = [
      "d /opt/blueprint 0755 root root -"
      "d /var/lib/blueprint 0755 root root -"
    ];

    systemd.services.blueprint-edge = {
      description = "Player-defined blueprint edge gateway";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" "blueprint-api.service" ];
      unitConfig.ConditionPathIsExecutable = "/opt/blueprint/edge";
      serviceConfig = {
        ExecStart = "/opt/blueprint/edge";
        Restart = "always";
        RestartSec = 1;
      };
    };
    systemd.services.blueprint-api = {
      description = "Player-defined blueprint API receiver";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      unitConfig.ConditionPathIsExecutable = "/opt/blueprint/api";
      serviceConfig = {
        ExecStart = "/opt/blueprint/api";
        Restart = "always";
        RestartSec = 1;
      };
    };
    systemd.services.blueprint-worker = {
      description = "Player-defined blueprint worker";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      unitConfig.ConditionPathIsExecutable = "/opt/blueprint/worker";
      serviceConfig = {
        ExecStart = "/opt/blueprint/worker";
        Restart = "always";
        RestartSec = 1;
      };
    };
    virtualisation = { cores = 1; memorySize = 768; diskSize = 2048; graphics = false; };
  };
}

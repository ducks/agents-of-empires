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

    environment.systemPackages = with pkgs; [
      curl
      jq
      lsof
      procps
      python3
      sqlite
      strace
      vim
    ];

    # The guests begin without an application, but NixOS does not permit an
    # agent to create unit files under /etc/systemd/system at runtime. Give
    # every competitor the same dormant deployment slot instead. It only
    # becomes healthy after the agent supplies /opt/builder/app.py.
    systemd.services.builder-app = {
      description = "Agent-built application slot";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      serviceConfig = {
        ExecStart = "${pkgs.python3}/bin/python /opt/builder/app.py";
        Restart = "always";
        RestartSec = 1;
      };
    };

    networking.firewall.enable = true;
    virtualisation = {
      cores = 1;
      memorySize = 768;
      diskSize = 2048;
      graphics = false;
    };
  };
}

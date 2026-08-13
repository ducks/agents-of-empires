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
      dig
      iproute2
      jq
      lsof
      nmap
      procps
      strace
      tcpdump
      vim
    ];

    networking.firewall.enable = true;
    virtualisation = {
      cores = 1;
      memorySize = 1024;
      diskSize = 3072;
      graphics = false;
    };
  };
}

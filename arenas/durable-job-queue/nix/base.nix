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
    systemd.tmpfiles.rules = [ "d /etc/replaybook 0755 root root -" ];

    systemd.services.accepted-job-spool = {
      description = "Preserve opaque jobs accepted before the deployment";
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };
      script = ''
        install -d -m 0755 /var/lib/accepted-jobs
        ${pkgs.python3}/bin/python - <<'PY'
        import json
        from pathlib import Path
        for job, payload in [('accepted-alpha-7d3', 'alpha'), ('accepted-beta-91e', 'beta'), ('accepted-gamma-c42', 'gamma')]:
            path = Path('/var/lib/accepted-jobs') / f'{job}.json'
            if not path.exists():
                path.write_text(json.dumps({'id': job, 'payload': payload, 'status': 'accepted'}))
        PY
      '';
    };
    systemd.services.job-api = {
      description = "Player-defined job API lifecycle slot";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" "accepted-job-spool.service" ];
      requires = [ "accepted-job-spool.service" ];
      unitConfig.ConditionPathExists = "/etc/replaybook/job-api-start";
      serviceConfig = {
        ExecStart = "/bin/sh /etc/replaybook/job-api-start";
        Restart = "always";
        RestartSec = 1;
      };
    };
    systemd.services.job-worker = {
      description = "Player-defined job worker lifecycle slot";
      wantedBy = [ "multi-user.target" ];
      after = [ "accepted-job-spool.service" ];
      requires = [ "accepted-job-spool.service" ];
      unitConfig.ConditionPathExists = "/etc/replaybook/job-worker-start";
      serviceConfig = {
        ExecStart = "/bin/sh /etc/replaybook/job-worker-start";
        Restart = "always";
        RestartSec = 1;
      };
    };
    virtualisation = { cores = 1; memorySize = 768; diskSize = 2048; graphics = false; };
  };
}

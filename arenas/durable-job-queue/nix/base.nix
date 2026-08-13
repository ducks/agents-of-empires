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

    systemd.services.queue-seed = {
      description = "Seed opaque jobs accepted before the deployment";
      wantedBy = [ "multi-user.target" ];
      before = [ "queue-api.service" "queue-worker.service" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };
      script = ''
        install -d -m 0755 /var/lib/job-queue
        ${pkgs.python3}/bin/python - <<'PY'
        import sqlite3
        db = sqlite3.connect('/var/lib/job-queue/jobs.sqlite3')
        db.execute('CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, payload TEXT NOT NULL, status TEXT NOT NULL, result TEXT, attempts INTEGER NOT NULL DEFAULT 0)')
        for job, payload in [('accepted-alpha-7d3', 'alpha'), ('accepted-beta-91e', 'beta'), ('accepted-gamma-c42', 'gamma')]:
            db.execute('INSERT OR IGNORE INTO jobs(id,payload,status,result,attempts) VALUES(?,?,\"queued\",NULL,0)', (job,payload))
        db.commit()
        PY
      '';
    };
    systemd.services.queue-api = {
      description = "Agent-built durable queue API slot";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" "queue-seed.service" ];
      requires = [ "queue-seed.service" ];
      serviceConfig = { ExecStart = "${pkgs.python3}/bin/python /opt/job-queue/api.py"; Restart = "always"; RestartSec = 1; };
    };
    systemd.services.queue-worker = {
      description = "Agent-built durable queue worker slot";
      wantedBy = [ "multi-user.target" ];
      after = [ "queue-seed.service" ];
      requires = [ "queue-seed.service" ];
      serviceConfig = { ExecStart = "${pkgs.python3}/bin/python /opt/job-queue/worker.py"; Restart = "always"; RestartSec = 1; };
    };
    virtualisation = { cores = 1; memorySize = 768; diskSize = 2048; graphics = false; };
  };
}

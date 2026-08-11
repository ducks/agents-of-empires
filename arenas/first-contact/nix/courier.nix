{ pkgs, ... }:

{
  arena = {
    territory = "courier";
    arenaAddress = "10.77.0.12";
    arenaMac = "52:54:00:77:00:0c";
    password = "courier-first-contact";
  };

  networking.firewall.allowedTCPPorts = [ 6379 8080 ];
  environment.systemPackages = [ pkgs.redis ];

  services.redis.servers.arena = {
    enable = true;
    bind = "0.0.0.0";
    port = 6379;
    openFirewall = true;
    settings.protected-mode = "no";
  };

  systemd.services.courier-worker = {
    description = "Courier heartbeat worker";
    wantedBy = [ "multi-user.target" ];
    after = [ "redis-arena.service" ];
    requires = [ "redis-arena.service" ];
    path = [ pkgs.redis pkgs.coreutils ];
    script = ''
      while true; do
        redis-cli SET empire:alive courier PX 5000 >/dev/null
        sleep 1
      done
    '';
    serviceConfig = {
      Restart = "always";
      RestartSec = 1;
    };
  };

  systemd.services.courier-app = {
    description = "Courier health service";
    wantedBy = [ "multi-user.target" ];
    after = [ "courier-worker.service" ];
    requires = [ "courier-worker.service" ];
    path = [ pkgs.redis ];
    serviceConfig = {
      ExecStart = "${pkgs.python3}/bin/python ${../services/courier.py}";
      Restart = "always";
      RestartSec = 1;
    };
  };
}

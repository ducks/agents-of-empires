{ lib, pkgs, ... }:

{
  arena = {
    territory = "archivist";
    arenaAddress = "10.77.0.11";
    arenaMac = "52:54:00:77:00:0b";
    password = "archivist-first-contact";
  };

  networking.firewall.allowedTCPPorts = [ 5432 8080 ];
  environment.systemPackages = [ pkgs.postgresql_16 ];

  services.postgresql = {
    enable = true;
    package = pkgs.postgresql_16;
    enableTCPIP = true;
    ensureDatabases = [ "empire" ];
    ensureUsers = [{ name = "arena"; }];
    authentication = lib.mkOverride 10 ''
      local all all trust
      host empire all 10.77.0.0/24 trust
      host all all 127.0.0.1/32 trust
    '';
  };

  systemd.services.archivist-provision = {
    description = "Provision Archivist durable state";
    wantedBy = [ "multi-user.target" ];
    after = [ "postgresql.service" ];
    requires = [ "postgresql.service" ];
    before = [ "archivist-app.service" ];
    path = [ pkgs.postgresql_16 ];
    serviceConfig.Type = "oneshot";
    script = ''
      psql --username postgres --dbname postgres --set ON_ERROR_STOP=1 --command \
        "ALTER DATABASE empire OWNER TO arena"
      psql --username arena --dbname empire --set ON_ERROR_STOP=1 --command \
        "CREATE TABLE IF NOT EXISTS empire_state (key text PRIMARY KEY, value text NOT NULL); INSERT INTO empire_state (key, value) VALUES ('status', 'archivist:ok') ON CONFLICT (key) DO NOTHING"
    '';
  };

  systemd.services.archivist-app = {
    description = "Archivist health service";
    wantedBy = [ "multi-user.target" ];
    after = [ "archivist-provision.service" ];
    requires = [ "archivist-provision.service" ];
    path = [ pkgs.postgresql_16 ];
    serviceConfig = {
      ExecStart = "${pkgs.python3}/bin/python ${../services/archivist.py}";
      Restart = "always";
      RestartSec = 1;
    };
  };
}

{ pkgs, ... }:

{
  arena = {
    territory = "gatekeeper";
    arenaAddress = "10.77.0.10";
    arenaMac = "52:54:00:77:00:0a";
    password = "gatekeeper-first-contact";
  };

  networking.firewall.allowedTCPPorts = [ 8080 ];
  systemd.tmpfiles.rules = [
    "d /var/lib/gatekeeper 0755 root root -"
    "f /var/lib/gatekeeper/route 0644 root root - checkout"
  ];

  systemd.services.gatekeeper-app = {
    description = "Gatekeeper routing control service";
    wantedBy = [ "multi-user.target" ];
    after = [ "network.target" ];
    serviceConfig = {
      ExecStart = "${pkgs.python3}/bin/python ${../services/gatekeeper.py}";
      Restart = "always";
      RestartSec = 1;
    };
  };

  services.nginx = {
    enable = true;
    virtualHosts.default = {
      default = true;
      listen = [{ addr = "0.0.0.0"; port = 8080; }];
      locations."/".proxyPass = "http://127.0.0.1:9000";
    };
  };
}

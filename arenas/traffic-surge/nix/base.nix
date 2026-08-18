{ lib, pkgs, config, modulesPath, ... }:
let
  initialApp = pkgs.writeText "traffic-surge-initial.py" ''
    #!/usr/bin/env python3
    import json
    import os
    import time
    from http.server import BaseHTTPRequestHandler, HTTPServer
    from pathlib import Path

    STATE = Path("/var/lib/traffic-surge/priority.json")

    def read_state():
        return json.loads(STATE.read_text())

    def write_state(state):
        temporary = STATE.with_suffix(".tmp")
        temporary.write_text(json.dumps(state, sort_keys=True))
        os.replace(temporary, STATE)

    class Handler(BaseHTTPRequestHandler):
        def reply(self, status, body=b""):
            self.send_response(status)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path == "/health":
                self.reply(200, b"ready")
            elif self.path.startswith("/priority/"):
                key = self.path[len("/priority/"):]
                value = read_state().get(key)
                if value is None:
                    self.reply(404)
                else:
                    self.reply(200, value.encode())
            elif self.path.startswith("/optional/"):
                key = self.path[len("/optional/"):]
                time.sleep(0.45)
                self.reply(200, ("optional:" + key).encode())
            else:
                self.reply(404)

        def do_PUT(self):
            if not self.path.startswith("/priority/"):
                self.reply(404)
                return
            key = self.path[len("/priority/"):]
            value = self.rfile.read(int(self.headers.get("Content-Length", "0"))).decode()
            state = read_state()
            state[key] = value
            write_state(state)
            self.reply(204)

        def log_message(self, *_):
            pass

    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
  '';
in {
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

    systemd.services.traffic-surge-seed = {
      description = "Seed historical priority traffic and editable service";
      wantedBy = [ "multi-user.target" ];
      before = [ "traffic-surge.service" ];
      serviceConfig = { Type = "oneshot"; RemainAfterExit = true; };
      script = ''
        install -d -m 0755 /opt/traffic-surge /var/lib/traffic-surge
        if [[ ! -e /opt/traffic-surge/app.py ]]; then
          cp ${initialApp} /opt/traffic-surge/app.py
          chmod 0755 /opt/traffic-surge/app.py
        fi
        if [[ ! -e /var/lib/traffic-surge/priority.json ]]; then
          printf '%s\n' '{"history-amber-a91":"ledger-alpha","history-cobalt-f73":"ledger-beta","history-umber-2d4":"ledger-gamma"}' > /var/lib/traffic-surge/priority.json
        fi
      '';
    };
    systemd.services.traffic-surge = {
      description = "Traffic surge service";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" "traffic-surge-seed.service" ];
      requires = [ "traffic-surge-seed.service" ];
      serviceConfig = {
        ExecStart = "/run/current-system/sw/bin/python3 /opt/traffic-surge/app.py";
        Restart = "always";
        RestartSec = 1;
      };
    };
    virtualisation = { cores = 1; memorySize = 768; diskSize = 2048; graphics = false; };
  };
}

{ lib, pkgs, config, modulesPath, ... }:
let
  v1 = pkgs.writeText "rollout-v1.py" ''
    import sqlite3
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    DB = "/var/lib/rollout/records.sqlite3"

    def database():
        connection = sqlite3.connect(DB, timeout=10)
        connection.execute("CREATE TABLE IF NOT EXISTS records (id TEXT PRIMARY KEY, value TEXT NOT NULL)")
        return connection

    class Handler(BaseHTTPRequestHandler):
        def send(self, status, body=b""):
            self.send_response(status)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path == "/health":
                self.send(200, b"ready")
                return
            if self.path == "/version":
                self.send(200, b"v1")
                return
            if self.path.startswith("/records/"):
                connection = database()
                row = connection.execute("SELECT value FROM records WHERE id=?", (self.path[9:],)).fetchone()
                connection.close()
                if row is None:
                    self.send(404)
                else:
                    self.send(200, row[0].encode())
                return
            self.send(404)

        def do_PUT(self):
            if not self.path.startswith("/records/"):
                self.send(404)
                return
            value = self.rfile.read(int(self.headers.get("Content-Length", "0"))).decode()
            connection = database()
            connection.execute(
                "INSERT INTO records(id,value) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET value=excluded.value",
                (self.path[9:], value),
            )
            connection.commit()
            connection.close()
            self.send(204)

        def log_message(self, *_):
            pass

    ThreadingHTTPServer(("127.0.0.1", 8081), Handler).serve_forever()
  '';

  proxy = pkgs.writeText "rollout-proxy.py" ''
    import http.client
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    UPSTREAM = "/var/lib/rollout/upstream"

    class Handler(BaseHTTPRequestHandler):
        def forward(self):
            try:
                with open(UPSTREAM) as source:
                    port = int(source.read().strip())
                body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
                connection.request(self.command, self.path, body=body)
                response = connection.getresponse()
                response_body = response.read()
                self.send_response(response.status)
                self.send_header("Content-Length", str(len(response_body)))
                self.end_headers()
                self.wfile.write(response_body)
                connection.close()
            except Exception:
                self.send_response(502)
                self.send_header("Content-Length", "0")
                self.end_headers()

        do_GET = forward
        do_PUT = forward

        def log_message(self, *_):
            pass

    ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
  '';
in
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
    networking.firewall = {
      enable = true;
      allowedTCPPorts = [ 8080 ];
    };

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
    environment.systemPackages = with pkgs; [ curl jq lsof procps python3 sqlite strace vim ];

    systemd.services.rollout-seed = {
      description = "Seed live v1 state";
      wantedBy = [ "multi-user.target" ];
      before = [ "rollout-v1.service" "rollout-v2.service" "rollout-proxy.service" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };
      script = ''
        install -d -m 0755 /var/lib/rollout /opt/rollout
        if [[ ! -e /var/lib/rollout/upstream ]]; then
          printf '8081\n' > /var/lib/rollout/upstream
        fi
        ${pkgs.python3}/bin/python - <<'PY'
        import sqlite3
        db = sqlite3.connect('/var/lib/rollout/records.sqlite3')
        db.execute('CREATE TABLE IF NOT EXISTS records (id TEXT PRIMARY KEY, value TEXT NOT NULL)')
        for record, value in [('customer-alpha-73c', 'alpha-original'), ('customer-beta-a19', 'beta-original')]:
            db.execute('INSERT OR IGNORE INTO records(id,value) VALUES(?,?)', (record,value))
        db.commit()
        PY
      '';
    };

    systemd.services.rollout-v1 = {
      description = "Existing rollout v1 service";
      wantedBy = [ "multi-user.target" ];
      after = [ "rollout-seed.service" ];
      requires = [ "rollout-seed.service" ];
      serviceConfig = {
        ExecStart = "${pkgs.python3}/bin/python ${v1}";
        Restart = "always";
        RestartSec = 1;
      };
    };

    systemd.services.rollout-v2 = {
      description = "Agent-built rollout v2 slot";
      wantedBy = [ "multi-user.target" ];
      after = [ "rollout-seed.service" ];
      requires = [ "rollout-seed.service" ];
      serviceConfig = {
        ExecStart = "${pkgs.python3}/bin/python /opt/rollout/v2.py";
        Restart = "always";
        RestartSec = 1;
      };
    };

    systemd.services.rollout-proxy = {
      description = "Stable public rollout proxy";
      wantedBy = [ "multi-user.target" ];
      after = [ "rollout-seed.service" "rollout-v1.service" ];
      requires = [ "rollout-seed.service" "rollout-v1.service" ];
      serviceConfig = {
        ExecStart = "${pkgs.python3}/bin/python ${proxy}";
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

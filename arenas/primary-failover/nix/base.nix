{ lib, pkgs, config, modulesPath, ... }:
let
  replica = pkgs.writeText "failover-replica.py" ''
    import sqlite3
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
    DB = "/var/lib/failover/records.sqlite3"
    ROLE = "/var/lib/failover/replica.role"
    def role():
        with open(ROLE) as source: return source.read().strip()
    def database():
        connection = sqlite3.connect(DB, timeout=10)
        connection.execute("CREATE TABLE IF NOT EXISTS records (id TEXT PRIMARY KEY, value TEXT NOT NULL)")
        return connection
    class Handler(BaseHTTPRequestHandler):
        def send(self, status, body=b""):
            self.send_response(status); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
        def do_GET(self):
            if self.path == "/health": self.send(200, b"ready"); return
            if self.path == "/role": self.send(200, role().encode()); return
            if self.path.startswith("/records/"):
                connection = database(); row = connection.execute("SELECT value FROM records WHERE id=?", (self.path[9:],)).fetchone(); connection.close()
                self.send(404 if row is None else 200, b"" if row is None else row[0].encode()); return
            self.send(404)
        def do_PUT(self):
            if not self.path.startswith("/records/"): self.send(404); return
            if role() != "primary": self.send(503, b"read-only"); return
            value = self.rfile.read(int(self.headers.get("Content-Length", "0"))).decode(); connection = database()
            connection.execute("INSERT INTO records(id,value) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET value=excluded.value", (self.path[9:], value))
            connection.commit(); connection.close(); self.send(204)
        def log_message(self, *_): pass
    ThreadingHTTPServer(("127.0.0.1", 8082), Handler).serve_forever()
  '';
  proxy = pkgs.writeText "failover-proxy.py" ''
    import http.client
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
    UPSTREAM = "/var/lib/failover/upstream"
    class Handler(BaseHTTPRequestHandler):
        def forward(self):
            try:
                with open(UPSTREAM) as source: port = int(source.read().strip())
                body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
                connection.request(self.command, self.path, body=body)
                response = connection.getresponse(); data = response.read()
                self.send_response(response.status); self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)
                connection.close()
            except Exception:
                self.send_response(502); self.send_header("Content-Length", "0"); self.end_headers()
        do_GET = forward
        do_PUT = forward
        def log_message(self, *_): pass
    ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
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
      "20-management" = { matchConfig.Name = "en*"; networkConfig.DHCP = "ipv4"; };
    };
    services.openssh = {
      enable = true;
      settings = { PermitRootLogin = "yes"; PasswordAuthentication = true; };
    };
    users.users.root.initialPassword = config.arena.password;
    environment.systemPackages = with pkgs; [ curl jq lsof procps python3 sqlite strace vim ];
    systemd.services.failover-seed = {
      description = "Seed failed-primary topology and replicated state";
      wantedBy = [ "multi-user.target" ];
      before = [ "failover-replica.service" "failover-proxy.service" ];
      serviceConfig = { Type = "oneshot"; RemainAfterExit = true; };
      script = ''
        install -d -m 0755 /var/lib/failover
        [[ -e /var/lib/failover/upstream ]] || printf '8081\n' > /var/lib/failover/upstream
        [[ -e /var/lib/failover/replica.role ]] || printf 'replica\n' > /var/lib/failover/replica.role
        touch /var/lib/failover/primary.failed
        ${pkgs.python3}/bin/python - <<'PY'
        import sqlite3
        db=sqlite3.connect('/var/lib/failover/records.sqlite3')
        db.execute('CREATE TABLE IF NOT EXISTS records (id TEXT PRIMARY KEY, value TEXT NOT NULL)')
        for record,value in [('customer-red-4a7','red-original'),('customer-blue-8d2','blue-original'),('customer-green-f31','green-original')]:
            db.execute('INSERT OR IGNORE INTO records(id,value) VALUES(?,?)',(record,value))
        db.commit()
        PY
      '';
    };
    systemd.services.failover-replica = {
      description = "Healthy read-only failover replica";
      wantedBy = [ "multi-user.target" ];
      after = [ "failover-seed.service" ];
      requires = [ "failover-seed.service" ];
      serviceConfig = { ExecStart = "${pkgs.python3}/bin/python ${replica}"; Restart = "always"; RestartSec = 1; };
    };
    systemd.services.failover-proxy = {
      description = "Public proxy stranded on failed primary";
      wantedBy = [ "multi-user.target" ];
      after = [ "failover-seed.service" ];
      requires = [ "failover-seed.service" ];
      serviceConfig = { ExecStart = "${pkgs.python3}/bin/python ${proxy}"; Restart = "always"; RestartSec = 1; };
    };
    virtualisation = { cores = 1; memorySize = 768; diskSize = 2048; graphics = false; };
  };
}

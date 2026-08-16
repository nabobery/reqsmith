# docs/demo.tape
#
# VHS script (https://github.com/charmbracelet/vhs) that records a short,
# deterministic, fully local tour of the `reqsmith` CLI and renders it to
# docs/demo.gif.
#
# This intentionally exercises the headless CLI, not the interactive TUI:
# the TUI is keyboard/state driven and flaky to record reproducibly, while
# `list` / `validate` / `fmt` / `run` / `diff` produce the same shape of
# output on every machine.
#
# Usage (run from the repo root):
#   cargo build --release --locked      # build ./target/release/reqsmith once
#   vhs docs/demo.tape                  # renders docs/demo.gif
#
# The live-request section uses a scratch Python HTTP server bound to loopback.
# No external network, real credentials, or checked-in fixture state is used.

Output docs/demo.gif

Set Shell "bash"
Set FontSize 16
Set Width 1200
Set Height 680
Set Padding 20
Set TypingSpeed 35ms
Set Theme "Dracula"

# --- Setup: build a scratch project so the recording never depends on (or
# pollutes) the repo's own example files. Runs hidden, off-screen.
Hide
Type "REPO_ROOT=$(pwd)"
Enter
Type "rm -rf /tmp/reqsmith-demo && mkdir -p /tmp/reqsmith-demo && cd /tmp/reqsmith-demo"
Enter
Type "export PATH=$REPO_ROOT/target/release:$PATH"
Enter
Type "hash -r"
Enter
Type "cat > fixture.py << 'PY'"
Enter
Type "import http.server, json"
Enter
Type "counter = 0"
Enter
Type "class Handler(http.server.BaseHTTPRequestHandler):"
Enter
Type "    def do_GET(self):"
Enter
Type "        global counter"
Enter
Type "        counter += 1"
Enter
Type "        body = json.dumps({'message': 'hello from reqsmith', 'version': counter}, sort_keys=True).encode()"
Enter
Type "        self.send_response_only(200)"
Enter
Type "        self.send_header('Content-Type', 'application/json')"
Enter
Type "        self.send_header('Content-Length', str(len(body)))"
Enter
Type "        self.send_header('Set-Cookie', 'session=synthetic-demo-secret')"
Enter
Type "        self.end_headers()"
Enter
Type "        self.wfile.write(body)"
Enter
Type "    def log_message(self, *_): pass"
Enter
Type "server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)"
Enter
Type "open('.env', 'w').write(f'base_url=http://127.0.0.1:{server.server_port}\\n')"
Enter
Type "server.serve_forever()"
Enter
Type "PY"
Enter
Type "python3 fixture.py >/dev/null 2>&1 & FIXTURE_PID=$!"
Enter
Type "while [ ! -s .env ]; do sleep 0.05; done"
Enter
Type "cat > get_users.req.yml << 'YAML'"
Enter
Type "name: Get Users"
Enter
Type "method: GET"
Enter
Type "url: '{{base_url}}/users'"
Enter
Type "headers:"
Enter
Type "  - key: Accept"
Enter
Type "    value: application/json"
Enter
Type "assertions:"
Enter
Type "  - expect_status: 200"
Enter
Type "  - expect_time_under: 3000ms"
Enter
Type "YAML"
Enter
Type "cat > broken.req.yml << 'YAML'"
Enter
Type "name: Missing Variable"
Enter
Type "method: GET"
Enter
Type "url: '{{missing_var}}/broken'"
Enter
Type "YAML"
Enter
Type "clear"
Enter
Show

# --- CLI overview ---
Type "reqsmith --help"
Enter
Sleep 2s
Type "clear"
Enter

# --- Discover request files in the project ---
Type "reqsmith list"
Enter
Sleep 1500ms
Type "clear"
Enter

# --- Validate: schema + interpolation checking, no network required ---
Type "reqsmith validate get_users.req.yml broken.req.yml"
Enter
Sleep 2500ms
Type "clear"
Enter

# --- Format: canonical YAML ordering, offline and idempotent ---
Type "reqsmith fmt get_users.req.yml --check"
Enter
Sleep 1s
Type "reqsmith fmt get_users.req.yml"
Enter
Sleep 1500ms
Type "clear"
Enter

# --- Local request: successful assertion, visible header redaction, save, diff ---
Type "reqsmith run get_users.req.yml --output json --save"
Enter
Sleep 2500ms
Hide
Type "mv .reqsmith/runs/*.json .reqsmith/runs/baseline.json"
Enter
Show
Type "clear"
Enter

Type "reqsmith run get_users.req.yml --save"
Enter
Sleep 2s
Hide
Type "mv $(ls .reqsmith/runs/*.json | grep -v baseline.json) .reqsmith/runs/candidate.json"
Enter
Show
Type "clear"
Enter

Type "reqsmith diff .reqsmith/runs/baseline.json .reqsmith/runs/candidate.json"
Enter
Sleep 3500ms
Type "clear"
Enter

# --- Open the deterministic scratch collection in the TUI, then quit ---
Type "reqsmith"
Enter
Sleep 3s
Type "q"
Sleep 1s

Sleep 2s
Hide
Type "kill $FIXTURE_PID"
Enter

# The live deployment

One small Ubuntu 24.04 VPS runs two systemd services as an unprivileged `quill` user, behind Caddy:

| Service | What it runs | Unit |
|---|---|---|
| `quill-api` | The API and the web app on `127.0.0.1:8091`; Caddy serves it at `https://quil.timilehin-olowu.xyz` and `https://quil-api.timilehin-olowu.xyz` | [quill-api.service](quill-api.service) |
| `quill-keeper` | Requests each hour's fixing and settles expired markets | [quill-keeper.service](quill-keeper.service) |

The keeper signs as `keeper.quill-1112-f33adb.testnet`, an account made only for this, holding a little test NEAR. Requesting a fixing and settling are open to anyone, so it needs no privileges and the deployer's key never leaves the developer's machine. Its key file is on the server only (mode 600, owner `quill`), never in git.

To set it up again:

```
sudo useradd --system --create-home --home-dir /opt/quill --shell /bin/bash quill
sudo -u quill -H bash -lc 'curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal && git clone https://github.com/Timi16/Quil ~/src && cd ~/src && ~/.cargo/bin/cargo build --release -p quill-api -p quill-keeper'
sudo install -d -o quill -g quill -m 700 /opt/quill/data /opt/quill/.near-credentials/testnet
# copy the keeper's key to /opt/quill/.near-credentials/testnet/<account>.json (mode 600)
sudo cp quill-api.service quill-keeper.service /etc/systemd/system/ && sudo systemctl daemon-reload
sudo systemctl enable --now quill-api quill-keeper
# add the Caddyfile block above to /etc/caddy/Caddyfile, then: sudo systemctl reload caddy
```

To update: `cd /opt/quill/src && git pull && cargo build --release -p quill-api -p quill-keeper`, then `sudo systemctl restart quill-api quill-keeper`.

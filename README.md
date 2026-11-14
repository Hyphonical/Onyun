<div align="center">
  <img src="web/public/branding/icon.svg" alt="Onyun" width="112" height="112" />
  <h1>Onyun</h1>
  <p>A little Tor v3 <code>.onion</code> vanity address generator.</p>
  <img src="assets\Demo.gif" alt="Demo" width="600"/>
  <p>
    <a href="https://github.com/Hyphonical/Onyun/actions/workflows/ci-linux.yml"><img src="https://github.com/Hyphonical/Onyun/actions/workflows/ci-linux.yml/badge.svg" alt="Linux Build" /></a>
    <a href="https://github.com/Hyphonical/Onyun/actions/workflows/ci-docker.yml"><img src="https://github.com/Hyphonical/Onyun/actions/workflows/ci-docker.yml/badge.svg" alt="Docker" /></a>
    <a href="https://github.com/Hyphonical/Onyun/releases"><img src="https://img.shields.io/github/v/release/Hyphonical/Onyun?label=release" alt="Release" /></a>
    <img src="https://img.shields.io/badge/license-AGPLv3-blue" alt="License: AGPLv3" />
  </p>
</div>

---

Onyun searches for custom Tor v3 `.onion` addresses starting with whatever prefix you want. You can run it straight in your terminal, open it as a desktop window, or leave it running on a home server with Docker.

Because hunting for long prefixes takes a while, Onyun saves every longer match it stumbles across along the way. If you aim for a 9-character name and decide to stop early, you still walk away with the closest `.onion` it found, already formatted and permissioned for Tor. If you run the same command later, it checks your existing `hostname` file and picks up right where it left off.

## How it works

* **Saves and resumes automatically:** Every time it hits a longer prefix (`o...` → `on...` → `onyun...`), it atomically writes the keys to disk. Stopping halfway through never wastes the run, and restarting automatically resumes from your best match on disk without needing a database.
* **Uses whatever hardware you have:** Runs across all your CPU cores by default, or offloads the work to your GPU via WebGPU / Vulkan if you have one available.
* **Three ways to run it:** A simple CLI for quick terminal runs, a live web dashboard if you want to check in on a VPS or home server, or a native Tauri desktop app.
* **Ready for `torrc`:** Outputs `hostname`, `hs_ed25519_secret_key`, and `hs_ed25519_public_key` directly in Tor's `HiddenServiceDir` layout, setting `0700` directory and `0600` secret key permissions automatically on Unix.

### Estimated search time

Average time to find a prefix of a given length at a given hashrate.

| Prefix length | 0.5 MH/s | 1 MH/s | 4 MH/s | 10 MH/s | 50 MH/s | 100 MH/s | 500 MH/s | 1000 MH/s |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 5 | 67 s | 34 s | 8 s | 3 s | <1 s | <1 s | <1 s | <1 s |
| 6 | 36 min | 18 min | 4.5 min | 1.8 min | 21 s | 11 s | 2 s | 1 s |
| 7 | 19 h | 9.5 h | 2.4 h | 57 min | 11.5 min | 5.7 min | 69 s | 34 s |
| 8 | 25 days | 13 days | 3.2 days | 30 h | 6.1 h | 3 h | 37 min | 18 min |
| 9 | 2.2 years | 1.1 years | 102 days | 41 days | 8 days | 4 days | 19.5 h | 9.8 h |
| 10 | 71 years | 36 years | 8.9 years | 3.6 years | 260 days | 130 days | 26 days | 13 days |

### Measured hashrates

| Hardware | Engine | Hashrate |
| :--- | :--- | ---: |
| AMD Ryzen 7 7700 (13 threads) | CPU | ~96 MH/s |
| AMD Ryzen 5 5600G (11 threads) | CPU | ~65 MH/s |
| NVIDIA GeForce RTX 4070 | GPU (WGPU/Vulkan) | ~181 MH/s |
| NVIDIA GeForce RTX 4060 Laptop | GPU (WGPU/Vulkan) | ~90 MH/s |

*Averages only: the search is random, so you can get lucky or unlucky. Roughly half of runs finish within 0.7× the listed time, and about 95% within 3×. Each extra character multiplies the time by 32.*

---

## Getting started

### Docker (Web UI)

If you want to leave it running on a server, you can spin up the container:

```bash
docker run -d \
  --name onyun \
  -p 8080:8080 \
  -v ./mints:/data/mints \
  ghcr.io/hyphonical/onyun:latest
```

Or use the included compose file:

```bash
docker compose up -d
```

Once it's up, head to `http://localhost:8080`.

* **Adding a password:** If you're exposing the web UI, set `ONYUN_USER` and `ONYUN_PASS` on the container (or in your shell) to put a login prompt in front of it.
* **Using a GPU in Docker:** The image ships with Vulkan support and falls back to CPU if it doesn't find a GPU. On Linux, pass `--device /dev/dri:/dev/dri` (or use the NVIDIA Container Toolkit) to let it use your card.

### Prebuilt binaries

Grab a binary for Linux, macOS, or Windows from [Releases](https://github.com/Hyphonical/Onyun/releases):

```bash
# Search for an address starting with "onyun" in the terminal
onyun mint onyun

# Open the desktop app
onyun ui

# Run just the web UI without a desktop window
onyun ui --web --port 8080
```

### Building from source

You'll need [Rust](https://rustup.rs/) and [Bun](https://bun.sh/) (for the web frontend):

```bash
# 1. Build the frontend assets
bun install --cwd web
bun run --cwd web build

# 2. Build Onyun (CLI + Web + GPU, skipping Tauri desktop dependencies)
cargo build --release --no-default-features --features web,gpu
```

*(Drop `--no-default-features` if you want the Tauri desktop app compiled in as well.)*

---

## Where the keys go

Each prefix gets its own directory inside `./mints/<prefix>/` (or `/data/mints/<prefix>/` in Docker):

```text
mints/
└── onyun/
    ├── hostname
    ├── hs_ed25519_public_key
    └── hs_ed25519_secret_key
```

To use one with Tor, copy that folder over and point `HiddenServiceDir` in your `torrc` to it (Onyun already sets `0700` on the folder and `0600` on `hs_ed25519_secret_key` on Linux/macOS, though you should double-check ownership if copying to a `debian-tor` user).

A quick heads-up on prefix length: Tor v3 addresses use Base32 (`a-z` and `2-7`, so `0`, `1`, `8`, and `9` aren't valid), and each extra character makes the search roughly **32x** harder. 5 to 7 characters usually pop up pretty fast, while 8+ characters are better suited for a GPU or leaving it running overnight to see how far it gets.

---

## CLI options

| Flag / Command | Default | What it does |
| :--- | :--- | :--- |
| `onyun mint <prefix>` | (required) | Search for an address starting with `<prefix>` |
| `-t, --threads <n>` | All cores | Number of CPU worker threads |
| `-e, --engine <kind>` | `auto` | Pick `auto`, `cpu`, or `gpu` |
| `-o, --out <dir>` | `./mints` | Folder where minted keys get saved |
| `-q, --quiet` | `false` | Only print the final `.onion` address |
| `onyun ui` | | Launch the desktop window (or Web UI if built headless) |
| `--web` | `false` | Serve the web UI without opening a desktop window |
| `-p, --port <port>` | `0` / `8080` | Port for the UI (`0` random in desktop mode, `8080` with `--web` or non-local `--bind`) |
| `-b, --bind <ip>` | `127.0.0.1` | Bind address (`0.0.0.0` to expose it on your network) |
| `--no-color` | `false` | Disable ANSI terminal colors |

---

## License

[AGPLv3](LICENSE)
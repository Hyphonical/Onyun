<div align="center">
  <img src="web/public/branding/icon.svg" alt="Onyun" width="112" height="112" />
  <h1>Onyun</h1>
  <p><strong>A fast, friendly Tor v3 <code>.onion</code> vanity address minter.</strong></p>
  <p>
    <a href="https://github.com/Hyphonical/Onyun/actions/workflows/ci-linux.yml"><img src="https://github.com/Hyphonical/Onyun/actions/workflows/ci-linux.yml/badge.svg" alt="Linux Build" /></a>
    <a href="https://github.com/Hyphonical/Onyun/actions/workflows/ci-docker.yml"><img src="https://github.com/Hyphonical/Onyun/actions/workflows/ci-docker.yml/badge.svg" alt="Docker" /></a>
    <a href="https://github.com/Hyphonical/Onyun/releases"><img src="https://img.shields.io/github/v/release/Hyphonical/Onyun?label=release" alt="Release" /></a>
    <img src="https://img.shields.io/badge/license-AGPLv3-blue" alt="License: AGPLv3" />
  </p>
</div>

---

Onyun generates custom Tor v3 `.onion` addresses starting with any prefix you choose. You can run it right in your terminal, open it as a desktop app, or host its web dashboard on a server with Docker.

As Onyun searches, it saves every longer match it finds along the way—so if you ask for a long prefix and stop early, you still keep the closest `.onion` address it reached, ready to drop straight into Tor.

## Highlights

- **Saves as it goes** — Progressively saves your best match left-to-right (`o...` → `on...` → `onyun...`), so nothing is lost if you pause or stop early.
- **CPU & GPU support** — Uses all available CPU cores out of the box, or accelerates on GPU via WebGPU / Vulkan when available.
- **Three ways to use it** — Clean CLI for quick runs, a live Web UI for servers, or a native Tauri desktop window.
- **Ready for Tor** — Writes `hostname`, `hs_ed25519_secret_key`, and `hs_ed25519_public_key` directly in Tor's `HiddenServiceDir` format.

---

## Quick Start

### Docker (Web UI)

The fastest way to run Onyun on a server or VPS:

```bash
docker run -d \
  --name onyun \
  -p 8080:8080 \
  -v ./mints:/data/mints \
  ghcr.io/hyphonical/onyun:latest
```

Or with Docker Compose from this repository:

```bash
docker compose up -d
```

Then open **`http://localhost:8080`** in your browser.

> **Password-protecting the Web UI:** Set `ONYUN_USER` and `ONYUN_PASS` environment variables on the container (or in your shell) to require a login before accessing the dashboard.
>
> **GPU in Docker:** The container includes Vulkan support and falls back to CPU automatically. On a Linux host with a Vulkan-capable GPU, pass `--device /dev/dri:/dev/dri` (or use the NVIDIA Container Toolkit) to enable GPU minting.

---

### Prebuilt Binary / CLI

Grab a binary for Linux, macOS, or Windows from [Releases](https://github.com/Hyphonical/Onyun/releases), then:

```bash
# Mint an address starting with "onyun" in the terminal
onyun mint onyun

# Launch the native desktop app
onyun ui

# Serve only the Web UI (no desktop window)
onyun ui --web --port 8080
```

---

### Build from Source

Requires [Rust](https://rustup.rs/) and [Bun](https://bun.sh/) (for building the frontend assets):

```bash
# 1. Build the Web UI
bun install --cwd web
bun run --cwd web build

# 2. Build Onyun (CLI + Web + GPU, without Tauri desktop dependencies)
cargo build --release --no-default-features --features web,gpu
```

*(Omit `--no-default-features` if you also want the native Tauri desktop window.)*

---

## Where Your Keys Live

Every target gets its own folder inside `./mints/<prefix>/` (or `/data/mints/<prefix>/` in Docker):

```text
mints/
└── onyun/
    ├── hostname
    ├── hs_ed25519_public_key
    └── hs_ed25519_secret_key
```

To use a minted address with Tor, copy that folder (with `700` permissions on Linux/macOS) and point your `torrc` `HiddenServiceDir` to it.

> **Tip:** Tor v3 addresses use Base32 characters (`a–z` and `2–7`). Each extra character makes the search roughly **32×** longer—5 to 7 characters usually finish quickly, while 8+ characters are where GPU acceleration or letting Onyun save its best match overnight really shines.

---

## Common Options

| Command & Flag | Default | What it does |
| :--- | :--- | :--- |
| `onyun mint <prefix>` | — | Start minting an address beginning with `<prefix>` |
| `-t, --threads <n>` | All cores | Number of CPU worker threads to use |
| `-e, --engine <kind>` | `auto` | Choose `auto`, `cpu`, or `gpu` |
| `-o, --out <dir>` | `./mints` | Folder where minted keys are saved |
| `-q, --quiet` | `false` | Print only the final `.onion` address |
| `onyun ui` | — | Open the desktop window (or Web UI when built headless) |
| `--web` | `false` | Run as a headless web server without opening a window |
| `-p, --port <port>` | `8080` | Port for the Web UI (`0` random port in desktop mode) |
| `-b, --bind <ip>` | `127.0.0.1` | Bind address (use `0.0.0.0` to expose on your network) |

---

## License

Released under the [AGPLv3 License](LICENSE).

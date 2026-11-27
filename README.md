![Rain Term](./public/logo.svg)

# Rain Term

A lightweight SSH client built with Tauri: tabbed terminals, jump hosts and SFTP.

Languages: English · [简体中文](./docs/README_zh-CN.md) · [繁體中文](./docs/README_zh-TW.md)

[![Release](https://img.shields.io/github/v/release/rain-kit/rain-term?style=flat-square)](https://github.com/rain-kit/rain-term/releases)
[![Downloads](https://img.shields.io/github/downloads/rain-kit/rain-term/total?style=flat-square)](https://github.com/rain-kit/rain-term/releases)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue?style=flat-square)

## Install

Go to the [Release page](https://github.com/rain-kit/rain-term/releases) to download the installer for your platform.

| Platform | Architecture | File |
| :------- | :----------- | :--- |
| Windows 10 / 11 | x64 | `Rain.Term_windows_x.y.z_x64-setup.exe` |
| Windows 11 | ARM64 | `Rain.Term_windows_x.y.z_arm64-setup.exe` |
| macOS | Apple Silicon (M series) | `Rain.Term_macos_x.y.z_aarch64.dmg` |
| macOS | Intel | `Rain.Term_macos_x.y.z_x64.dmg` |
| Debian / Ubuntu | x64 | `Rain.Term_linux_x.y.z_amd64.deb` |
| Debian / Ubuntu | ARM64 | `Rain.Term_linux_x.y.z_arm64.deb` |
| RHEL / Fedora / CentOS | x64 | `Rain.Term_linux_x.y.z_x86_64.rpm` |
| RHEL / Fedora / CentOS | ARM64 | `Rain.Term_linux_x.y.z_aarch64.rpm` |

On Linux, install with `sudo apt install ./<file>.deb` or `sudo dnf install ./<file>.rpm`.

When you run a newer Windows installer over an existing installation, it recommends uninstalling the current version first. Choose "Do not uninstall" to install over it directly. Updating from inside the app skips this step. Your connections, settings and saved passwords are kept either way.

> [!NOTE]
> The installers are not code-signed yet.
>
> - **Windows**: if SmartScreen blocks the installer, click "More info", then "Run anyway".
> - **macOS**: if macOS says the app "is damaged and can't be opened", run this once in Terminal:
>
>   ```shell
>   xattr -cr "/Applications/Rain Term.app"
>   ```

## Features

- **Connection management**: groups, color labels, notes, search, drag-to-reorder and duplicate. Import and export connections, or import from FinalShell and MobaXterm.
- **Terminal**
  - Open as many tabs as you like; tabs to the same server share one SSH connection.
  - Copy on select, right-click paste, `Ctrl+Shift+C` / `V`, search with `Ctrl+Shift+F`, and clickable links.
  - Correct width for CJK characters and emoji; `vim`, `top` and `htop` render properly. WebGL rendering, with a fallback when WebGL is unavailable.
  - Shows why a session ended and the exit code, with one-click reconnect. Keepalive detects dropped connections.
  - Quick connect with `Ctrl+Shift+K` (`⌘K` on macOS). Tabs from the last session are restored on restart, without connecting automatically.
- **Jump hosts**: connect through one or more jump hosts. Each hop's fingerprint is verified, and errors tell you which hop failed.
- **SFTP**
  - Shares the terminal's connection. Browsing stays smooth even in folders with tens of thousands of files.
  - Create folders, rename, delete and copy paths.
  - Upload and download files and whole folders, or drag files in from your system. Transfers are queued with progress and speed, can be cancelled at any time, and incomplete files are removed. Uploading over an existing file or folder asks for confirmation first.
- **Flexible authentication**: password, private key (with passphrase), or the system SSH agent.
- **Safe by default**
  - Server fingerprints are verified on first connection and saved to `known_hosts`. Connections are refused if the fingerprint changes.
  - Passwords are stored in the system credential store (Windows Credential Manager / macOS Keychain / Linux Secret Service), never in config files or exported files.
  - Mark a connection as production: it is shown in red, and pasting anything into its terminal asks for confirmation. Pasting multi-line text into any terminal asks too.
- **Personalization**: terminal font, size, color schemes (One Dark, Dracula, Nord, Solarized and more), cursor style and scrollback; light / dark / system theme; English, Simplified Chinese and Traditional Chinese.
- **More**: system tray with recent connections, built-in log viewer, in-app updates.

## Data

All data stays on your own device. The data folder is:

- Windows: `%AppData%\rain-dial`
- macOS: `~/Library/Application Support/rain-dial`
- Linux: `~/.config/rain-dial`

| File | Contents |
| :--- | :------- |
| `connections.json` | Connections (no passwords) |
| `config.yaml` | App settings |
| `known_hosts` | Trusted server fingerprints |
| `workspace.json` | Tabs open in the last session |
| `logs/` | Log files |

Fingerprints in `~/.ssh/known_hosts` are trusted as well.

Set the `RAIN_DIAL_DATA_DIR` environment variable to use a different data folder, e.g. for portable use or testing.

## Privacy

Rain Term has no telemetry and does not collect personal data. It only makes these network connections:

- To the SSH servers (and jump hosts) you connect to.
- To GitHub, to check for updates: the app requests the latest release information from `api.github.com` after launch and every 6 hours. The request carries only the app version. When you choose to update, the installer is downloaded from GitHub Releases. GitHub sees your IP address as with any web request; see the [GitHub Privacy Statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement).

## Development

Prerequisites:

- [Node.js](https://nodejs.org/) 22.12+
- [Rust](https://www.rust-lang.org/tools/install) 1.88+ (stable)
- The [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform

```shell
# Install dependencies
npm install

# Start the app in development mode
npm run tauri dev

# Build installers for the current platform
npm run tauri build

# Type-check the frontend
npx tsc --noEmit

# Run backend tests
cd src-tauri
cargo test
```

`cargo test` also runs integration tests against real OpenSSH servers in Docker (`src-tauri/tests/docker`), which need Docker Compose v2. They are skipped when Docker is not available or `RAIN_DIAL_SKIP_DOCKER` is set.

For trying the UI without a real server, `cargo run --example dev_sshd` (in `src-tauri`) starts a local test SSH server on port 2222 with user `tester` and password `pa55`.

### Release

Pushing a tag that starts with `v` triggers [GitHub Actions](./.github/workflows/release.yml), which builds installers for Windows, macOS and Linux (x64 and ARM64 each) and publishes them to Releases. Pushing to `main` only compiles, which checks the build and warms the cache for the next release.

Installers also produce signed in-app update packages and a `latest.json`, so building them requires the updater signing key. Set the repository secret `TAURI_SIGNING_PRIVATE_KEY` (and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if the key has one); for a local `npm run tauri build`, set the same environment variables. The matching public key is `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`.

1. Update the version in `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`.
2. Add a `## vX.Y.Z` section to [ChangeLog.md](./ChangeLog.md). It becomes the "What's new" part of the release notes and of the in-app update page; the release fails if the section is missing.
3. Commit, then create and push a matching tag:

   ```shell
   git tag v1.0.0
   git push origin main --tags
   ```

The workflow fails if the tag does not match the version in `tauri.conf.json`.

## Contributions

Issues and pull requests are welcome!

## Acknowledgement

Rain Term is built on these projects:

- [tauri-apps/tauri](https://github.com/tauri-apps/tauri): Build smaller, faster, and more secure desktop applications with a web frontend.
- [Eugeny/russh](https://github.com/Eugeny/russh): Rust SSH client and server library.
- [xtermjs/xterm.js](https://github.com/xtermjs/xterm.js): A terminal for the web.
- [tokio-rs/tokio](https://github.com/tokio-rs/tokio): An asynchronous runtime for Rust.
- [facebook/react](https://github.com/facebook/react): The library for web and native user interfaces.
- [vitejs/vite](https://github.com/vitejs/vite): Next generation frontend tooling.
- [tailwindlabs/tailwindcss](https://github.com/tailwindlabs/tailwindcss): A utility-first CSS framework.
- [lucide-icons/lucide](https://github.com/lucide-icons/lucide): Beautiful and consistent icons.

## License

[MIT](./LICENSE)

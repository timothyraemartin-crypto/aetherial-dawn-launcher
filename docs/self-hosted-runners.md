# Self-hosted runners (builds on Timothy's PC)

Every workflow job except `bug-investigate.yml` runs on one of two runners:

| Labels | Where | Jobs |
|---|---|---|
| `self-hosted, windows, x64` | native Windows, dedicated local user | `build.yml` `windows` (tests, clippy, Tauri NSIS build, signed release build, installer test) |
| `self-hosted, linux, x64` | WSL2 Ubuntu 22.04+ | everything else (`canonical-plugins-linux`, `release`, contract, scripts, publish-feed, autofix-bridge, capture-errors) |

No macOS jobs exist. `bug-investigate.yml` stays on GitHub-hosted runners: its verify job runs a model-written patch.

## Windows runner must have
- Git for Windows, with `C:\Program Files\Git\bin` BEFORE `C:\Windows\System32` in PATH (else `shell: bash` runs in WSL)
- Node.js 22 on PATH, PowerShell 7 (`pwsh`)
- rustup with the stable MSVC toolchain (`x86_64-pc-windows-msvc`); the workflow adds clippy and rustfmt
- Visual Studio 2022 Build Tools: "Desktop development with C++" (MSVC, Windows 11 SDK)
- Google Chrome in `C:\Program Files\Google\Chrome` (headless UI tests)
- WebView2 runtime (the installer test launches the app); internet access (Tauri downloads NSIS, the WebView2 bootstrapper, `cargo-tauri` is installed by the workflow)
- Runner as a DEDICATED local Windows user (e.g. `adrunner`) with auto-logon, started by `run.cmd` at logon, NOT as a service: the installer test needs a visible window, and it installs, force-closes and uninstalls the launcher for that user.
- `AD_CI_RUNNER=1` in the runner's `.env`. `one-click-test.ps1` refuses to run on a self-hosted runner without it.

## Linux (WSL2) runner must have
- `build-essential` (gcc, g++ for unrar/bzip2/zstd/ring), `pkg-config`, `musl-tools`, `curl`, `git`, `jq`, `openssh-client`, `unzip`, `ca-certificates`
- rustup (stable; the workflow adds the musl target)
- Node.js 22 on PATH (contract, scripts and release jobs call `node` without setup-node)
- GitHub CLI `gh` (release job)
- Passwordless sudo is NOT needed if `musl-tools` is preinstalled
- Keep WSL running while the PC is on (autofix-bridge fires every 5 minutes)

## Behaviour to know
- `actions/checkout` wipes the workspace each run, so Rust rebuilds from `Swatinem/rust-cache`.
- Jobs only run while the PC and runners are on; queued jobs older than 24 h fail.
- Secrets (`TAURI_SIGNING_PRIVATE_KEY`, `AD_SSH_KEY`) are visible to the runner during a job; keep the repo private and never allow fork PRs on these runners.

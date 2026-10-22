# coocoo-desktop

Tauri 2 shell around the Coocoo daemon. On launch it attaches to a `coocoo serve` that already
serves the same config (`/api/health` host id matches `config.toml`), otherwise it starts the
daemon in-process. The Svelte UI (`../ui`) gets `window.__COOCOO__ = { wsUrl, token, native }`
before it loads. Closing the window hides to the tray; agents keep running until **Quit**.

## Build

Linux needs the webkit2gtk-4.1/gtk3 dev packages (or `source ~/.local/share/coocoo-sysroot/env.sh`
on machines using the user-local sysroot; it also sets `LD_LIBRARY_PATH` for `libxdo`).

```sh
cd src-tauri
npm install                 # @tauri-apps/cli (devDependency)
npm run dev                 # Vite dev server (ui) + debug app
npm run build               # ui/dist + release bundles (deb, AppImage / app, dmg / nsis)
npm run icon                # regenerate icons/* from icons/icon.svg
```

Plain cargo works too: `cargo build -p coocoo-desktop` loads the dev server (`devUrl`);
add `--features custom-protocol` to serve the built `ui/dist` instead.

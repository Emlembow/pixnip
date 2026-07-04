# BetterSS

A screenshot tool for Windows that actually works with HDR monitors.

## The problem

Windows' built-in Snipping Tool captures the raw HDR framebuffer without
tone mapping it down correctly. If you have HDR enabled — as you should —
every screenshot of SDR content (browser windows, most apps) comes out
blown out and washed out. Disabling HDR isn't an option if you want it for
the content that actually benefits from it.

## What BetterSS does

- **Captures HDR displays correctly.** Reads each monitor's actual SDR
  white level and tone-maps against it, so SDR content looks exactly as it
  does on screen and true HDR highlights are compressed gracefully instead
  of clipped.
- **Intercepts PrintScreen** to launch a region picker instead of the
  built-in Snipping Tool.
- **Modern markup editor** — crop, pen, shapes, arrows, text, blur/redact,
  step badges — in a minimal, Instagram/Snapchat-style interface.
- **Lives in the tray** as a lightweight background app, auto-saving
  screenshots to a configurable folder with clipboard copy on export.

## Status

Early development. See the plan/roadmap for what's built so far.

## Building from source

Requires:
- [Rust](https://rustup.rs/) (stable, MSVC toolchain on Windows)
- [Node.js](https://nodejs.org/) 18+
- [Tauri prerequisites](https://tauri.app/start/prerequisites/) for Windows

```sh
npm install
npm run tauri dev    # run in development
npm run tauri build  # produce a release build
```

### If saved screenshots don't appear

If your Pictures folder is redirected into OneDrive and screenshots silently
fail to save, Windows' **Controlled Folder Access** (Windows Security >
Virus & threat protection > Ransomware protection) is likely blocking an
unrecognized/unsigned build of BetterSS from writing there. Add the exe
under "Allow an app through Controlled folder access" — every screenshot
tool runs into this on a fresh build.

## License

[MIT](LICENSE)

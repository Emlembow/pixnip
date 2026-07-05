# Publishing Pixnip to the Microsoft Store

The repo builds a Store-ready MSIX (`npm run pack:msix` → `Pixnip_<version>_x64.msix`).
The Store signs packages on ingestion, so no code-signing certificate is needed.

## One-time setup (Partner Center)

1. Enroll at https://partner.microsoft.com/dashboard — individual account, $19 one-time.
2. Apps and Games → New product → **MSIX or PWA app** → reserve the name **Pixnip**.
3. Open Product management → Product identity, and copy the assigned values into
   `Package.appxmanifest`:
   - `Identity Name` (looks like `12345YourName.Pixnip`)
   - `Identity Publisher` (looks like `CN=xxxxxxxx-xxxx-...`)
   - `PublisherDisplayName`
4. Rebuild the package: `npm run pack:msix`.

## Each submission

1. Bump `Version` in `Package.appxmanifest` (four parts, e.g. `0.2.0.0`;
   keep in sync with `tauri.conf.json` / `Cargo.toml`).
2. `npm run pack:msix` — upload the resulting `.msix` in the Packages step.
3. Store listing requirements:
   - Description, at least one screenshot (capture the editor with markup on it),
     app icon is taken from the package.
   - **Privacy policy URL** — required; a page on the project repo/site stating
     Pixnip processes screenshots locally and sends nothing anywhere suffices.
   - Age rating questionnaire (Pixnip has no user content/ads → all-ages).
4. Optional pre-check: run the Windows App Certification Kit against the MSIX
   (`appcert.exe` from the Windows SDK) to catch failures before certification.

## Local side-load testing

```powershell
winapp cert generate --if-exists skip     # once; creates devcert.pfx (gitignored)
winapp cert install .\devcert.pfx         # once, as admin — trusts the dev cert
.\packaging\pack-msix.ps1 -Sign
Add-AppxPackage .\Pixnip_0.1.0.0_x64.msix
```

`winapp unregister` removes a dev registration; installed-package updates need a
higher manifest version.

## MSIX behavior notes

- Start-with-Windows uses the `PixnipStartup` startup task declared in the
  manifest (registry Run keys are virtualized under MSIX and never execute);
  the same tray toggle drives it — `apply_autostart` picks the right mechanism
  by detecting package identity.
- The PrintScreen hook, screen capture, and clipboard all work under
  `runFullTrust`.
- WebView2: Windows 11 and updated Windows 10 ship the Evergreen runtime;
  the MSIX does not bundle it.

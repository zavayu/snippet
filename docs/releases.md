# Signed in-app updates

Snippet uses public GitHub Releases as its update feed. This is separate from
Windows Authenticode signing; the Windows installer remains unsigned, while
Tauri verifies every in-app download with the public key embedded in the app.

## One-time repository setup

Use the private signing key corresponding to the public key in
`src-tauri/tauri.conf.json`. Store the private key outside the repository. Set
`SNIPPET_UPDATER_KEY_FILE` in your local environment to its location; keep that
machine-specific setting out of version control.

1. Make a secure backup of the private key. Keep it out of commits, releases,
   screenshots, and logs. Losing it prevents updates to installations that trust
   this public key. Do not regenerate the key for each release.
2. In GitHub → repository Settings → Secrets and variables → Actions, create
   `TAURI_SIGNING_PRIVATE_KEY` with the **contents** of that file. With the GitHub
   CLI authenticated to the correct repository, you can upload it without
   printing the contents:

   ```powershell
   Get-Content -Raw -LiteralPath $env:SNIPPET_UPDATER_KEY_FILE | gh secret set TAURI_SIGNING_PRIVATE_KEY --repo zavayu/snippet
   ```

3. If the key has no password, leave `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` unset.
   For a password-protected key, set that second secret as well.

The workflow fails early if the signing secret is missing. No Authenticode
certificate or paid signing service is needed.

## Publishing a release

1. Update versions together in `package.json`, `package-lock.json`,
   `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, and
   `src-tauri/tauri.conf.json`. Use a new version higher than the latest public
   release. Never overwrite an existing published version.
2. Run `npm test`, `npm run build`, and
   `cargo test --manifest-path src-tauri/Cargo.toml --locked`.
3. Commit and push the changes, then push the matching `vX.Y.Z` tag. The workflow
   runs tests and builds the installer using the signing secret.
4. Confirm the GitHub Release has the NSIS installer, its `.sig` file, and
   `latest.json`. The manifest must contain the actual signature text and an
   HTTPS URL for the same installer. Tauri Action generates it automatically,
   including release notes.

The app checks:
`https://github.com/zavayu/snippet/releases/latest/download/latest.json`.
Only public stable releases are available through this endpoint; drafts and
prereleases do not replace it. The latest release must contain a valid update
manifest. The preference in Settings controls automatic checks, not manual
checks. Checks start after a 10-second delay and make at most one automatic
network attempt per 24 hours, including across restarts. Manual checks may be
retried immediately. A failed network check counts as an attempt.

Downloads and installation each require an explicit click. Signature failures
leave nothing installable. Downloaded bytes remain in memory until install or
exit. Snippet blocks installation during generation, OCR, or region selection,
and blocks new captures while launching the installer. Windows exits Snippet
and the installer restarts it. Settings and sign-in registration keep the same
application identity and installation location.

## Local builds and verification

Use `npm run tauri dev` for everyday development. Automatic checks and the
install button are disabled in development builds; manual checks are available.
To produce a signed local installer, supply the private key as a path via the
environment for that terminal session:

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = $env:SNIPPET_UPDATER_KEY_FILE
npx tauri build
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY
```

On another machine use the securely backed-up key. The build generates the
installer and `.sig` under `src-tauri/target/release/bundle/nsis/`.

For end-to-end verification, install an updater-enabled baseline in a test
Windows account or VM, then publish a higher signed version. Check from the
tray or Settings, review its notes, download it, and choose Install and restart.
Confirm settings, shortcuts, and launch-at-sign-in survive. Test while a response
or OCR is active: installation must wait. Test offline and retry after restoring
connectivity. A version predating the updater must be upgraded manually once.

Frontend tests cover the scheduler, restart persistence, consent boundaries,
progress, failed downloads/signatures, resource cleanup, and failed installation
retries. A full Windows replacement test still requires two real releases; do
not publish a dummy version to the stable feed just to exercise the UI.

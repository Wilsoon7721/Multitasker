# Releasing

A tag like `v0.2.0` runs [the release workflow](../.github/workflows/release.yml). It tests everything, builds the
Windows installer, and drafts a GitHub release with:

- `MultiTasker_<version>_x64-setup.exe`, the installer (per user, no admin needed);
- `latest.json` and the installer's signature, which installed copies read to update themselves;
- `multitasker-extension-<version>.zip`, ready to upload to the Chrome Web Store and Edge Add-ons.

Nothing reaches anyone until you publish the draft. Installed copies check for updates when they start and every six
hours, download a new version in the background, and install it when you choose **Restart to update** (in Settings).

## One-time setup

Add these as repository secrets (Settings → Secrets and variables → Actions):

| Secret | What it is |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | The contents of `.secrets/updater.key`. Installed copies only accept updates signed with it. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Empty; the key has no password. Leave the secret unset or empty. |
| `WILSOON_CLIENT_ID` | From `.env`, for signing in to JavaDashboard. |
| `MULTITASKER_GOOGLE_CLIENT_ID`, `MULTITASKER_GOOGLE_CLIENT_SECRET`, `MULTITASKER_MICROSOFT_CLIENT_ID` | From `.env` when set, for syncing through Google Drive and OneDrive (see [cloud-sign-in.md](cloud-sign-in.md)). |

Keep `.secrets/updater.key` safe and backed up. If it's lost, installed copies can't be updated any more and have to be
reinstalled from a build signed with a new key.

## Each release

1. `npm run set-version -- 0.2.0` sets the version for the app, `package.json` and the extension.
2. Commit, then tag and push: `git tag v0.2.0` and `git push origin main v0.2.0`.
3. When the workflow finishes, check the draft release and publish it.

## Publishing the extension

The zip in each release has the manifest's `key` taken out, since the stores won't accept it. Each store gives the
extension its own id, which MultiTasker has to allow before the published extension can talk to it:

1. Upload the zip to the [Chrome Web Store](https://chrome.google.com/webstore/devconsole) and
   [Edge Add-ons](https://partner.microsoft.com/dashboard/microsoftedge). Both ask for:
   - a privacy policy: the extension sends the address and title of the tab in front, and the pages you link, only to
     MultiTasker on the same computer, and nothing anywhere else;
   - why it needs each permission: `nativeMessaging` to talk to the MultiTasker app, and `tabs` to read the address and
     title of the tab in front;
   - a note that it needs the MultiTasker desktop app installed, and screenshots of the popup.
2. Add each store's id to `EXTENSION_IDS` in [`src-tauri/src/bridge.rs`](../src-tauri/src/bridge.rs) and release
   MultiTasker. Once updated, it lets the published extension in too.

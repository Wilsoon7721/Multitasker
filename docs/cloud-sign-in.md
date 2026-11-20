# Setting up Google Drive and OneDrive sign-in

MultiTasker signs in to Google and Microsoft the way desktop apps are meant to: it opens the browser, the person signs
in on Google's or Microsoft's own page, and the browser hands a one-time code back to MultiTasker on a port on the same
computer (OAuth 2.0 authorization code flow with PKCE). MultiTasker never sees a password. The long-lived sign-in
(refresh token) is kept in Windows Credential Manager.

To do this, each service needs to know about the app: an **app registration**, which gives a **client ID**. Register
once, then either build MultiTasker with the IDs or paste them into Settings.

Both registrations ask only for access to MultiTasker's own folder:

| Service      | Permission                  | What MultiTasker can reach                                   |
| ------------ | --------------------------- | ------------------------------------------------------------ |
| Google Drive | `drive.appdata`             | A hidden app-data folder; nothing appears in the person's Drive |
| OneDrive     | `Files.ReadWrite.AppFolder` | `Apps/<registration name>` in the person's OneDrive          |

## Google Drive

1. Open the [Google Cloud console](https://console.cloud.google.com/) and create a project (e.g. "MultiTasker").
2. **APIs & Services → Library**: enable the **Google Drive API**.
3. **Google Auth Platform** (formerly "OAuth consent screen"):
   - **Branding**: app name "MultiTasker", a support email, and (for publishing) a homepage and privacy policy link.
     The GitHub repository and a short privacy note in it are enough.
   - **Audience**: choose **External**.
   - **Data access**: add the scopes `openid`, `.../auth/userinfo.email` and `.../auth/drive.appdata`. All three are
     non-sensitive, so Google doesn't require the security review that full Drive access needs.
4. **Clients → Create client**: application type **Desktop app**. Copy the **client ID** and **client secret**.
   Google doesn't treat a desktop app's secret as confidential, but keep it out of the repository anyway (see below).
5. **Testing vs. production**: while the app is in *Testing*, only accounts listed as test users can sign in, and
   sign-ins expire after 7 days. Add yourself as a test user to try it out, then **Publish app** to move it to
   *In production*. With only non-sensitive scopes, publishing doesn't need Google's review, but Google may ask you to
   verify the brand details (domain, homepage) before it shows your app name and logo on the sign-in page.

## OneDrive

1. Open the [Microsoft Entra admin center](https://entra.microsoft.com/) → **App registrations → New registration**.
   - **Name**: "MultiTasker". OneDrive names the app folder after this (`Apps/MultiTasker`).
   - **Supported account types**: **Personal Microsoft accounts only**. OneDrive's app-folder permission only exists
     for personal accounts, and MultiTasker signs in through the `consumers` endpoint.
   - **Redirect URI**: platform **Public client/native (mobile & desktop)**, URI `http://localhost`. Microsoft lets a
     desktop app use any port on localhost at sign-in time, which MultiTasker relies on.
2. **API permissions → Add a permission → Microsoft Graph → Delegated**: add `Files.ReadWrite.AppFolder`, `openid`,
   `email`, `profile` and `offline_access`. You can remove the default `User.Read`.
3. Copy the **Application (client) ID** from the overview page. There's no secret: desktop apps are public clients.
4. Optional: without **publisher verification**, Microsoft's consent page marks the app as from an unverified
   publisher. Verification is free but needs a Microsoft AI Cloud Partner Program account and a verified domain.

## Using the IDs

**While developing**, set them in the terminal before running the app:

```powershell
$env:MULTITASKER_GOOGLE_CLIENT_ID = "1234-abc.apps.googleusercontent.com"
$env:MULTITASKER_GOOGLE_CLIENT_SECRET = "GOCSPX-..."
$env:MULTITASKER_MICROSOFT_CLIENT_ID = "00000000-0000-0000-0000-000000000000"
npm run tauri dev
```

They're read when the app is compiled (`option_env!`), so builds made this way can sign in with no setup. For release
builds, set the same three variables as CI secrets.

**Without rebuilding**, choose Google Drive or OneDrive in **Settings → Sync** on a build that has no IDs, and paste
them there. The client ID goes in `settings.json`; Google's client secret goes in Windows Credential Manager.

**Forks** should register their own apps rather than reuse the official IDs.

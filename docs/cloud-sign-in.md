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

## What to enter

The same details work for both registrations.

| Field                     | Value                                                                                      |
| ------------------------- | ------------------------------------------------------------------------------------------ |
| App name                  | MultiTasker                                                                                |
| Short description         | An always-on-top focus list that keeps the tasks you've started in view.                   |
| Home page                 | https://multitasker.wilsoon.dev                                                            |
| Privacy policy            | https://multitasker.wilsoon.dev/privacy                                                    |
| Authorized domain         | wilsoon.dev                                                                                |
| Terms of service          | Leave empty (optional)                                                                     |
| Support / developer email | Your own email address                                                                     |
| Logo                      | Google: [`docs/branding/logo-120.png`](branding/logo-120.png) (120×120) · Microsoft: [`docs/branding/logo-240.png`](branding/logo-240.png) (240×240) |

**Longer description**, for anywhere that asks what the app does:

> MultiTasker is an open-source desktop app for people who juggle several tasks at once. Tasks you've started stay
> pinned in a small always-on-top widget, so nothing in progress gets forgotten. It can link files, apps and web pages
> to tasks, notice which task you're working on, and sync your tasks between your own computers.

**How the data is used**, for Google's data-access step or a verification form:

> MultiTasker stores its sync files (the user's tasks, which tasks are in progress, and references to attached files
> and links) in its own app folder, so the user's task list stays the same on each of their computers. It only reads
> and writes files it created in that folder (`drive.appdata` / `Files.ReadWrite.AppFolder`) and never accesses any
> other files. The email address (`openid`, `email`) is only shown in Settings to tell the user which account is
> signed in. Data is never sent to any server other than the user's own Google Drive or OneDrive, is never shared,
> and is never used for advertising or AI training.

**Until multitasker.wilsoon.dev is live:** registering and testing work with the GitHub links instead
(https://github.com/Wilsoon7721/Multitasker and its [`PRIVACY.md`](../PRIVACY.md), authorized domain `github.com`).
Switch to the wilsoon.dev links before publishing the Google app or asking Google to verify the brand: Google requires
those links to be live and on a domain you've proved you own, which you do by verifying `wilsoon.dev` in
[Google Search Console](https://search.google.com/search-console) (a DNS TXT record). The privacy page should say what
[`PRIVACY.md`](../PRIVACY.md) says; keep the two in step.

## Google Drive

1. Open the [Google Cloud console](https://console.cloud.google.com/) and create a project (e.g. "MultiTasker").
2. **APIs & Services → Library**: enable the **Google Drive API**.
3. **Google Auth Platform** (formerly "OAuth consent screen"):
   - **Branding**: fill in the fields from [What to enter](#what-to-enter), including the authorized domain.
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
4. **Branding & properties**: fill in the logo, home page and privacy statement URLs from
   [What to enter](#what-to-enter).
5. Optional: without **publisher verification**, Microsoft's consent page marks the app as from an unverified
   publisher. Verification is free but needs a Microsoft AI Cloud Partner Program account and a verified domain. For
   the domain, set **Branding & properties → Publisher domain** to `wilsoon.dev`; Microsoft checks it by fetching a
   small JSON file it gives you from `https://wilsoon.dev/.well-known/microsoft-identity-association.json`.

## WilsoonID and JavaDashboard

MultiTasker can take its tasks from JavaDashboard instead of keeping them itself (**Settings → Task list**). It signs
in to WilsoonID and asks for tokens for JavaDashboard's API as a resource (RFC 8707), so it gets only what JavaDashboard
delegates to it.

WilsoonID matches redirect URIs exactly and keeps localhost callbacks out of production clients, so MultiTasker uses an
app link instead (RFC 8252 §7.1): the browser hands the sign-in to `dev.wilsoon.multitasker:/oauth2redirect`, which
Windows passes to MultiTasker. Installers register that link; development builds register it for the current user
when they start.

1. In WilsoonID's admin, create an application **MultiTasker**: a **public** client (no secret), redirect URI
   `dev.wilsoon.multitasker:/oauth2redirect`, scopes `openid profile email offline_access`. Marking it first-party
   skips the consent screen.
2. On **JavaDashboard**'s application page (its resource address is `https://dash.wilsoon.dev`), delegate to
   MultiTasker the keys `calendar.read`, `events.write`, `events.modify` and `events.delete`.
3. Put its `cl_…` client ID in `.env` as `WILSOON_CLIENT_ID`.

JavaDashboard tasks have a due date but no due time, so MultiTasker hides the time field while it's the source.
In-progress marks and attachments are kept in JavaDashboard too, so they follow you to any computer signed in to it;
Sync pauses and leaves Settings while JavaDashboard is the source, and picks up again when you go back to local tasks.

The **Task list** section only appears in builds made with `WILSOON_CLIENT_ID`. Public release builds leave it unset,
so people who can't use JavaDashboard never see the option.

## Using the IDs

**While developing**, copy [`.env.example`](../.env.example) to `.env` at the repository root and fill it in:

```sh
MULTITASKER_GOOGLE_CLIENT_ID=1234-abc.apps.googleusercontent.com
MULTITASKER_GOOGLE_CLIENT_SECRET=GOCSPX-...
MULTITASKER_MICROSOFT_CLIENT_ID=00000000-0000-0000-0000-000000000000
WILSOON_CLIENT_ID=cl_...
```

`.env` is git-ignored. The values are read when the app is compiled, so builds made this way sign in with no setup;
saving `.env` makes the next `npm run tauri dev` rebuild with them. Environment variables of the same names take
priority over `.env`, so for release builds set them as CI secrets instead.

**Without rebuilding**, choose Google Drive or OneDrive in **Settings → Sync** on a build that has no IDs, and paste
them there. The client ID goes in `settings.json`; Google's client secret goes in Windows Credential Manager.

**Forks** should register their own apps rather than reuse the official IDs.

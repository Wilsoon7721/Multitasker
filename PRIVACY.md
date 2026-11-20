# Privacy

MultiTasker is a desktop app. It has no server of its own, no accounts, no analytics and no tracking. Everything it
keeps stays on your computer, or in storage you choose to sync through.

## What it stores on your computer

- Your tasks, which tasks are in progress, and attachments (references to files, folders, links and programs you link
  to tasks; the files themselves are never copied).
- Its settings and an ID for your computer, so synced data can say which computer something happened on.

These live in MultiTasker's app data folder (`%APPDATA%\dev.wilsoon.multitasker` on Windows).

## Activity detection

If you leave activity detection on, MultiTasker reads the title and program of the window in front, and watches the
files and folders you've attached to tasks for saves. This is only used, in memory, to show which task you're working
on. Window titles are never stored or sent anywhere. You can turn it off in Settings or from the tray.

## Sync

Sync is off until you turn it on. When it's on, your tasks, what's in progress and attachments are written to the
place you choose, and nowhere else:

- **Google Drive**: a hidden app-data folder in your own Drive. MultiTasker asks only for access to that folder
  (`drive.appdata`) and your email address (to show which account is signed in). It can't see any of your other files.
- **OneDrive**: the `Apps/MultiTasker` folder in your own OneDrive (`Files.ReadWrite.AppFolder`), plus your email
  address and name for the same reason. It can't see any of your other files.
- **A shared folder** or **an S3-compatible bucket** that you set up and control.

Data from Google and Microsoft is used only to sync MultiTasker between your own computers. It isn't shared with
anyone, used for advertising, or used to train AI models. MultiTasker's use of information received from Google APIs
adheres to the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements.

Sign-ins and secret keys are kept in Windows Credential Manager. Signing out in Settings (or choosing another way to
sync) removes them from your computer. You can also revoke MultiTasker's access at any time from your
[Google account](https://myaccount.google.com/permissions) or [Microsoft account](https://account.live.com/consent/Manage),
and delete its data by removing the `Apps/MultiTasker` folder in OneDrive or, for Google Drive, under
**Settings → Manage apps** in Drive.

## Contact

Questions or concerns: open an issue at https://github.com/Wilsoon7721/Multitasker/issues.

# Online task lists

MultiTasker keeps tasks on your computer by default. **Settings → Task list** can take them from an online task list
instead. While one is in use, MultiTasker keeps its extras (what's in progress, and the files, links and apps attached
to tasks) in that list too, so every computer signed in to it sees them; **Sync** pauses until you go back to tasks on
this computer, which stay saved in the meantime.

## Todoist

Todoist connects with a personal API token, so there's no app to register.

1. In Todoist, open **Settings → Integrations → Developer** and copy your **API token**.
2. In MultiTasker, choose **Todoist** under **Settings → Task list**, paste it and select **Connect**.

The token is kept in Windows Credential Manager and forgotten when you switch away from Todoist. Anyone with it can
read and change all your Todoist tasks, so treat it like a password; you can issue a new one on the same page.

How tasks map:

| MultiTasker | Todoist |
| --- | --- |
| Category | Project (none is the Inbox; a new category makes a new project) |
| Due date and time | Due date, with a time when one is set |
| In progress | An `in-progress` label, plus a "Started on (computer)" comment recording when and where |
| Attachments | Comments starting with 📎, each ending with a `MultiTasker · …` line that it reads them back by |

Adding or removing the `in-progress` label in Todoist itself starts or stops the task in MultiTasker too. Tasks
finished in the last two weeks show as done.

To run the provider's live test against your own account (it creates and then deletes a task and a project):

```powershell
$env:TODOIST_TOKEN = "<your token>"; cargo test -p multitasker-provider-todoist -- --ignored
```

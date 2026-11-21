# Online task lists

MultiTasker keeps tasks on your computer by default. **Settings → Task list** can take them from an online task list
instead. While one is in use, MultiTasker keeps its extras (what's in progress, and the files, links and apps attached
to tasks) in that list too, so every computer signed in to it sees them; **Sync** pauses until you go back to tasks on
this computer, which stay saved in the meantime. Work handed in for its tasks is uploaded to it too, so it's on every
computer signed in to the list; MultiTasker keeps a local copy and fetches ones handed in elsewhere when needed.
JavaDashboard has no file storage, so work handed in for its tasks stays on the computer it was handed in on.

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
| Handed-in work | A "📦 Handed in" comment with the file attached (Todoist's file size limit applies) |

Adding or removing the `in-progress` label in Todoist itself starts or stops the task in MultiTasker too. Tasks
finished in the last two weeks show as done.

To run the provider's live test against your own account (it creates and then deletes a task and a project):

```powershell
$env:TODOIST_TOKEN = "<your token>"; cargo test -p multitasker-provider-todoist -- --ignored
```

## Notion

Notion connects with an internal integration's token, and only sees the databases you share with that integration.

1. In Notion, open **Settings → Connections → Develop or manage integrations**, create an internal integration for your
   workspace and copy its token.
2. Open your task database (or the page it's in), choose **••• → Connections** and add the integration.
3. In MultiTasker, choose **Notion** under **Settings → Task list**, paste the token, select **Find databases**, pick
   yours and select **Connect**.

How tasks map, using the database's own columns where it has them:

| MultiTasker | Notion |
| --- | --- |
| Done and in progress | The Status column's Complete and In progress groups, or a Done checkbox |
| Due date and time | A date column such as Due (added if there isn't one) |
| Category | A select column such as Category (added if there isn't one) |
| Description | A text column such as Description or Notes (added if there isn't one) |
| When and where it started | Started (date) and Started on (computer name), added on connecting |
| Attachments | A Files & media column such as Attachments; files and folders are kept as `file:///` links |
| Handed-in work | A file in the task's page, captioned "Handed in from (computer)"; the workspace's upload limit applies (5 MB on the free plan) |

Moving a task to In progress in Notion itself starts it in MultiTasker too. Files you upload to Notion show up as
links. Deleting a task moves its page to Notion's trash.

To run the provider's live test against a database of your own (it adds and then trashes one page):

```powershell
$env:NOTION_TOKEN = "<token>"; $env:NOTION_DATABASE = "<data source id>"; cargo test -p multitasker-provider-notion -- --ignored
```

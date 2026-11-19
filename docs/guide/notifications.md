# Notifications & tray

RepoHarbor keeps watch in the background so you don't have to keep checking. It polls your git hosts on a timer and surfaces anything that needs you as a desktop notification and a tray glance.

## What you get notified about

In **Settings → Notifications & Startup**, toggle each event independently:

- **Background notifications** — master switch for desktop notifications.
- **Urgent attention items** — review requests and high-priority items.
- **New pull requests** — a PR you should know about appears.
- **Review requested** — someone asked for your review.
- **CI failure** — a check suite goes red.
- **Agent finished** — a dispatched coding agent finished with commits to review.
- **Launch automatically on desktop login** — writes or removes `~/.config/autostart/com.digitscode.repoharbor.desktop`, plus a one-click **Install / repair desktop shortcut & icon** button.

Notifications are de-duplicated, so you're told about each thing once. On first run RepoHarbor seeds what's already there silently, so you don't get a burst of notifications for history the moment you enable it.

## System tray & window controls

- **Close (`✕`)** quits RepoHarbor cleanly.
- **Minimize (`−`)** hides the window while keeping the tray icon and background poller active.

The tray icon doubles as a quick glance:

- A summary of what currently needs attention.
- Shortcuts to recently used repos.
- **Quit** to exit from the tray menu.

::: tip Summon it back
Global-shortcut registration is disabled so cold start does not pop the desktop “add a shortcut” portal dialog. Use the tray or Dash / taskbar pin to raise the window; bind a summon hotkey in your desktop’s global-shortcuts settings if you want one.
:::

## todone

Todone - the opposite of todo.

A terminal app for tracking what you did each workday. Entries are persisted to act as a searchable log of your work history. Uses vim-style keybinds. Optional GitLab and calendar integrations show your activity as context alongside what you're writing.

## Install

### Nix

Run without installing:

```
nix run github:matthewtapps/todone
```

Install into your profile:

```
nix profile install github:matthewtapps/todone
```

Or add it as a flake input.

### Non-Nix

Linux (X11 and Wayland) and macOS are supported; Windows is not.

Prebuilt binary (installs to `~/.cargo/bin` by default):

```
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/matthewtapps/todone/releases/latest/download/todone-installer.sh | sh
```

Via `cargo binstall` (no compilation):

```
cargo binstall todone
```

Or build from source. Needs a Rust toolchain (stable, edition 2024):

```
git clone git@github.com:matthewtapps/todone.git
cd todone
cargo install --path .
```

## Usage

Run `todone`. You get two stacked panes (yesterday's "did" and today's "planning") and a context pane showing planning notes, GitLab activity, or calendar events.

The status line at the bottom shows the current mode and contextual hints. Press `?` anywhere for the full keybind list.

Quit with `q` or `Ctrl-Q`; both save first. `:w` saves without quitting.

## Data locations

- Entries: `~/.local/share/todone/entries.json`
- Config: `~/.config/todone/config.toml` (stored chmod 600 since it holds a PAT)

Both files are plain JSON/TOML. Back them up however you back up the rest of your dotfiles.

## Integrations

Open settings with `<Space>s`. Both integrations are off by default.

### GitLab

Pulls your events for each viewed day (pushes, comments, MR opens/merges/approvals/closes) and groups them by project in the context pane. Used as a memory aid when filling in "did".

You need:

- **Instance URL**: e.g. `https://gitlab.com` or your self-hosted instance.
- **Personal access token**: create one at `<instance>/-/user_settings/personal_access_tokens` with the `read_api` scope. No expiry is fine for personal use; pick whatever your org policy allows.
- **Username**: your GitLab username (the handle, not the display name).

### Calendar

Shows today's meetings in the context pane, parsed from any iCalendar (`.ics`) feed.

- **Outlook / Office 365**: open Outlook on the web, go to Settings > Calendar > Shared calendars > Publish a calendar. Pick the calendar, set permission to "Can view all details", click Publish, then copy the ICS link (not the HTML one).
- **Google Calendar**: open calendar settings, select the calendar under "Settings for my calendars", scroll to "Integrate calendar", and copy the "Secret address in iCal format".

Paste the URL into the `ics_url` field in settings.

## Clipboard yanks

In addition to general clipboard support for vim-mode yanks, there are leader shortcuts that copy formatted output:

- `yt`: HTML formatted for pasting into Microsoft Teams (yesterday/today sections, bullets, nested sub-items). A plain-text alternative is included for clients that don't accept HTML.
- `yx`: plain text, newline-separated, no bullet markers. Useful for time-tracking tools like Xero that don't render lists.
- `yd`: yesterday's "did" as plain bullets.
- `yp`: today's "planning" as plain bullets.

## History

Press `<Space>h` to open the history view. Move with `j`/`k` (or `22j` to jump 22 days), `gg` for earliest recorded day, `G` for today. `Enter` loads the selected day as the new "yesterday".

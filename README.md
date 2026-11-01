# Orange

Orange is a messaging app. This fork keeps the desktop GUI and adds a headless `orange` command-line interface so scripts can use the same account.

Running `orange` with no arguments still opens the GUI (`cargo piston dev linux` is unchanged). Only a recognized subcommand, or `--help`, enters the CLI. Any other argument is left to the existing GUI. Subcommands never open a window.

## Command-line interface

Build on Linux without a display. Keep the committed `Cargo.lock`.

```sh
cargo build --locked --bin orange
cargo test --locked --lib
./target/debug/orange help
```

The CLI uses the GUI's data directory: `$XDG_DATA_HOME/Orange` when that variable is an absolute path, otherwise `~/.local/share/Orange`. `--data-dir` and `ORANGE_DATA_DIR` select that folder (the one that contains `CACHE` and `DEVICE_ID`). A new directory creates an identity; an existing GUI directory reuses it.

```sh
orange --offline whoami
orange --data-dir "$HOME/.local/share/Orange" rooms --json
```

Global flags: `--json`, `--data-dir PATH`, `--timeout SECONDS` (default 20), `--offline`, `--verbose`.

| Command | What it does |
| --- | --- |
| `whoami`, `invite` | Public name, username, invite link, data directory |
| `rooms` | Chats with ids, names, unread counts, last preview |
| `read <room> [--since T] [--limit N]` | History. `--since` is unix seconds or RFC3339. Does not mark read |
| `send <room> <text>` | Send text |
| `dm <name-or-key> <text>` | Open the direct chat (or the existing one) and send |
| `send-media <room> <file> [--caption TEXT]` | Photo or video. A caption is the following text message |
| `send-gif <room> <query-or-id> [--index N] [--search]` | Search the same Giphy service the GIF picker uses, then send |
| `gif search <query>` | List matches without sending |
| `download <room> <message-id> [--out PATH]` | Save a photo, video, sticker, or GIF |
| `mark-read <room>` | Advance this identity's read cursor. `watch` does not do this |
| `watch [room] [--include-own]` | Newline-delimited JSON for messages that arrive while it runs |
| `contacts`, `contact add <name-or-link>` | Local contact book. Accepts an Orange name or invite URL |
| `profile show`, `profile set --username --notes --avatar` | The same profile fields as the profile screen |
| `group create <name> --member <who> --member <who>` | New group. Two people besides you; one person is a DM |
| `group add <room> --member <who>` | Add a member and share existing videos |
| `group leave <room>`, `group remove <room>` | Leave. Orange has no kick |
| `group rename <room> <name>`, `group photo <room> <file>` | Group name and photo |
| `delete <room> <message-id>` | Delete your message, or hide someone else's from you |
| `block <name>`, `unblock <name>`, `blocks` | The same block record as the profile screen |
| `typing <room> [--stop]` | Typing activity, without sending draft text |
| `notify <room>` | Show the local push flag. Does not change the listener's watches |
| `stickers`, `sticker add <image>`, `send-sticker <room> <index>` | Personal sticker pack |

Rooms accept the chat id from `rooms`, a group name, a contact username, or an Orange name. People accept an Orange name, an `https://orange.me/invite.html?name=…` link, or a contact username.

```sh
orange whoami
orange contact add https://orange.me/invite.html?name=<66-hex-name>
orange dm <name> "lunch tomorrow?"
orange rooms --json
orange read <room-id> --limit 20 --since 2026-11-01T00:00:00Z
orange send <room-id> "on my way"
orange send-media <room-id> photo.png --caption "the door"
orange send-gif <room-id> "orange cat" --index 0
orange download <room-id> <message-id> --out ./saved.jpg
orange mark-read <room-id>
orange group create "Trip" --member <name-a> --member <name-b>
orange profile set --username BrightMango --notes "around"
```

`--json` prints one JSON document. `confirmed` is true after the Orange network acknowledges a send.

### In the desktop window

Ctrl+Alt+Shift+O opens a console over the running app (Ctrl+Option+Shift+O on macOS). It takes the same commands against the account that is already signed in. Esc, or the shortcut again, closes it. The normal screens have no new buttons, menus, or layout; the panel is drawn only while the console is open.

### `orange watch`

Stdout is newline-delimited JSON, one object per message that arrives after the command starts. History is not replayed; use `read` for that. Diagnostics go to stderr. `--json` is implied. `--include-own` also prints messages from this identity. An optional room argument filters to one chat.

```sh
orange watch --data-dir "$HOME/.local/share/Orange"
orange watch <room-id>
```

```json
{"event":"message","room":"<chat id>","sender":"<sender public key>","sender_username":"BrightMango","timestamp":1793568679906189289,"id":"<message id>","content":{"type":"text","text":"hello"}}
```

`timestamp` is the same nanosecond value the app stores. `content.type` is `text`, `photo`, `video`, `gif`, `sticker`, or `system`. Media objects carry metadata (dimensions, byte size, GIF id and URL, video clip id and duration), not the file bytes. `download` saves the file.

### orange-listener.service

Leave `orange-listener.service` running. It is the always-on path that wakes bots when a cloud computer is suspended. This CLI does not install, stop, restart, mask, or rewrite that unit.

`orange watch` is an extra reader on the same identity, not a replacement:

- It does not take SQLite's exclusive lock. The GUI opens `CACHE` with `locking_mode=EXCLUSIVE`. Watch copies that file (and its WAL) into a private temp directory and reads the copy, so it can run beside the listener on the same data directory.
- It does not write the shared cache, so it does not move the listener's read cursors.
- It does not mark chats read.
- It does not send `Notification::Received`, `Notification::Watch`, or `Notification::Remove`. Those are the acknowledgements the listener uses to arm and retire wakeups. Consuming them would swallow the push the listener is supposed to deliver.
- It only issues AIR reads. Queued writes already in the cache stay there for the listener.

Other commands write the local cache with a normal lock, never an exclusive one. If the listener or the GUI already holds the cache exclusively, those commands fail and tell you to use `watch` or another `--data-dir`. They still do not acknowledge notification watches.

### What the GUI can do that this CLI does not

- Reactions, replies, and edits are not in the chat contract, so there is no command for them.
- A group member cannot remove someone else. `group remove` leaves.
- Push registration stays with the GUI and `orange-listener.service`. `notify` only reports the local flag.
- Wallet and Bitcoin screens are unchanged and have no CLI commands.
- A local GIF file is sent as a still photo, matching the attachment picker. Animated GIFs are the Giphy messages from `send-gif`.
- `GIPHY_API_KEY` in `.env` or the environment is required for search, same as the app. The key is read at build time and, for the CLI, again at runtime.

### Checking two identities

`cargo test --locked --lib -- --ignored two_identities` creates two data directories, sends text, a photo, and a GIF through the Orange network, reads them from the other identity, saves the photo, and checks that `watch` prints a newline-delimited message with `event`, `room`, `sender`, `content`, and `timestamp`. It is ignored by the default test run so existing CI does not depend on the network. Offline tests cover the cache lock and the watch JSON shape.

```sh
alice=$(orange --data-dir /tmp/orange-alice --offline whoami --json | jq -r .name)
bob=$(orange --data-dir /tmp/orange-bob --offline whoami --json | jq -r .name)
orange --data-dir /tmp/orange-alice dm "$bob" "hello"
orange --data-dir /tmp/orange-bob read "$alice" --json
orange --data-dir /tmp/orange-bob watch
```

## GIF configuration

Put `GIPHY_API_KEY=your_key_here` in `.env` at the Orange repository root
(beside `Cargo.toml`). This file is ignored by Git; `.env.example` is a template.
Orange's Cargo build script reads the key for both Xcode and Cargo builds, and
passes it to CHK's shared GIF service at startup. Rebuild after changing `.env`;
the mobile app does not need access to the file at runtime. An exported
`GIPHY_API_KEY` overrides the file value.

## Frontend setup

Cargo retrieves CHK 2.0.0, Pelican UI 3.0.0, and PTSD 0.2.0 from pinned Git revisions.
Backend dependencies use published Ramp 3.0.4, Maverick 5.0.2, and AIR Notifications 0.2.3.
This includes native text autocorrection and notification registration recovery. The older
Ramp 3.0.0 copy in `vendor/ramp2` is not used by this dependency graph.

```sh
git clone git@github.com:ramp-stack/orange.git
cd orange
cargo check --locked
cargo install cargo-piston --version 0.5.8 --locked --force
```

Keep the committed `Cargo.lock`; no `cargo update` is needed. Clone the other frontend
repositories only when editing them. Use local Cargo overrides for related development and
keep those overrides out of release manifests.

In the local `/Users/ellamae/ramp` workspace, the parent `.cargo/config.toml` patches
CHK, Pelican UI, and PTSD to their sibling checkouts. This preserves local UI edits
while using the published backend fixes. Run Cargo from inside this workspace; local
lockfiles reflect these path overrides.

Restore the complete Cargo Piston identity backup supplied privately by Caleb with
`cargo piston restore "<complete backup line>"`. Then run `cargo piston setup` in
`orange` and select team `RYYG6X8WK7`. Keep the backup out of Git, screenshots, and
build logs. The manifest contains only the team's public identity.

Choose one development platform per command:

```sh
cargo piston dev macos
cargo piston dev ios
cargo piston dev android
cargo piston dev linux
```

For a connected iPhone, unlock it and enable Developer Mode. Piston builds, signs,
installs, and streams logs. It uses the existing `org.ramp.orange` identifier.
Press Ctrl-C to end the development session. Xcode and Apple account sign-in are
required for iOS development; Piston reports missing prerequisites. If several
devices match, select a number from Piston's device list, or enter `q` to cancel.
Use `--device "<device name or UDID>"` to select one explicitly, including in scripts.

For the Pelican component studio, clone `pelican_ui` and run from that checkout:

```sh
cargo run --locked --features demo --bin pelican_ui
```

Send setup or build issues to `caleb@orange.me` with your OS, the command, repository
commit IDs, and relevant error output with secrets removed. Frontend changes can
be pushed directly. Backend changes go through pull requests reviewed by Caleb.

## Application resources

Piston generates the iPhone/iPad bundle from the Cargo configuration. The minimum
supported iOS version is 17.0.

- `icon.png` is the original Orange icon; Piston generates its required sizes.
- `ios/OrangeLaunchScreen.storyboard` and `ios/OrangeLaunchWordmark.png` preserve the black launch
  screen with the centered Orange wordmark.
- `ios/Info.plist` contains Orange's display name, dark appearance, camera/photo
  explanations, orientations, and background declarations.
- `package.metadata.piston.ios` selects the signing team, bundle identifier,
  plist, and launch resources.

The legacy `orange/` Xcode project remains for reference. Cargo Piston uses the
configuration above. Development commands do not upload to TestFlight.

## Chat notifications

Orange enables notifications for joined chats at startup and newly received chats through
its AIR service, independently of the selected tab. Leaving a group disables its notifications,
and rejoining enables them. Creating a chat enables notifications immediately. Ramp persists
the notification flag, while Orange reapplies membership-based notification settings at startup;
the generic instance default remains disabled.

This update uses fresh AIR instance caches. Existing server history is loaded again, but unsent
work in the previous instance caches is not recovered. There is no cache migration; the saved
AIR identity is retained.

Build through Cargo Piston after restoring the team's developer identity. Piston retrieves
notification credentials from the encrypted AIR developer contract and configures the selected
Apple platform. iPhone development uses sandbox APNs; TestFlight uses production APNs and
requires its own notification configuration.

On October 29, 2026, real APNs sandbox delivery passed on an iOS 26.3 simulator: an incoming
chat message produced a native notification with Orange stopped; tapping it reopened Orange,
loaded the message, and rearmed its watch. This did not validate production APNs or a physical
iPhone. Piston's simulator flow currently skips remote notification setup; that test built and
bundled Orange directly with the development notification configuration.

## iOS FFmpeg libraries

Ramp 3.0.4 uses published Maverick 5.0.2 and the shared `media2` 0.1.1 crate. Media requires
`ramp-ffmpeg` 9.0.4 or later and `ramp-ffmpeg-sys` 9.0.5 or later. `Cargo.lock` selects those releases
and the 9.0.2 binary packages from crates.io. Cargo selects separate precompiled libraries for ARM64
devices, ARM64 simulators, and Intel simulators. These builds do not compile FFmpeg
or NASM or run Make. The legacy Xcode project selects the matching Rust static
library for each SDK, architecture, and Debug/Release configuration.

Update the Orange checkout and use its committed lockfile on other machines. The dependency
requirements prevent selecting the old sys 9.0.4 source build; no local backend checkout,
FFmpeg fork, or separate FFmpeg update command is required.

## TestFlight

```sh
cargo piston deploy testflight
```

This uses the same Orange identifier, team, icon and launch resources as development.
It builds for distribution and waits for Apple's processing result. No App Store
screenshots are needed. Piston prompts for missing App Store Connect credentials, as described in
[Piston's documentation](https://github.com/CalebCouch/cargo-piston). Select Xcode 26 or later
with the iOS 26 SDK or later. The command reports missing prerequisites before building.

Use `cargo piston deploy testflight --prepare-only` to check setup, or add `--setup`
to replace the saved account configuration. Deployment displays progress and stays
running until the build is ready or fails. After an interruption, rerun the same
command to resume an existing upload without building or uploading again. Increase
`package.version` before each new binary upload; both Apple version fields use its
numeric version. Piston keeps waiting during export-compliance steps and opens the
TestFlight page when answers are required. Tester-group setup is separate.

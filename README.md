# Orange

## Frontend setup

Orange 1.0.1 uses CHK 2.0.0, Pelican UI 3.0.0, and PTSD 0.2.0 from Git. Cargo downloads
published Ramp 3.0.3, Prism 3.1.0, and their backend dependencies from crates.io.
A single Orange checkout is enough to build the application.

```sh
git clone git@github.com:ramp-stack/orange.git
cd orange
cargo check --locked
cargo install cargo-piston --version 0.5.8 --locked --force
```

To edit another frontend crate, clone its repository and use a local Cargo override while
working. Keep local overrides out of release manifests. The committed lockfile pins the Git
revisions used by this release.

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
- `ios/LaunchScreen.storyboard` and `ios/wordmark.png` preserve the black launch
  screen with the centered Orange wordmark.
- `ios/Info.plist` contains Orange's display name, dark appearance, camera/photo
  explanations, orientations, and background declarations.
- `package.metadata.piston.ios` selects the signing team, bundle identifier,
  plist, and launch resources.

The legacy `orange/` Xcode project remains for reference. Cargo Piston uses the
configuration above. Development commands do not upload to TestFlight.

## Chat notifications

Orange enables notifications for existing chats at startup and newly received chats through
its AIR service, independently of the selected tab. Creating a chat enables notifications
immediately. Ramp saves the notification flag when changed and restores it with the instance;
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

Ramp 3.0.3 uses published Maverick 5.0.1 and the shared `media2` 0.1.1 crate. Media requires
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

# Orange

## GIF configuration

Put `GIPHY_API_KEY=your_key_here` in `.env` at the Orange repository root
(beside `Cargo.toml`). This file is ignored by Git; `.env.example` is a template.
Orange's Cargo build script reads the key for both Xcode and Cargo builds, and
passes it to CHK's shared GIF service at startup. Rebuild after changing `.env`;
the mobile app does not need access to the file at runtime. An exported
`GIPHY_API_KEY` overrides the file value.

## Frontend setup

Frontend work lives in four sibling repositories: `orange`, `chk`, `pelican_ui`, and
`ptsd`. Caleb owns the backend crates; Cargo downloads their published releases.
Use the current default branches of all four frontend repositories together. This revision uses
Prism 3.0.0 and WGPU Prism 8.0.0, including the updated layout, native keyboard, media,
and gesture interfaces. No backend source checkout is required.

Orange temporarily patches the published Ramp 3.0.0 crate with the copy in
`vendor/ramp2` to keep the iOS keyboard open when switching text fields and
present placeholder frames before deferred UI work such as QR generation.
`vendor/ramp2/PATCHES.md` describes the fix; Xcode and Cargo both use this copy.

```sh
mkdir orange-frontend
cd orange-frontend
git clone git@github.com:ramp-stack/orange.git
git clone git@github.com:ramp-stack/chk.git
git clone git@github.com:ramp-stack/pelican_ui.git
git clone git@github.com:ramp-stack/ptsd.git
cd orange
cargo check --locked
cargo install cargo-piston --version 0.5.7 --locked --force
```

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

For the Pelican component studio, run from the sibling `pelican_ui` checkout:

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

After changing these launch resources, rerun `cargo piston dev ios` to regenerate
the app bundle. The generated Xcode signing project under `target/piston/` copies
the staged bundle; rebuilding that project alone does not rebuild these resources.

## iOS FFmpeg libraries

Ramp 3.0.0 uses published Maverick 5.0.0 and the shared `media2` 0.1.0 crate. Media requires
`ramp-ffmpeg` 9.0.4 or later and `ramp-ffmpeg-sys` 9.0.5 or later. `Cargo.lock` selects those releases
and the 9.0.2 binary packages from crates.io. Cargo selects separate precompiled libraries for ARM64
devices, ARM64 simulators, and Intel simulators. These builds do not compile FFmpeg
or NASM or run Make. The legacy Xcode project selects the matching Rust static
library for each SDK, architecture, and Debug/Release configuration.

Update Orange and its sibling frontend repositories together on other machines.
The dependency requirements prevent selecting the old sys 9.0.4 source build;
no separate FFmpeg update command or local Maverick/FFmpeg checkout is required.

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

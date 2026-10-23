# Orange

Connect and unlock your iPhone, then run from this directory:

```sh
cargo piston dev
```

Piston builds the app, signs it with the orange me LLC team, installs it on the
connected iPhone, and streams its logs in the terminal. It reuses the existing
`org.ramp.orange` app identifier. Press Ctrl-C to stop the development session.

Development uses the sibling checkouts listed in `Cargo.toml`: Ramp, CHK,
Pelican UI, PTSD, Prism, Maverick, Air, Air Server, and WGPU Prism. Cargo resolves
the remaining dependencies through `Cargo.lock`.

Piston generates the iPhone/iPad bundle and both version fields from the Cargo
package version. The minimum supported iOS version is 17.0.

- `icon.png` is the original Orange icon; Piston generates its required sizes.
- `ios/LaunchScreen.storyboard` and `ios/wordmark.png` preserve the black launch
  screen with the centered Orange wordmark.
- `ios/Info.plist` contains Orange's display name, dark appearance, camera/photo
  explanations, orientations, and existing background declarations. Background
  declarations alone do not implement background tasks or enable push signing.
- `package.metadata.piston.ios` selects the signing team, bundle identifier,
  plist, and launch resources.

The legacy `orange/` Xcode project remains for reference. `cargo piston dev`
uses the configuration above and does not build that project or upload to
TestFlight.

## iOS FFmpeg libraries

Ramp requires published Maverick 4.0.0, which requires `ramp-ffmpeg` 9.0.4 or later
and, through it, `ramp-ffmpeg-sys` 9.0.5 or later. `Cargo.lock` selects those releases
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
screenshots are needed. Piston prompts for missing App Store Connect credentials and remembers them per project, as described in
[Piston's documentation](../cargo-piston/README.md), and select Xcode 26 or later
with the iOS 26 SDK or later. The command reports missing prerequisites before building.

Use `cargo piston deploy testflight --prepare-only` to check setup, or add `--setup`
to replace the saved account configuration. Deployment displays progress and stays
running until the build is ready or fails. After an interruption, rerun the same
command to resume an existing upload without building or uploading again. Increase
`package.version` before each new binary upload; both Apple version fields use its
numeric version. Piston keeps waiting during export-compliance steps and opens the
TestFlight page when answers are required. Tester-group setup is separate.

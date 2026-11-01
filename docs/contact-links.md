# Contact invitations

The self profile has **Share** under the **Orange name** data item. The QR page's
**Share** action shares the same invitation link; **Copy** still copies the raw name.
It shares `https://orange.me/invite.html?name=<public Orange name>`.
Only the public identity is shared; the link contains no private key or credentials.

Opening the link in an updated iOS app uses the existing CHK direct-message action,
including its existing-room lookup. It does not send a message automatically.
New-contact forms also accept the full invitation URL or a raw Orange name.
The website's **Open in Orange** button uses `orange-contact://contact/<name>`.
Someone who installs the app afterwards must tap the invitation again.

## iOS release setup

- Both Xcode and cargo-piston Info.plist files register the `orange-contact` scheme.
- Both signing paths include associated domains `applinks:orange.me` and
  `applinks:www.orange.me`.
- The Apple App ID `org.ramp.orange` and the provisioning profile used for the next
  release must support Associated Domains. Regenerate the profile if necessary.
- The hosted association file identifies `RYYG6X8WK7.org.ramp.orange` and permits
  only `/invite.html` with a `name` query parameter.
- Existing TestFlight installations need a new app build to receive this feature.

Orange installs a forwarding UIKit application delegate at launch, preserving the
existing runtime delegate and notification delegate chain. Incoming links queue
until the UI is ready. Runtime and UI dependency crates are not modified.

## Verification (2026-11-01)

- `cargo test --locked --offline --lib`: 2 tests passed.
- `cargo check --locked --offline --lib --target aarch64-apple-ios`: passed.
- `cargo build --locked --offline --bin orange --target aarch64-apple-ios-sim`: passed.
- iOS 18.5 simulator: cold-start custom-scheme invitation opened the expected DM
  in an isolated test app without sending a message.
- iOS 27 simulator startup stopped at UIKit's scene-lifecycle requirement, before
  invitation handling could be tested. No runtime dependency changes were made.
- A signed physical-device HTTPS universal-link test remains necessary with the
  new Associated Domains provisioning profile. Simulator custom-scheme testing
  does not prove Apple's domain association or release signing.

Website source and deployment instructions are in the adjacent `orangeme_web`
repository, `deploy/contact-links.md`.

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

Orange owns `chk::Invitations<Name>` as a normal field. Its format, parser, and
DM-opening handler are methods on `Orange` in `src/lib.rs`; there is no global
invitation instance. Cloned handles share the same CHK inbox across the UI and
native receiver. CHK queues delivery until the UI is ready and invokes Orange's
handler; other apps supply their own payloads and handlers.

The iOS entry point holds an `AppLinks::prepare()` guard around runtime startup.
Maverick temporarily queues URLs received before Orange exists. Once constructed,
Orange attaches a closure with `AppLinks::listen`, which drains those URLs into
its own inbox. The forwarding delegate preserves the runtime and push callbacks.

The local `feature/apple-app-links` branches in Orange and Maverick OS are paired:
Orange's crates.io patch selects `../maverick_os` until the bridge is released.

## Maverick extraction verification (2026-11-01)

- `cargo test --locked --offline --lib`: 2 tests passed, including queuing links
  before the UI is ready and rejecting invalid invitation URLs.
- `cargo check --locked --offline --lib --target aarch64-apple-ios`: passed.
- `cargo build --locked --offline --lib --target aarch64-apple-ios`: passed with
  the local Maverick bridge. Device link-opening and notification delivery were
  not rerun after this extraction.

## Verification before the Maverick extraction (2026-11-01)

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

Based on the published `ramp2` 3.0.0 crate.

`src/native_text/ios.rs`: while another editable text view is focused, defer
resigning the previous first responder. UIKit transfers first responder status
directly when the new view becomes active, keeping the keyboard open when moving
between fields. Explicit blur and keyboard dismissal retain their existing paths.

`src/lib.rs` and `src/renderer.rs`: wait for submitted drawing instructions to be
presented before polling the next component change. This lets navigation display
the temporary QR image before QR generation runs on the UI thread. Unchanged
frames acknowledge immediately when there is no pending presentation.

Remove Orange's Cargo patch when an upstream release includes these fixes.

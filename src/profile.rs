use chk::{Action, ActionItem, Bumper, Context, Display, Flow, Icons, Offset, Page, PageType, Root};
use chk::air::profiles::Profile;

pub struct ProfileHome;
impl ProfileHome {
    pub fn build(ctx: &mut Context) -> Root {
        let mut profile = Profile::me(ctx);
        let mut page = Page::profile(ctx, &mut profile);
        if let Page::Static(page) = &mut page {
            if let PageType::EditAndDisplay { display, .. } = page.as_mut() {
                let name = ctx.air.me().to_string();
                let share = Action::share(&crate::contact_links::contact_url(ctx.air.me()));
                for item in display {
                    if let Display::Cta {label, actions, ..} = item && label == "Orange name" {
                        *actions = contact_actions(&name, share.clone());
                    }
                }
            }
        }
        Root::custom(page)
    }
}

fn contact_actions(name: &str, share: Action) -> Vec<ActionItem> {
    let qr = PageType::display(
        "Share name", vec![Display::qr_code(name, "Scan to share orange name.")], None,
        Bumper::custom("Share", share.clone()), Offset::Center,
    );
    vec![
        ActionItem::new_with_active(Action::copy(name), "Copy", "Copied", Icons::Copy),
        ActionItem::new(Action::flow(Flow::new(vec![Page::Static(Box::new(qr))])),
            "Display QR Code", Icons::QrCode),
        ActionItem::new(share, "Share", Icons::Send),
    ]
}

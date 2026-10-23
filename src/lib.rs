use chk::{App, Assets, RootInfo, ChkTheme, AvatarIconStyle, Context, Icons, AvatarContent, Color, Theme};
use ramp::prism;
use prism::drawable::Drawable;

mod messages;
mod contacts;
mod profile;

ramp::run!(Orange);

pub struct Orange;

impl prism::Application for Orange {
    fn name() -> String {"Orange".into()}

    fn new(ctx: &mut Context) -> impl Drawable {
        Orange.build(ctx, Assets::new(vec![include_dir::include_dir!("$CARGO_MANIFEST_DIR/resources")]))
    }
}

impl App for Orange {
    fn roots(&self, ctx: &mut Context, theme: &Theme) -> Vec<RootInfo> {
        let messages_home = messages::MessagesHome::new();
        let contacts_home = contacts::ContactsHome::new();
        let profile_home = profile::ProfileHome::new(ctx);
        vec![
            RootInfo::icon(ctx, theme, Icons::Messages, "Messages", messages_home),
            RootInfo::icon(ctx, theme, Icons::Group, "Contacts", contacts_home),
            RootInfo::avatar(ctx, theme, AvatarContent::icon(Icons::Profile, AvatarIconStyle::Secondary),
                "Profile", profile_home),
        ]
    }

    fn theme(&self) -> ChkTheme {ChkTheme::Dark(Color::from_hex("#eb343a", 255))}
}

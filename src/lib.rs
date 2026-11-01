use chk::{App, Assets, RootInfo, ChkTheme, AvatarIconStyle, Context, Icons, AvatarContent, Color, Theme};
use ramp::prism;
use prism::drawable::Drawable;

mod messages;
mod contacts;
mod profile;
mod contact_links;
#[cfg(target_os = "ios")]
mod ios_links;

#[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
mod cli;
#[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
mod console;
#[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
pub use cli::{cli_requested, run as run_cli};

#[cfg(not(target_os = "ios"))]
ramp::run!(Orange);

#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn maverick_main() {
    // Install before UIKit finishes launching so cold-start invitations aren't lost.
    ios_links::prepare();
    ramp::maverick_os::MaverickOS::<ramp::Ramp<Orange>>::start();
}

pub struct Orange;

impl prism::Application for Orange {
    fn name() -> String {"Orange".into()}

    fn new(ctx: &mut Context) -> impl Drawable {
        ctx.air.start::<messages::MessagesHome>(());
        let assets = Assets::new(vec![include_dir::include_dir!("$CARGO_MANIFEST_DIR/resources")]);
        let theme = Orange.theme().to_pelican(assets.clone());
        let app = contact_links::ContactLinks::new(Orange.build(ctx, assets), theme.clone());
        // Desktop only. While closed, this wrapper forwards layout, drawing, and
        // input unchanged; it draws only after Ctrl+Alt+Shift+O.
        #[cfg(not(any(target_os = "ios", target_os = "android", target_arch = "wasm32")))]
        {console::Console::new(app, &theme)}
        #[cfg(any(target_os = "ios", target_os = "android", target_arch = "wasm32"))]
        {app}
    }
}

impl App for Orange {
    fn giphy_api_key(&self) -> Option<&str> {option_env!("GIPHY_API_KEY")}

    fn roots(&self, ctx: &mut Context, theme: &Theme) -> Vec<RootInfo> {
        let messages_home = messages::MessagesHome::build();
        let contacts_home = contacts::ContactsHome::build();
        let profile_home = profile::ProfileHome::build(ctx);
        vec![
            RootInfo::icon(ctx, theme, Icons::Messages, "Messages", messages_home),
            RootInfo::icon(ctx, theme, Icons::Group, "Contacts", contacts_home),
            RootInfo::avatar(ctx, theme, AvatarContent::icon(Icons::Profile, AvatarIconStyle::Secondary),
                "Profile", profile_home),
        ]
    }

    fn theme(&self) -> ChkTheme {ChkTheme::Dark(Color::from_hex("#eb343a", 255))}
}

use chk::{App, Assets, RootInfo, ChkTheme, AvatarIconStyle, Context, Icons, AvatarContent, Color, Theme};
use ramp::prism;
use prism::{air::Name, drawable::Drawable};

mod messages;
mod contacts;
mod profile;

#[cfg(not(target_os = "ios"))]
ramp::run!(Orange);

pub struct Orange {invitations: chk::Invitations<Name>}

impl Orange {
    fn new() -> Self {
        Self {invitations: chk::Invitations::new(Self::contact_url, Self::parse_invite, Self::open_contact)}
    }

    fn contact_url(name: Name) -> String {format!("https://orange.me/invite.html?name={name}")}

    fn parse_invite(input: &str) -> Option<Name> {
        let input = input.trim();
        let name = ["https://orange.me/invite.html?name=", "https://www.orange.me/invite.html?name=",
            "orange-contact://contact/"].iter().find_map(|prefix| input.strip_prefix(prefix))?;
        // Accept only the canonical compressed public key, never arbitrary URL content.
        if name.len() != 66 || !name.bytes().all(|c| c.is_ascii_hexdigit()) {return None;}
        let parsed: Name = name.parse().ok()?;
        (parsed.to_string() == name).then_some(parsed)
    }

    fn open_contact(ctx: &mut Context, theme: &Theme, name: Name) {
        chk::profiles::Profile::from_name(ctx, name);
        // Reuse existing DMs, including received rooms and self DMs; never send automatically.
        (chk::Action::message(name).get())(ctx, theme);
    }

    #[cfg(target_os = "ios")]
    #[unsafe(no_mangle)]
    pub extern "C" fn maverick_main() {
        // Install before UIKit finishes launching so cold-start invitations aren't lost.
        let _app_links = ramp::maverick_os::AppLinks::prepare();
        ramp::maverick_os::MaverickOS::<ramp::Ramp<Self>>::start();
    }
}

impl prism::Application for Orange {
    fn name() -> String {"Orange".into()}

    fn new(ctx: &mut Context) -> impl Drawable {
        ctx.air.start::<messages::MessagesHome>(());
        let assets = Assets::new(vec![include_dir::include_dir!("$CARGO_MANIFEST_DIR/resources")]);
        let app = Self::new();
        let invitations = app.invitations.clone();
        #[cfg(target_os = "ios")]
        {
            let receiver = invitations.clone();
            ramp::maverick_os::AppLinks::listen(move |url| receiver.receive(url));
        }
        app.build_with_invitations(ctx, assets, &invitations)
    }
}

impl App for Orange {
    
    fn giphy_api_key(&self) -> Option<&str> {option_env!("GIPHY_API_KEY")}

    fn roots(&self, ctx: &mut Context, theme: &Theme) -> Vec<RootInfo> {
        let messages_home = messages::MessagesHome::build(self.invitations.clone());
        let contacts_home = contacts::ContactsHome::build(self.invitations.clone());
        let profile_home = profile::ProfileHome::build(ctx, &self.invitations);
        vec![
            RootInfo::icon(ctx, theme, Icons::Messages, "Messages", messages_home),
            RootInfo::icon(ctx, theme, Icons::Group, "Contacts", contacts_home),
            RootInfo::avatar(ctx, theme, AvatarContent::icon(Icons::Profile, AvatarIconStyle::Secondary),
                "Profile", profile_home),
        ]
    }

    fn theme(&self) -> ChkTheme {ChkTheme::Dark(Color::from_hex("#eb343a", 255))}
}

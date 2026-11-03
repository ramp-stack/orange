use chk::{App, Assets, RootInfo, ChkTheme, Context,
    Color, Theme, Invitations};
use ramp::prism::air::Name;

ramp::run!(chk::Application<Orange>);

pub struct Orange;

impl App for Orange {
    type Invitation = Name;

    fn name() -> String {"Orange".into()}

    fn new(_ctx: &mut Context) -> Self {Self}

    fn assets(&self) -> Assets {
        Assets::new(vec![include_dir::include_dir!("$CARGO_MANIFEST_DIR/resources")])
    }

    fn invitations(&self, _ctx: &mut Context) -> Option<Invitations<Name>> {
        Some(Invitations::new(
            |name| format!("https://orange.me/invite.html?name={name}"),
            |input| {
                let name = ["https://orange.me/invite.html?name=", "https://www.orange.me/invite.html?name=",
                    "orange-contact://contact/"].iter().find_map(|prefix| input.strip_prefix(prefix))?;
                if name.len() != 66 || !name.bytes().all(|c| c.is_ascii_hexdigit()) {return None;}
                let parsed: Name = name.parse().ok()?;
                (parsed.to_string() == name).then_some(parsed)
            },
            |ctx, name| {
                chk::profiles::Profile::from_name(ctx, name);
                chk::presets::ChatPresets::message(name)
            },
        ))
    }

    fn roots(&self, ctx: &mut Context, theme: &Theme,
        invitations: Option<&Invitations<Name>>) -> Vec<RootInfo> {
        let invitations = invitations.expect("Orange configures contact invitations");
        vec![
            RootInfo::messages(ctx, theme, invitations),
            RootInfo::contacts(ctx, theme, invitations),
            RootInfo::profile(ctx, theme, invitations),
        ]
    }

    fn theme(&self) -> ChkTheme {ChkTheme::Dark(Color::from_hex("#eb343a", 255))}
}

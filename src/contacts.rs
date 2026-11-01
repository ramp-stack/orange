use chk::{
    Context, Display, Flow, Form, FormAction, FormComplete, FormItem, FormSubmit,
    FormValidState, ListItem, Page, PageType, Root, State, Theme, Watch
};
use chk::air::profiles::Profile;
use ramp::prism::air::Instance;
use crate::contact_links::parse_contact;

pub struct ContactsHome;
impl ContactsHome {
    pub fn build() -> Root {
        Root::custom(Page::watching(Watch::list::<Profile>(),
            |ctx: &mut Context, _theme: &Theme, profiles: Vec<Instance<Profile>>| {
                let me = ctx.air.me();
                let mut items = profiles.into_iter().filter_map(|mut instance| {
                    if instance.pending().name == Some(me) {return None;}
                    let view = Flow::new(vec![Page::profile(ctx, &mut instance)]);
                    let profile = instance.pending();
                    Some(ListItem::avatar(profile.avatar.clone(), &profile.username,
                        &profile.name(), None, Some(view)))
                }).collect::<Vec<_>>();
                items.sort_by(|a, b| a.title.cmp(&b.title));
                let new_contact = Box::new(|_ctx: &mut Context, theme: &Theme| {
                    Flow::from_form(NewContact::build(theme))
                });
                PageType::root("Contacts", vec![], vec![Display::list(None, items,
                    Some("No contacts yet.\nGet started by adding a friend."))], None,
                    Some(("New Contact".into(), new_contact)), None)
            }))
    }
}

pub struct NewContact;
impl NewContact {
    pub fn build(theme: &Theme) -> Form {
        let submit = Box::new(|ctx: &mut Context, state: &Vec<State>| {
            let State::Text(name) = &state[0] else {unreachable!()};
            let Some(name) = parse_contact(name) else {return FormComplete::None};
            let mut profile = Profile::from_name(ctx, name);
            FormComplete::Next(Page::profile(ctx, &mut profile))
        }) as Box<dyn FormSubmit>;
        Form::flow(theme, vec![FormItem::text("New contact", "orange name", Some(vec![
            FormAction::Paste, FormAction::scan_qr_code("Scan a profile QR code"),
        ]), |ctx: &mut Context, name: String| {
            if name.is_empty() {FormValidState::Invalid}
            else {match parse_contact(&name) {
                Some(name) if name == ctx.air.me() => FormValidState::InvalidWithData(
                    "You cannot add yourself as a contact.".into()),
                Some(_) => FormValidState::Valid,
                None => FormValidState::InvalidWithData("This is not a valid Orange name or contact link.".into()),
            }}
        })], None, None, submit)
    }
}

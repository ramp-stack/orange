use chk::{Action, AvatarContent, AvatarIconStyle, Context, Display, Flow, Form, FormAction,
    FormComplete, FormItem, FormSubmit, FormValidState, Icons, ListItem, Page, PageType, Root,
    State, Theme, Watch};
use chk::air::{messages::{ChatRoom, MessageContent}, profiles::Profile};
use ramp::prism::air::{Instance, Name};
use std::str::FromStr;

pub struct MessagesHome;
impl MessagesHome {
    pub fn new() -> Root {
        Root::custom(Page::watching((Watch::list::<ChatRoom>(), Watch::list::<Profile>()),
            |ctx: &mut Context, _theme: &Theme, (rooms, _): (Vec<Instance<ChatRoom>>, Vec<Instance<Profile>>)| {
                let me = ctx.air.me();
                let mut items = rooms.into_iter().map(|mut instance| {
                    let chat = Flow::new(vec![Page::messaging(ctx, &mut instance)]);
                    let room = instance.pending();
                    let members = room.members.iter().copied().filter(|name| *name != me).collect::<Vec<_>>();
                    let name = room.name(ctx);
                    let avatar = if members.len() > 1 {
                        AvatarContent::icon(Icons::Group, AvatarIconStyle::Secondary)
                    } else {
                        let profile = Profile::from_name(ctx, members.first().copied().unwrap_or(me));
                        profile.pending().avatar.clone()
                    };
                    let (timestamp, last) = match room.messages(me).last() {
                        Some(last) => {
                            let author = if last.author == me {"You".into()} else {
                                Profile::from_name(ctx, last.author).pending().username.clone()
                            };
                            let preview = match &last.body {
                                MessageContent::Text(text) => text.as_str(),
                                MessageContent::Sticker(_) => "Sticker",
                                MessageContent::Gif(_) => "GIF",
                                MessageContent::Photo(_) => "Photo",
                                MessageContent::Video(_) => "Video",
                            };
                            (last.timestamp, format!("{author}: {preview}"))
                        },
                        None => (0, "No messages yet.".into()),
                    };
                    (timestamp, ListItem::avatar(avatar, &name, &last, None, Some(chat)))
                }).collect::<Vec<_>>();
                items.sort_by(|a, b| b.0.cmp(&a.0));
                let items = items.into_iter().map(|(_, item)| item).collect();
                let new_message = Box::new(|ctx: &mut Context, theme: &Theme| {
                    Flow::from_form(NewMessageFlow::new(ctx, theme))
                });
                PageType::root("Messages", vec![], vec![Display::list(None, items,
                    Some("No messages yet.\nGet started by messaging a friend."))], None,
                    Some(("New Message".into(), new_message)), None)
            }))
    }
}

pub struct NewMessageFlow;
impl NewMessageFlow {
    pub fn new(ctx: &mut Context, theme: &Theme) -> Form {
        let submit = Box::new(|ctx: &mut Context, state: &Vec<State>| {
            let State::Search(recipients) = &state[0] else {unreachable!()};
            let mut room = ChatRoom::create(ctx, recipients.clone());
            FormComplete::Next(Page::messaging(ctx, &mut room))
        }) as Box<dyn FormSubmit>;
        let me = ctx.air.me();
        let items = ctx.air.instances::<Profile>().list().into_values().map(|profile| {
            let profile = profile.pending();
            let username = if profile.name == Some(me) {format!("{} (You)", profile.username)}
                else {profile.username.clone()};
            (ListItem::avatar(profile.avatar.clone(), &username, &profile.name(), None, None),
                profile.name.unwrap())
        }).collect();
        Form::flow(theme, vec![FormItem::search("New message", "profile name", items,
            Some(vec![FormAction::Custom("New contact".into(), None, Icons::Add,
                Action::flow(Flow::from_form(CreateAndAddContact::new(theme))))]))], None, None, submit)
    }
}

struct CreateAndAddContact;
impl CreateAndAddContact {
    fn new(theme: &Theme) -> Form {
        let submit = Box::new(|ctx: &mut Context, state: &Vec<State>| {
            let State::Text(name) = &state[0] else {unreachable!()};
            let name = Name::from_str(name).unwrap();
            Profile::create(ctx, name);
            FormComplete::Return(Box::new(move |ctx: &mut Context, theme: &Theme| {
                (Action::choose_search(name).get())(ctx, theme)
            }))
        }) as Box<dyn FormSubmit>;
        Form::flow(theme, vec![FormItem::text("New contact", "orange name", Some(vec![
            FormAction::Paste, FormAction::scan_qr_code("Scan a profile QR code"),
        ]), |_ctx: &mut Context, name: String| {
            if name.is_empty() {FormValidState::Invalid}
            else if Name::from_str(&name).is_ok() {FormValidState::Valid}
            else {FormValidState::InvalidWithData("Not a valid Orange Name.".into())}
        })], None, None, submit)
    }
}

//! Orange-owned contact invitations. Only public identities go into a link.
use std::{any::Any, sync::OnceLock};
use async_channel::{Receiver, Sender};
use chk::{Action, Context, Theme};
use chk::profiles::Profile;
use ramp::prism::{air::Name, drawable::{Drawable, Instruction, LayoutTree, Offset, Rect},
    events::{DynEvent, ListenCallback, ListenFuture}};

pub fn contact_url(name: Name) -> String {format!("https://orange.me/invite.html?name={name}")}

pub fn parse_invite(input: &str) -> Option<Name> {
    let input = input.trim();
    let name = ["https://orange.me/invite.html?name=", "https://www.orange.me/invite.html?name=",
        "orange-contact://contact/"].iter().find_map(|prefix| input.strip_prefix(prefix))?;
    // Accept only the canonical compressed public key, never arbitrary URL content.
    if name.len() != 66 || !name.bytes().all(|c| c.is_ascii_hexdigit()) {return None;}
    let parsed: Name = name.parse().ok()?;
    (parsed.to_string() == name).then_some(parsed)
}

pub fn parse_contact(input: &str) -> Option<Name> {
    parse_invite(input).or_else(|| input.trim().parse().ok())
}

fn inbox() -> &'static (Sender<Name>, Receiver<Name>) {
    static INBOX: OnceLock<(Sender<Name>, Receiver<Name>)> = OnceLock::new();
    INBOX.get_or_init(|| async_channel::bounded(16))
}

#[cfg(any(target_os = "ios", test))]
pub fn receive(url: &str) -> bool {
    let Some(name) = parse_invite(url) else {return false};
    inbox().0.try_send(name).is_ok()
}

#[derive(Clone, Debug)]
pub struct ContactLinks {
    inner: Box<dyn Drawable>,
    theme: Theme,
    receiver: Receiver<Name>,
}

impl ContactLinks {
    pub fn new(inner: impl Drawable, theme: Theme) -> Self {
        Self {inner: Box::new(inner), theme, receiver: inbox().1.clone()}
    }

    fn open(&mut self, ctx: &mut Context, name: Name) {
        Profile::from_name(ctx, name);
        // Uses CHK's existing DM lookup (including rooms created by the other person).
        // Opening an invitation never sends a message automatically.
        (Action::message(name).get())(ctx, &self.theme);
    }
}

impl Drawable for ContactLinks {
    fn request_width(&mut self) -> (f32, f32) {self.inner.request_width()}
    fn request_height(&mut self, width: f32) -> (f32, f32) {self.inner.request_height(width)}
    fn layout(&mut self, size: (f32, f32)) -> LayoutTree {self.inner.layout(size)}
    fn draw(&self, layout: &LayoutTree, offset: Offset, bound: Rect) -> Vec<Instruction> {
        self.inner.draw(layout, offset, bound)
    }
    fn event(&mut self, ctx: &mut Context, layout: &LayoutTree, event: Box<dyn DynEvent>) {
        self.inner.event(ctx, layout, event);
    }
    fn changed(&mut self) -> ListenFuture<'_> {
        Box::pin(async move {
            let receive = Box::pin(self.receiver.recv());
            match futures::future::select(receive, self.inner.as_mut().changed()).await {
                futures::future::Either::Left((Ok(mut name), _)) => {
                    // UIKit may report a cold launch through both launch options and
                    // openURL. Collapse pending deliveries without suppressing future taps.
                    while let Ok(next) = self.receiver.try_recv() {name = next;}
                    Box::new(move |target: &mut dyn Any, ctx: &mut Context| {
                        target.downcast_mut::<Self>().unwrap().open(ctx, name);
                    }) as ListenCallback
                }
                futures::future::Either::Right((callback, _)) => {
                    Box::new(move |target: &mut dyn Any, ctx: &mut Context| {
                        let target = target.downcast_mut::<Self>().unwrap();
                        callback(target.inner.as_mut().as_any_mut(), ctx);
                    }) as ListenCallback
                }
                _ => unreachable!("the process-owned invitation sender stays alive"),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invitations_accept_only_our_hosts_paths_and_valid_public_names() {
        let name = Name::orange_me();
        assert_eq!(parse_invite(&contact_url(name)), Some(name));
        assert_eq!(parse_invite(&format!("https://www.orange.me/invite.html?name={name}")), Some(name));
        assert_eq!(parse_invite(&format!("orange-contact://contact/{name}")), Some(name));
        assert_eq!(parse_contact(&name.to_string()), Some(name));
        for url in [format!("https://orange.me.evil.test/invite.html?name={name}"),
            format!("http://orange.me/invite.html?name={name}"), format!("https://orange.me/invite.html?name={name}&redirect=evil"),
            format!("https://orange.me/invite.html?name={name}/extra"), format!("https://orange.me/invite.html?name={}", "0".repeat(66)),
            "javascript:alert(1)".into(), "https://orange.me/invite.html?name=invalid".into()] {
            assert_eq!(parse_invite(&url), None, "{url}");
        }
    }
    #[test]
    fn cold_start_links_wait_until_the_ui_can_receive_them() {
        assert!(!receive("https://orange.me/invite/invalid"));
        let name = Name::orange_me();
        assert!(receive(&contact_url(name)));
        assert_eq!(inbox().1.try_recv().unwrap(), name);
    }
}

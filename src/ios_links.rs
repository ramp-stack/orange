//! UIKit bridge owned by Orange. Does not replace or patch Ramp's runtime.
use std::{cell::RefCell, ptr::NonNull};
use block2::{DynBlock, RcBlock};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2_foundation::{NSArray, NSDictionary, NSNotification, NSNotificationCenter,
    NSObject, NSObjectProtocol, NSString, NSURL, NSUserActivity};
use objc2_ui_kit::{UIApplication, UIApplicationDelegate, UIApplicationDidFinishLaunchingNotification};

thread_local! {
    static OBSERVER: RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>> = const {RefCell::new(None)};
    static DELEGATE: RefCell<Option<Retained<OrangeLinkDelegate>>> = const {RefCell::new(None)};
}

struct LinkIvars {previous: Option<Retained<AnyObject>>}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "OrangeLinkDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = LinkIvars]
    struct OrangeLinkDelegate;

    unsafe impl NSObjectProtocol for OrangeLinkDelegate {
        #[unsafe(method(respondsToSelector:))]
        fn responds_to_selector(&self, selector: Sel) -> bool {
            let responds: bool = unsafe {msg_send![super(self), respondsToSelector: selector]};
            responds || self.ivars().previous.as_ref().is_some_and(|previous| {
                unsafe {msg_send![previous, respondsToSelector: selector]}
            })
        }
    }
    unsafe impl UIApplicationDelegate for OrangeLinkDelegate {}

    impl OrangeLinkDelegate {
        #[unsafe(method_id(forwardingTargetForSelector:))]
        fn forwarding_target(&self, _selector: Sel) -> Option<Retained<AnyObject>> {
            self.ivars().previous.clone()
        }

        #[unsafe(method(application:openURL:options:))]
        fn open_url(&self, _app: &UIApplication, url: &NSURL, _options: &NSDictionary<NSString, AnyObject>) -> bool {
            accept_url(url)
        }

        #[unsafe(method(application:continueUserActivity:restorationHandler:))]
        fn continue_activity(&self, _app: &UIApplication, activity: &NSUserActivity,
            _restoration: &DynBlock<dyn Fn(*mut NSArray)>) -> bool {
            activity.activityType().to_string() == "NSUserActivityTypeBrowsingWeb"
                && activity.webpageURL().is_some_and(|url| accept_url(&url))
        }
    }
);

fn accept_url(url: &NSURL) -> bool {
    url.absoluteString().is_some_and(|url| crate::contact_links::receive(&url.to_string()))
}

pub fn prepare() {
    // Winit calls UIApplicationMain itself. Observe launch *before* starting it,
    // without creating UIApplication (which would violate Winit's startup contract).
    let block = RcBlock::new(|notification: NonNull<NSNotification>| {
        let mtm = MainThreadMarker::new().expect("UIKit launch must run on the main thread");
        let app = UIApplication::sharedApplication(mtm);
        let previous: Option<Retained<AnyObject>> = unsafe {msg_send![&app, delegate]};
        let delegate = OrangeLinkDelegate::alloc(mtm).set_ivars(LinkIvars {previous});
        let delegate: Retained<OrangeLinkDelegate> = unsafe {msg_send![super(delegate), init]};
        unsafe {app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));}
        DELEGATE.with(|slot| *slot.borrow_mut() = Some(delegate));

        // Custom URLs can also arrive in the initial launch options.
        let notification = unsafe {notification.as_ref()};
        if let Some(info) = notification.userInfo() {
            let key = NSString::from_str("UIApplicationLaunchOptionsURLKey");
            if let Some(value) = info.objectForKey(&key) {
                if let Some(url) = value.downcast_ref::<NSURL>() {accept_url(url);}
            }
        }
    });
    let observer = unsafe {NSNotificationCenter::defaultCenter()
        .addObserverForName_object_queue_usingBlock(
            Some(UIApplicationDidFinishLaunchingNotification), None, None, &block)};
    OBSERVER.with(|slot| *slot.borrow_mut() = Some(observer));
}

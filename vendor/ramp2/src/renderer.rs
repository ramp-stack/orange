use futures::channel::mpsc::UnboundedSender;
use futures::channel::oneshot;
use maverick_os::Window;
use prism::drawable::Instruction;

use crate::native_text::{Action, NativeText};

/// Drawing output and the private callback route back to its owning application.
pub struct Frame {
    instructions: Vec<Instruction>,
    sender: UnboundedSender<Action>,
    complete: oneshot::Sender<()>,
}
impl Frame {
    pub(crate) fn new(instructions: Vec<Instruction>, sender: UnboundedSender<Action>, complete: oneshot::Sender<()>) -> Self {
        Self {instructions, sender, complete}
    }
}

#[derive(Default)]
struct Presentation {
    pending: Option<oneshot::Sender<()>>,
}
impl Presentation {
    fn update(&mut self, complete: oneshot::Sender<()>, changed: bool) {
        if changed || self.pending.is_some() {
            self.pending = Some(complete);
        } else {
            // Identical instructions are already visible; no redraw is needed.
            let _ = complete.send(());
        }
    }
    fn finish(&mut self, presented: bool) {
        if presented && let Some(complete) = self.pending.take() {let _ = complete.send(());}
    }
}

pub struct Renderer {
    window: Window,
    renderer: wgpu_prism::Renderer<'static>,
    native_text: Option<NativeText>,
    presentation: Presentation,
}
impl maverick_os::Renderer for Renderer {
    type Instructions = Frame;

    fn new(window: Window) -> Self {
        let size = window.physical();
        let renderer = futures::executor::block_on(wgpu_prism::Renderer::new(
            window.clone(), size.width, size.height
        ));
        Self {window, renderer, native_text: None, presentation: Presentation::default()}
    }
    fn resize(&mut self) {
        let size = self.window.physical();
        self.renderer.resize(size.width, size.height);
    }
    fn update(&mut self, frame: Frame) -> bool {
        let size = self.window.physical();
        if size.width == 0 || size.height == 0 {return false}
        self.native_text.get_or_insert_with(|| NativeText::new(&self.window, frame.sender))
            .update(&frame.instructions);
        let changed = self.renderer.update(frame.instructions, size.scale_factor as f32);
        self.presentation.update(frame.complete, changed);
        changed
    }
    fn draw(&mut self) -> bool {
        let size = self.window.physical();
        if size.width == 0 || size.height == 0 {return false}
        let presented = self.renderer.render();
        self.presentation.finish(presented);
        presented
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_ui_work_waits_for_the_placeholder_frame() {
        let mut presentation = Presentation::default();
        let (complete, mut ready) = oneshot::channel();
        presentation.update(complete, true);
        assert_eq!(ready.try_recv(), Ok(None));
        presentation.finish(false);
        assert_eq!(ready.try_recv(), Ok(None));
        presentation.finish(true);
        assert_eq!(ready.try_recv(), Ok(Some(())));
    }

    #[test]
    fn unchanged_frames_do_not_wait_for_an_unrequested_redraw() {
        let mut presentation = Presentation::default();
        let (complete, mut ready) = oneshot::channel();
        presentation.update(complete, false);
        assert_eq!(ready.try_recv(), Ok(Some(())));
    }

    #[test]
    fn replacement_snapshot_still_waits_for_the_pending_frame() {
        let mut presentation = Presentation::default();
        let (first, _old) = oneshot::channel();
        presentation.update(first, true);
        let (latest, mut ready) = oneshot::channel();
        presentation.update(latest, false);
        assert_eq!(ready.try_recv(), Ok(None));
        presentation.finish(true);
        assert_eq!(ready.try_recv(), Ok(Some(())));
    }
}

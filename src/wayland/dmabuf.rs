use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        egl::EGLDevice,
        renderer::{ImportDma, gles::GlesRenderer},
    },
    delegate_dmabuf,
    wayland::dmabuf::{
        DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier,
    },
};

use super::State;

pub(super) struct Buffers {
    state: DmabufState,
    pending: Vec<(Dmabuf, ImportNotifier)>,
}

impl Buffers {
    pub fn new() -> Self {
        Self {
            state: DmabufState::new(),
            pending: Vec::new(),
        }
    }
}

impl State {
    pub fn advertise_dmabuf(&mut self, renderer: &GlesRenderer) {
        let formats = renderer.dmabuf_formats();
        if formats.iter().next().is_none() {
            return;
        }
        let render_node = EGLDevice::device_for_display(renderer.egl_context().display())
            .and_then(|device| device.try_get_render_node())
            .ok()
            .flatten();
        let feedback = render_node.and_then(|node| {
            DmabufFeedbackBuilder::new(node.dev_id(), formats.iter().copied())
                .build()
                .ok()
        });
        if let Some(feedback) = feedback {
            self.dmabuf
                .state
                .create_global_with_default_feedback::<Self>(&self.display, &feedback);
        } else {
            self.dmabuf
                .state
                .create_global::<Self>(&self.display, formats.iter().copied());
        }
    }

    pub fn import_pending_dmabufs(&mut self, renderer: &mut GlesRenderer) {
        for (buffer, notifier) in self.dmabuf.pending.drain(..) {
            if renderer.import_dmabuf(&buffer, None).is_ok() {
                let _ = notifier.successful::<Self>();
            } else {
                notifier.failed();
            }
        }
    }
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf.state
    }

    fn dmabuf_imported(&mut self, _: &DmabufGlobal, buffer: Dmabuf, notifier: ImportNotifier) {
        self.dmabuf.pending.push((buffer, notifier));
    }
}

delegate_dmabuf!(State);

//! Present premultiplied ARGB pixels without losing the window's transparent corners.
//! Softbuffer 0.4 uses XRGB on Wayland, so only that backend needs a SHM presenter.

use smithay_client_toolkit::shm::{slot::SlotPool, Shm};
use std::{error::Error, num::NonZeroU32, sync::Arc};
use tiny_skia::{Pixmap, PremultipliedColorU8};
use wayland_client::{
    backend::{Backend, ObjectId},
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_registry, wl_shm, wl_surface},
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use winit::{
    raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle},
    window::Window,
};

type PresentResult<T> = Result<T, Box<dyn Error>>;

pub struct Presenter {
    backend: PresentationBackend,
}

enum PresentationBackend {
    Wayland(WaylandPresenter),
    X11(softbuffer::Surface<Arc<Window>, Arc<Window>>),
}

impl Presenter {
    pub fn new(window: Arc<Window>) -> PresentResult<Self> {
        let backend = if matches!(window.display_handle()?.as_raw(), RawDisplayHandle::Wayland(_)) {
            PresentationBackend::Wayland(WaylandPresenter::new(window)?)
        } else {
            let context = softbuffer::Context::new(window.clone())?;
            PresentationBackend::X11(softbuffer::Surface::new(&context, window)?)
        };
        Ok(Self { backend })
    }

    pub fn present(&mut self, frame: &Pixmap) -> PresentResult<()> {
        match &mut self.backend {
            PresentationBackend::Wayland(surface) => surface.present(frame),
            PresentationBackend::X11(surface) => {
                let width = NonZeroU32::new(frame.width()).ok_or("zero frame width")?;
                let height = NonZeroU32::new(frame.height()).ok_or("zero frame height")?;
                surface.resize(width, height)?;
                let mut buffer = surface.buffer_mut()?;
                for (target, pixel) in buffer.iter_mut().zip(frame.pixels()) {
                    *target = argb_pixel(*pixel);
                }
                buffer.present()?;
                Ok(())
            }
        }
    }
}

struct QueueState;
wayland_client::delegate_noop!(QueueState: ignore wl_shm::WlShm);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for QueueState {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event,
        _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

struct WaylandPresenter {
    surface: wl_surface::WlSurface,
    pool: SlotPool,
    queue: EventQueue<QueueState>,
    connection: Connection,
    // Keep the borrowed native display/surface alive until the fields above drop.
    _window: Arc<Window>,
}

impl WaylandPresenter {
    fn new(window: Arc<Window>) -> PresentResult<Self> {
        let RawDisplayHandle::Wayland(display) = window.display_handle()?.as_raw() else {
            return Err("expected a Wayland display".into());
        };
        let RawWindowHandle::Wayland(handle) = window.window_handle()?.as_raw() else {
            return Err("expected a Wayland window".into());
        };
        // SAFETY: the retained Arc<Window> owns these native objects. This backend
        // borrows the display; it never disconnects it or destroys winit's surface.
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let id = unsafe { ObjectId::from_ptr(wl_surface::WlSurface::interface(), handle.surface.as_ptr().cast()) }?;
        let surface = wl_surface::WlSurface::from_id(&connection, id)?;
        let (globals, queue) = registry_queue_init::<QueueState>(&connection)?;
        let shm = globals.bind::<wl_shm::WlShm, _, _>(&queue.handle(), 1..=1, ())?;
        let pool = SlotPool::new(4, &Shm::from(shm))?;
        Ok(Self { surface, pool, queue, connection, _window: window })
    }

    fn present(&mut self, frame: &Pixmap) -> PresentResult<()> {
        let width = i32::try_from(frame.width())?;
        let height = i32::try_from(frame.height())?;
        let stride = width.checked_mul(4).ok_or("frame stride overflow")?;
        i32::try_from(frame.data().len())?; // wl_shm_pool sizes are signed 32-bit.
        self.queue.dispatch_pending(&mut QueueState)?;
        // ARGB8888 is a required wl_shm format. SlotPool never reuses storage
        // while the compositor is reading it; Buffer frees its slot on release.
        let (buffer, canvas) = self.pool.create_buffer(width, height, stride, wl_shm::Format::Argb8888)?;
        for (target, pixel) in canvas.chunks_exact_mut(4).zip(frame.pixels()) {
            target.copy_from_slice(&argb_pixel(*pixel).to_ne_bytes());
        }
        buffer.attach_to(&self.surface)?;
        if self.surface.version() >= 4 {
            self.surface.damage_buffer(0, 0, width, height);
        } else {
            self.surface.damage(0, 0, i32::MAX, i32::MAX);
        }
        self.surface.commit();
        self.connection.flush()?;
        Ok(())
    }
}

fn argb_pixel(pixel: PremultipliedColorU8) -> u32 {
    (u32::from(pixel.alpha()) << 24) | (u32::from(pixel.red()) << 16) | (u32::from(pixel.green()) << 8) | u32::from(pixel.blue())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_keeps_transparency_and_premultiplied_channels() {
        let clear = PremultipliedColorU8::from_rgba(0, 0, 0, 0).unwrap();
        assert_eq!(argb_pixel(clear), 0);
        let edge = PremultipliedColorU8::from_rgba(10, 15, 20, 100).unwrap();
        assert_eq!(argb_pixel(edge), 0x640a0f14);
    }
}

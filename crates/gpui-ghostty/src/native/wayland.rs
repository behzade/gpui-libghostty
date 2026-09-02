use std::{
    ffi::c_void,
    io,
    os::fd::AsRawFd,
    os::unix::net::UnixStream,
    ptr::NonNull,
    thread::{self, JoinHandle},
};

use khronos_egl as egl;
use libloading::Library;
use wayland_backend::{client::ObjectId, sys::client::Backend};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_compositor::WlCompositor, wl_region::WlRegion, wl_registry,
        wl_subcompositor::WlSubcompositor, wl_subsurface::WlSubsurface, wl_surface::WlSurface,
    },
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};

use super::NativeWakeup;

pub struct WaylandGlSurface {
    // EGL must be destroyed before the child wl_surface.
    egl: EglSurface,
    wayland: WaylandSurface,
    logical_width: i32,
    logical_height: i32,
    scale: f64,
    visible: bool,
}

impl WaylandGlSurface {
    pub fn new(
        display: NonNull<c_void>,
        parent_surface: NonNull<c_void>,
        scale: f64,
        wakeup: NativeWakeup,
    ) -> Result<Box<Self>, String> {
        let wayland = WaylandSurface::new(display, parent_surface, wakeup)?;
        let egl = EglSurface::new(display, wayland.surface_ptr())?;
        let mut result = Box::new(Self {
            egl,
            wayland,
            logical_width: 1,
            logical_height: 1,
            scale: scale.max(1.0),
            visible: false,
        });
        result.resize(0, 0, 1, 1, scale)?;
        Ok(result)
    }

    pub fn make_current(&self) -> Result<(), String> {
        self.egl.make_current()
    }

    pub fn clear_current(&self) {
        self.egl.clear_current();
    }

    pub fn swap_buffers(&self) {
        if self.visible {
            self.egl.swap_buffers();
        }
    }

    pub fn dispatch_pending(&mut self) {
        self.wayland.dispatch_pending();
    }

    pub fn resize(
        &mut self,
        x: i32,
        y: i32,
        logical_width: i32,
        logical_height: i32,
        scale: f64,
    ) -> Result<(u32, u32), String> {
        let logical_width = logical_width.max(1);
        let logical_height = logical_height.max(1);
        let requested_scale = scale.max(1.0);
        let scale = self
            .wayland
            .set_geometry(x, y, logical_width, logical_height, requested_scale);
        let physical_width = ((f64::from(logical_width) * scale).round() as i32).max(1);
        let physical_height = ((f64::from(logical_height) * scale).round() as i32).max(1);

        self.egl.resize(physical_width, physical_height);
        self.make_current()?;
        self.egl.viewport(physical_width, physical_height);
        self.clear_current();
        self.logical_width = logical_width;
        self.logical_height = logical_height;
        self.scale = scale;
        Ok((physical_width as u32, physical_height as u32))
    }

    pub fn set_visible(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.wayland.set_visible(visible);
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }
}

struct WaylandState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(WaylandState: ignore WlCompositor);
delegate_noop!(WaylandState: ignore WlSubcompositor);
delegate_noop!(WaylandState: ignore WlSubsurface);
delegate_noop!(WaylandState: ignore WlSurface);
delegate_noop!(WaylandState: ignore WlRegion);
delegate_noop!(WaylandState: ignore WpViewporter);
delegate_noop!(WaylandState: ignore WpViewport);

struct WaylandEventWatcher {
    shutdown: Option<UnixStream>,
    thread: Option<JoinHandle<()>>,
}

impl WaylandEventWatcher {
    fn new(connection: Connection, wakeup: NativeWakeup) -> io::Result<Self> {
        let (shutdown, shutdown_reader) = UnixStream::pair()?;
        let thread = thread::Builder::new()
            .name("gpui-ghostty-wayland".to_owned())
            .spawn(move || watch_wayland(connection, shutdown_reader, wakeup))?;
        Ok(Self {
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    fn stop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        drop(self.shutdown.take());
        let _ = thread.join();
    }
}

impl Drop for WaylandEventWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

fn watch_wayland(connection: Connection, shutdown: UnixStream, wakeup: NativeWakeup) {
    loop {
        let Some(read_guard) = connection.prepare_read() else {
            match connection.backend().dispatch_inner_queue() {
                Ok(count) if count > 0 => wakeup.signal(),
                Ok(_) => {}
                Err(_) => {
                    wakeup.signal();
                    break;
                }
            }
            continue;
        };
        let mut poll_fds = [
            libc::pollfd {
                fd: read_guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: shutdown.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: Both descriptors stay live for the duration of the call.
        let result =
            unsafe { libc::poll(poll_fds.as_mut_ptr(), poll_fds.len() as libc::nfds_t, -1) };
        if result < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            wakeup.signal();
            break;
        }
        if poll_fds[1].revents != 0 {
            break;
        }
        if poll_fds[0].revents == 0 {
            continue;
        }
        match read_guard.read() {
            Ok(count) if count > 0 => wakeup.signal(),
            Ok(_) => {}
            Err(wayland_backend::client::WaylandError::Io(error))
                if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => {
                wakeup.signal();
                break;
            }
        }
    }
}

enum SurfaceScaling {
    Viewport(WpViewport),
    BufferScale { supported: bool },
}

struct WaylandSurface {
    watcher: WaylandEventWatcher,
    connection: Connection,
    event_queue: EventQueue<WaylandState>,
    state: WaylandState,
    parent: WlSurface,
    surface: WlSurface,
    subsurface: WlSubsurface,
    scaling: SurfaceScaling,
    visible: bool,
}

impl WaylandSurface {
    fn new(
        display: NonNull<c_void>,
        parent_surface: NonNull<c_void>,
        wakeup: NativeWakeup,
    ) -> Result<Self, String> {
        // SAFETY: GPUI owns both pointers and keeps them alive for the Window lifetime.
        // libwayland synchronizes reads across queues; the guest backend never closes the display.
        let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, event_queue) = registry_queue_init::<WaylandState>(&connection)
            .map_err(|error| format!("read Wayland globals: {error}"))?;
        let qh = event_queue.handle();
        let compositor: WlCompositor = globals
            .bind(&qh, 1..=6, ())
            .map_err(|error| format!("bind wl_compositor: {error}"))?;
        let subcompositor: WlSubcompositor = globals
            .bind(&qh, 1..=1, ())
            .map_err(|error| format!("bind wl_subcompositor: {error}"))?;
        let viewporter: Option<WpViewporter> = globals.bind(&qh, 1..=1, ()).ok();

        // SAFETY: The raw handle is GPUI's live wl_surface on this same display.
        let parent_id =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), parent_surface.as_ptr().cast()) }
                .map_err(|error| format!("adopt parent wl_surface: {error}"))?;
        let parent = WlSurface::from_id(&connection, parent_id)
            .map_err(|error| format!("wrap parent wl_surface: {error}"))?;
        let surface = compositor.create_surface(&qh, ());
        let region = compositor.create_region(&qh, ());
        surface.set_input_region(Some(&region));
        region.destroy();
        let subsurface = subcompositor.get_subsurface(&surface, &parent, &qh, ());
        subsurface.set_desync();
        let scaling = if let Some(viewporter) = viewporter {
            let viewport = viewporter.get_viewport(&surface, &qh, ());
            viewport.set_destination(1, 1);
            SurfaceScaling::Viewport(viewport)
        } else {
            let supported = surface.version() >= 3;
            if supported {
                surface.set_buffer_scale(1);
            }
            SurfaceScaling::BufferScale { supported }
        };
        surface.commit();
        connection
            .flush()
            .map_err(|error| format!("flush Wayland child surface: {error}"))?;
        let watcher = WaylandEventWatcher::new(connection.clone(), wakeup)
            .map_err(|error| format!("watch Wayland events: {error}"))?;

        Ok(Self {
            watcher,
            connection,
            event_queue,
            state: WaylandState,
            parent,
            surface,
            subsurface,
            scaling,
            visible: false,
        })
    }

    fn surface_ptr(&self) -> NonNull<c_void> {
        NonNull::new(self.surface.id().as_ptr().cast()).expect("live child wl_surface")
    }

    fn dispatch_pending(&mut self) {
        let _ = self.event_queue.dispatch_pending(&mut self.state);
    }

    fn set_geometry(&self, x: i32, y: i32, width: i32, height: i32, requested_scale: f64) -> f64 {
        self.subsurface.set_position(x, y);
        let effective_scale = match &self.scaling {
            SurfaceScaling::Viewport(viewport) => {
                viewport.set_destination(width, height);
                requested_scale
            }
            SurfaceScaling::BufferScale { supported: true } => {
                let integer_scale = requested_scale.ceil().min(f64::from(i32::MAX)) as i32;
                self.surface.set_buffer_scale(integer_scale);
                f64::from(integer_scale)
            }
            SurfaceScaling::BufferScale { supported: false } => 1.0,
        };
        self.surface.commit();
        let _ = self.connection.flush();
        effective_scale
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if !visible {
            self.surface.attach(None, 0, 0);
            self.surface.commit();
            let _ = self.connection.flush();
        }
    }
}

impl Drop for WaylandSurface {
    fn drop(&mut self) {
        self.watcher.stop();
        if let SurfaceScaling::Viewport(viewport) = &self.scaling {
            viewport.destroy();
        }
        self.subsurface.destroy();
        self.surface.destroy();
        let _ = self.connection.flush();
        let _ = &self.parent;
    }
}

type WlEglWindowCreate = unsafe extern "C" fn(*mut c_void, i32, i32) -> *mut c_void;
type WlEglWindowDestroy = unsafe extern "C" fn(*mut c_void);
type WlEglWindowResize = unsafe extern "C" fn(*mut c_void, i32, i32, i32, i32);
type GlViewport = unsafe extern "C" fn(i32, i32, i32, i32);

struct WaylandEgl {
    _library: Library,
    create: WlEglWindowCreate,
    destroy: WlEglWindowDestroy,
    resize: WlEglWindowResize,
}

impl WaylandEgl {
    fn load() -> Result<Self, String> {
        // SAFETY: Symbols are copied into function pointers while the library is retained.
        unsafe {
            let library = Library::new("libwayland-egl.so.1")
                .map_err(|error| format!("load libwayland-egl.so.1: {error}"))?;
            let create = *library
                .get::<WlEglWindowCreate>(b"wl_egl_window_create\0")
                .map_err(|error| format!("load wl_egl_window_create: {error}"))?;
            let destroy = *library
                .get::<WlEglWindowDestroy>(b"wl_egl_window_destroy\0")
                .map_err(|error| format!("load wl_egl_window_destroy: {error}"))?;
            let resize = *library
                .get::<WlEglWindowResize>(b"wl_egl_window_resize\0")
                .map_err(|error| format!("load wl_egl_window_resize: {error}"))?;
            Ok(Self {
                _library: library,
                create,
                destroy,
                resize,
            })
        }
    }
}

struct EglSurface {
    api: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
    surface: egl::Surface,
    window: NonNull<c_void>,
    wayland_egl: WaylandEgl,
    viewport: GlViewport,
}

impl EglSurface {
    fn new(display_ptr: NonNull<c_void>, surface_ptr: NonNull<c_void>) -> Result<Self, String> {
        // SAFETY: Dynamic EGL symbols remain owned by the returned instance.
        let api = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }
            .map_err(|error| format!("load libEGL.so.1: {error}"))?;
        api.bind_api(egl::OPENGL_API)
            .map_err(|error| format!("select desktop OpenGL: {error:?}"))?;
        // SAFETY: The wl_display is live for the GPUI Window lifetime.
        let display = unsafe { api.get_display(display_ptr.as_ptr()) }
            .ok_or_else(|| "EGL rejected GPUI's wl_display".to_owned())?;
        api.initialize(display)
            .map_err(|error| format!("initialize EGL: {error:?}"))?;
        let attributes = [
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_BIT,
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::NONE,
        ];
        let config = api
            .choose_first_config(display, &attributes)
            .map_err(|error| format!("choose EGL config: {error:?}"))?
            .ok_or_else(|| "no EGL OpenGL window config".to_owned())?;
        let context_attributes = [
            egl::CONTEXT_MAJOR_VERSION,
            4,
            egl::CONTEXT_MINOR_VERSION,
            3,
            egl::CONTEXT_OPENGL_PROFILE_MASK,
            egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            egl::NONE,
        ];
        let context = api
            .create_context(display, config, None, &context_attributes)
            .map_err(|error| format!("create OpenGL 4.3 context: {error:?}"))?;
        let wayland_egl = WaylandEgl::load()?;
        // SAFETY: The child wl_surface remains alive until after this EGL window.
        let window = unsafe { (wayland_egl.create)(surface_ptr.as_ptr(), 1, 1) };
        let window = NonNull::new(window).ok_or_else(|| "create wl_egl_window".to_owned())?;
        // SAFETY: wl_egl_window is the native window type required by Wayland EGL.
        let surface = unsafe { api.create_window_surface(display, config, window.as_ptr(), None) }
            .map_err(|error| format!("create EGL window surface: {error:?}"))?;
        api.make_current(display, Some(surface), Some(surface), Some(context))
            .map_err(|error| format!("make OpenGL context current: {error:?}"))?;
        let viewport = api
            .get_proc_address("glViewport")
            .and_then(|proc| {
                let raw = proc as *const ();
                (!raw.is_null()).then(|| {
                    // SAFETY: EGL returned glViewport with the standard OpenGL ABI.
                    unsafe { std::mem::transmute::<*const (), GlViewport>(raw) }
                })
            })
            .ok_or_else(|| "load glViewport".to_owned())?;
        unsafe { viewport(0, 0, 1, 1) };
        // Ghostty renders this child surface from GPUI's event-loop thread. A
        // blocking, vsynced swap here can stall Wayland key-repeat timers; the
        // top-level GPUI window already provides frame pacing.
        let _ = api.swap_interval(display, 0);

        Ok(Self {
            api,
            display,
            context,
            surface,
            window,
            wayland_egl,
            viewport,
        })
    }

    fn make_current(&self) -> Result<(), String> {
        self.api
            .make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
            .map_err(|error| format!("make OpenGL context current: {error:?}"))
    }

    fn clear_current(&self) {
        let _ = self.api.make_current(self.display, None, None, None);
    }

    fn viewport(&self, width: i32, height: i32) {
        // SAFETY: The context is current and the dimensions are positive.
        unsafe { (self.viewport)(0, 0, width, height) };
    }

    fn resize(&self, width: i32, height: i32) {
        // SAFETY: The wl_egl_window is live and owned by this value.
        unsafe { (self.wayland_egl.resize)(self.window.as_ptr(), width, height, 0, 0) };
    }

    fn swap_buffers(&self) {
        let _ = self.api.swap_buffers(self.display, self.surface);
    }
}

impl Drop for EglSurface {
    fn drop(&mut self) {
        self.clear_current();
        let _ = self.api.destroy_surface(self.display, self.surface);
        let _ = self.api.destroy_context(self.display, self.context);
        // Do not terminate the shared Wayland EGLDisplay while another terminal may use it.
        // SAFETY: The wl_egl_window is uniquely owned and still live.
        unsafe { (self.wayland_egl.destroy)(self.window.as_ptr()) };
    }
}

//! sharpless: draw black rounded corners at the edges of each screen output.
//!
//! For every monitor, a transparent, click-through layer-shell surface is
//! created in the overlay layer anchored to all four edges so it covers the
//! whole screen. Each frame, a tiny-skia pixmap backed by the shared SHM slot
//! is filled with opaque black squircle corner shapes (anti-aliased by the
//! rasterizer, kept fully transparent everywhere else). The compositor
//! composites the result, so each corner of the screen shows a black rounded
//! edge.

mod rounding;

use std::num::NonZeroU32;

use clap::Parser;
use rounding::corner_contours;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, PixmapMut, Transform};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_region, wl_shm, wl_surface},
    Connection, Dispatch, QueueHandle,
};

/// Command-line options for the corner overlay.
#[derive(Parser, Debug, Clone)]
#[command(version, about)]
struct Cli {
    /// Corner radius in logical pixels.
    #[arg(long, default_value_t = crate::rounding::CORNER_RADIUS)]
    radius: u32,
    /// Squircle strength: 2.0 is a circular arc, larger values flatten the
    /// edges and tighten the turn.
    #[arg(long, default_value_t = crate::rounding::CORNER_CURVATURE)]
    curvature: f64,
}

/// A per-output overlay: the layer surface plus its last known geometry.
struct OutputOverlay {
    output: wl_output::WlOutput,
    layer: LayerSurface,
    width: u32,
    height: u32,
    scale: i32,
    transform: wl_output::Transform,
    first_configure: bool,
    needs_redraw: bool,
}

struct App {
    cli: Cli,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,

    registry_state: RegistryState,
    output_state: OutputState,

    overlays: Vec<OutputOverlay>,
    exit: bool,
}

fn main() {
    let cli = Cli::parse();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let conn = Connection::connect_to_env().expect("failed to connect to the Wayland compositor");
    let (globals, mut event_queue) =
        registry_queue_init(&conn).expect("failed to enumerate globals");
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor is not available");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("wlr-layer-shell is not available");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm is not available");

    // Reasonable starting allocation; the pool grows on demand.
    let pool = SlotPool::new(256 * 256 * 4, &shm).expect("failed to create shm pool");

    let mut app = App {
        cli,
        compositor,
        layer_shell,
        shm,
        pool,
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        overlays: Vec::new(),
        exit: false,
    };

    // Outputs that already exist get their `new_output` callback fired on their
    // first `Done` event, so no manual pass is needed here.
    while !app.exit {
        event_queue
            .blocking_dispatch(&mut app)
            .expect("failed to dispatch Wayland events");
    }
}

/// Build the corner mask as a single tiny-skia path.
///
/// `rounding.rs` computes the squircle outline (pure math, no libraries);
/// here it is only converted into a `tiny_skia::Path` so the rasterizer can
/// fill and anti-alias it.
fn corner_path(width: u32, height: u32, radius: u32, curvature: f64) -> Option<tiny_skia::Path> {
    let contours = corner_contours(width, height, radius, curvature);
    let mut builder = PathBuilder::new();
    for contour in contours {
        let Some((x, y)) = contour.first().copied() else {
            continue;
        };
        builder.move_to(x, y);
        for &(x, y) in contour.iter().skip(1) {
            builder.line_to(x, y);
        }
        builder.close();
    }
    builder.finish()
}

/// Paint the whole `canvas` (ARGB8888, little-endian `[B, G, R, A]`) with the
/// black squircle corner mask using tiny-skia.
fn paint_corners(canvas: &mut [u8], width: u32, height: u32, radius: u32, curvature: f64) {
    let Some(mut pixmap) = PixmapMut::from_bytes(canvas, width, height) else {
        return;
    };
    // tiny-skia wants RGBA bytes; our buffer is ARGB8888 (i.e. B,G,R,A on
    // little-endian), so swap R and B while painting through the pixmap.
    let data = pixmap.data_mut();
    for chunk in data.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }
    let Some(path) = corner_path(width, height, radius, curvature) else {
        return;
    };
    let paint = Paint {
        shader: tiny_skia::Shader::SolidColor(Color::from_rgba8(0, 0, 0, 0xFF)),
        ..Default::default()
    };
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    let data = pixmap.data_mut();
    for chunk in data.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }
}

impl App {
    fn create_output_surface(
        &mut self,
        output: &wl_output::WlOutput,
        qh: &QueueHandle<Self>,
        transform_hint: Option<wl_output::Transform>,
    ) {
        if self.overlays.iter().any(|o| o.output == *output) {
            return;
        }

        let scale = self
            .output_state
            .info(output)
            .map_or(1, |i| i.scale_factor.max(1));

        let surface = self.compositor.create_surface(qh);
        // An empty input region makes the overlay fully click-through; note
        // that set_input_region(None) means the opposite (whole surface hits).
        let input_region = self.compositor.wl_compositor().create_region(qh, ());
        surface.set_input_region(Some(&input_region));
        // Match the buffer transform to the output's orientation so the
        // compositor maps our buffer 1:1 instead of rotating it again on top
        // of the screen rotation; draw() refreshes this on every redraw.
        let transform = self
            .output_state
            .info(output)
            .map_or(wl_output::Transform::Normal, |i| i.transform);
        surface.set_buffer_transform(transform);
        surface.set_buffer_scale(1);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("sharpless"),
            Some(output),
        );
        // Anchored to all four edges => the surface is sized to the whole output.
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);
        // Let the compositor pick the size from the anchors.
        layer.set_size(0, 0);
        layer.commit();

        // The last known output transform; used to detect real changes so a
        // transform event for an already-matched surface does not loop.
        let transform = transform_hint
            .or_else(|| self.output_state.info(output).map(|i| i.transform))
            .unwrap_or(wl_output::Transform::Normal);

        self.overlays.push(OutputOverlay {
            output: output.clone(),
            layer,
            width: 0,
            height: 0,
            scale,
            transform,
            first_configure: true,
            needs_redraw: false,
        });
    }

    /// Tear the overlay down and build a brand-new surface for this output.
    ///
    /// Some compositors keep a stale buffer/transform mapping for an existing
    /// layer surface across output rotations (even 180°, which changes no
    /// sizes); repainting into the old surface cannot fix that. Destroying
    /// the wl_surface takes the layer role with it, and the replacement
    /// surface starts from a clean configure.
    fn recreate_output_surface(
        &mut self,
        output: &wl_output::WlOutput,
        qh: &QueueHandle<Self>,
        transform_hint: Option<wl_output::Transform>,
    ) {
        if let Some(index) = self.overlays.iter().position(|o| o.output == *output) {
            let surface = self.overlays[index].layer.wl_surface().clone();
            self.overlays.remove(index);
            surface.destroy();
        }
        self.create_output_surface(output, qh, transform_hint);
    }

    fn draw(&mut self, index: usize, qh: &QueueHandle<Self>) {
        let Some(overlay) = self.overlays.get_mut(index) else {
            return;
        };
        let width = overlay.width;
        let height = overlay.height;
        if width == 0 || height == 0 {
            return;
        }
        let scale = overlay.scale.max(1) as u32;
        // The mask is always drawn upright in buffer space; the compositor
        // applies the output transform itself. A transform CHANGE is handled
        // by destroying and recreating the surface, not by repainting.
        let buf_w = width * scale;
        let buf_h = height * scale;
        let radius = self.cli.radius.max(1) * scale;
        println!("draw: logical {width}x{height} scale {scale} => buffer {buf_w}x{buf_h}");

        let surface = overlay.layer.wl_surface();
        surface.set_buffer_transform(wl_output::Transform::Normal);
        surface.set_buffer_scale(1);

        let (buffer, canvas) = match self.pool.create_buffer(
            buf_w as i32,
            buf_h as i32,
            (buf_w * 4) as i32,
            wl_shm::Format::Argb8888,
        ) {
            Ok(b) => b,
            Err(err) => {
                log::error!("failed to create buffer: {err:?}");
                return;
            }
        };

        paint_corners(canvas, buf_w, buf_h, radius, self.cli.curvature);

        surface.damage_buffer(0, 0, buf_w as i32, buf_h as i32);
        surface.frame(qh, FrameCallbackData(surface.clone()));
        buffer
            .attach_to(surface)
            .expect("failed to attach buffer to surface");
        overlay.layer.commit();
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _new_scale: i32,
    ) {
        if let Some(index) = self
            .overlays
            .iter()
            .position(|o| o.layer.wl_surface() == surface)
        {
            self.overlays[index].needs_redraw = true;
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_transform: wl_output::Transform,
    ) {
        let Some(index) = self
            .overlays
            .iter()
            .position(|o| o.layer.wl_surface() == surface)
        else {
            return;
        };
        if self.overlays[index].transform == new_transform {
            return;
        }
        let output = self.overlays[index].output.clone();
        self.recreate_output_surface(&output, qh, Some(new_transform));
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        // Debug: dump per-frame size snapshot so rotation issues are visible
        // in stdout.
        if let Some(o) = self
            .overlays
            .iter()
            .find(|o| o.layer.wl_surface() == surface)
        {
            println!(
                "frame: logical {}x{} scale {} => buffer {}x{}",
                o.width,
                o.height,
                o.scale,
                o.width * o.scale.max(1) as u32,
                o.height * o.scale.max(1) as u32,
            );
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.create_output_surface(&output, qh, None);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(index) = self.overlays.iter().position(|o| o.output == output) {
            if let Some(info) = self.output_state.info(&output) {
                self.overlays[index].scale = info.scale_factor.max(1);
                if self.overlays[index].transform != info.transform {
                    self.recreate_output_surface(&output, qh, Some(info.transform));
                    return;
                }
            }
            self.draw(index, qh);
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.overlays.retain(|o| o.output != output);
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        self.overlays.retain(|o| o.layer != *layer);
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(index) = self.overlays.iter().position(|o| o.layer == *layer) else {
            return;
        };
        let (new_w, new_h) = configure.new_size;
        let changed = new_w > 0
            && new_h > 0
            && (new_w != self.overlays[index].width || new_h != self.overlays[index].height);
        self.overlays[index].width =
            NonZeroU32::new(new_w).map_or(self.overlays[index].width, NonZeroU32::get);
        self.overlays[index].height =
            NonZeroU32::new(new_h).map_or(self.overlays[index].height, NonZeroU32::get);
        // Redraw on any observed change: first configure, rotation/transform
        // flags, or a size snapshot that differs from the last one drawn.
        if self.overlays[index].first_configure || self.overlays[index].needs_redraw || changed {
            self.overlays[index].first_configure = false;
            self.overlays[index].needs_redraw = false;
            self.draw(index, qh);
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(App);

// wl_region is an inert object with no events; it only needs to exist so the
// dispatcher does not reject it.
impl Dispatch<wl_region::WlRegion, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_region::WlRegion,
        _: <wl_region::WlRegion as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

smithay_client_toolkit::delegate_dispatch2!(App);

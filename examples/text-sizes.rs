use metalglyph::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight,
};
use objc2::{
    rc::{autoreleasepool, Retained},
    runtime::ProtocolObject,
};
use objc2_app_kit::NSView;
use objc2_core_foundation::CGSize;
use objc2_metal::{
    MTL4CommandAllocator, MTL4CommandBuffer, MTL4CommandEncoder as _, MTL4CommandQueue,
    MTL4RenderPassDescriptor, MTLClearColor, MTLCreateSystemDefaultDevice, MTLDevice,
    MTLDrawable as _, MTLLoadAction, MTLPixelFormat, MTLStoreAction,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{ptr::NonNull, sync::Arc};
use winit::{
    dpi::{LogicalSize, PhysicalSize},
    event::WindowEvent,
    event_loop::EventLoop,
    window::Window,
};

const TEXT: &str = "The quick brown fox jumped over the lazy doggo. 🐕";
const WEIGHT: Weight = Weight::NORMAL;
const SIZES: [f32; 16] = [
    8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 18.0, 20.0, 22.0, 24.0, 28.0, 32.0, 48.0,
];
const LINE_HEIGHT: f32 = 1.15;
const BG_COLOR: Color = Color::rgb(255, 255, 255);
const FONT_COLOR: Color = Color::rgb(0, 0, 0);
//const BG_COLOR: wgpu::Color = wgpu::Color::BLACK;
//const FONT_COLOR: Color = Color::rgb(255, 255, 255);

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop
        .run_app(&mut Application { window_state: None })
        .unwrap();
}

struct WindowState {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTL4CommandQueue>>,
    alloc: Retained<ProtocolObject<dyn MTL4CommandAllocator>>,
    buffer: Retained<ProtocolObject<dyn MTL4CommandBuffer>>,

    surface: Retained<CAMetalLayer>,
    physical_size: PhysicalSize<i32>,
    scale_factor: f32,

    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    buffers: Vec<Buffer>,

    // Make sure that the winit window is last in the struct so that
    // it is dropped after the wgpu surface is dropped, otherwise the
    // program may crash when closed. This is probably a bug in wgpu.
    window: Arc<Window>,
}

impl WindowState {
    fn new(window: Arc<Window>) -> Self {
        let physical_size = window.inner_size();
        let scale_factor = window.scale_factor();

        let view = match window.window_handle().expect("Window handle").as_raw() {
            RawWindowHandle::AppKit(appkit_handle) => unsafe {
                Retained::retain(appkit_handle.ns_view.as_ptr() as *mut NSView).unwrap()
            },
            _ => panic!("Unsupported platform"),
        };

        let device = MTLCreateSystemDefaultDevice().expect("Create MTL device");

        let queue = device
            .newMTL4CommandQueue()
            .expect("Create MTL command queue");

        let alloc = device
            .newCommandAllocator()
            .expect("Create MTL command allocator");

        let buffer = device
            .newCommandBuffer()
            .expect("Create MTL command buffer");

        let surface = CAMetalLayer::new();
        surface.setDevice(Some(&device));
        surface.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        surface.setPresentsWithTransaction(false);

        surface.setDrawableSize(CGSize {
            width: physical_size.width as f64,
            height: physical_size.height as f64,
        });

        // Set up text renderer
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device);
        let mut atlas = TextAtlas::new(&device, &cache, MTLPixelFormat::BGRA8Unorm);
        let text_renderer = TextRenderer::new(&mut atlas, &device);

        view.setWantsLayer(true);
        view.setLayer(Some(&surface));

        queue.addResidencySet(&surface.residencySet());
        queue.addResidencySet(text_renderer.residency_set());

        let attrs = Attrs::new().family(Family::SansSerif).weight(WEIGHT);
        let shaping = Shaping::Advanced;
        let logical_width = physical_size.width as f32 / scale_factor as f32;

        let buffers: Vec<Buffer> = SIZES
            .iter()
            .copied()
            .map(|s| {
                let mut text_buffer =
                    Buffer::new(&mut font_system, Metrics::relative(s, LINE_HEIGHT));

                text_buffer.set_size(&mut font_system, Some(logical_width - 20.0), None);

                text_buffer.set_text(
                    &mut font_system,
                    &format!("size {s}: {TEXT}"),
                    &attrs,
                    shaping,
                );

                text_buffer.shape_until_scroll(&mut font_system, false);

                text_buffer
            })
            .collect();

        Self {
            device,
            queue,
            alloc,
            buffer,

            surface,
            physical_size: physical_size.cast(),
            scale_factor: scale_factor as f32,

            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            buffers,

            window,
        }
    }
}

struct Application {
    window_state: Option<WindowState>,
}

impl winit::application::ApplicationHandler for Application {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.window_state.is_some() {
            return;
        }

        // Set up window
        let (width, height) = (800, 600);
        let window_attributes = Window::default_attributes()
            .with_inner_size(LogicalSize::new(width as f64, height as f64))
            .with_title("glyphon text sizes test");
        let window = Arc::new(event_loop.create_window(window_attributes).unwrap());

        self.window_state = Some(WindowState::new(window));
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &mut self.window_state else {
            return;
        };

        let WindowState {
            window,
            device,
            queue,
            alloc,
            buffer,
            surface,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            buffers,
            scale_factor,
            physical_size,
            ..
        } = state;

        match event {
            WindowEvent::Resized(size) => {
                surface.setDrawableSize(CGSize {
                    width: size.width as f64,
                    height: size.height as f64,
                });

                *scale_factor = window.scale_factor() as f32;
                *physical_size = size.cast();

                let logical_width = size.width as f32 / *scale_factor;

                for b in buffers.iter_mut() {
                    b.set_size(font_system, Some(logical_width - 20.0), None);
                    b.shape_until_scroll(font_system, false);
                }

                window.request_redraw();
            }

            WindowEvent::RedrawRequested => {
                autoreleasepool(|_| {
                    let drawable = match surface.nextDrawable() {
                        Some(drawable) => drawable,
                        None => panic!("Failed to get next drawable"),
                    };

                    alloc.reset();
                    buffer.beginCommandBufferWithAllocator(&alloc);

                    let resolution = Resolution {
                        width: surface.drawableSize().width as u32,
                        height: surface.drawableSize().height as u32,
                    };

                    viewport.update(resolution);

                    let scale_factor = *scale_factor;

                    let left = 10.0 * scale_factor;
                    let mut top = 10.0 * scale_factor;

                    let bounds_left = left.floor() as i32;
                    let bounds_right = physical_size.width - 10;

                    let text_areas: Vec<TextArea> = buffers
                        .iter()
                        .map(|b| {
                            let a = TextArea {
                                buffer: b,
                                left,
                                top,
                                scale: scale_factor,
                                bounds: TextBounds {
                                    left: bounds_left,
                                    top: top.floor() as i32,
                                    right: bounds_right,
                                    bottom: top.floor() as i32 + physical_size.height,
                                },
                                default_color: FONT_COLOR,
                                custom_glyphs: &[],
                            };

                            let total_lines = b
                                .layout_runs()
                                .fold(0usize, |total_lines, _| total_lines + 1);

                            top +=
                                (total_lines as f32 * b.metrics().line_height + 5.0) * scale_factor;

                            a
                        })
                        .collect();

                    text_renderer
                        .prepare(
                            device,
                            font_system,
                            atlas,
                            viewport,
                            text_areas,
                            swash_cache,
                        )
                        .unwrap();

                    let render_pass_descriptor = MTL4RenderPassDescriptor::new();
                    let color_attachment = unsafe {
                        render_pass_descriptor
                            .colorAttachments()
                            .objectAtIndexedSubscript(0)
                    };

                    color_attachment.setTexture(Some(&drawable.texture()));
                    color_attachment.setLoadAction(MTLLoadAction::Clear);
                    color_attachment.setClearColor(MTLClearColor {
                        red: BG_COLOR.r() as f64 / 255.0,
                        green: BG_COLOR.g() as f64 / 255.0,
                        blue: BG_COLOR.b() as f64 / 255.0,
                        alpha: BG_COLOR.a() as f64 / 255.0,
                    });
                    color_attachment.setStoreAction(MTLStoreAction::Store);

                    let Some(render_encoder) =
                        buffer.renderCommandEncoderWithDescriptor(&render_pass_descriptor)
                    else {
                        return;
                    };

                    text_renderer.render(atlas, viewport, &render_encoder);

                    render_encoder.endEncoding();

                    buffer.endCommandBuffer();
                    queue.waitForDrawable(drawable.as_ref());
                    queue.signalDrawable(drawable.as_ref());

                    unsafe {
                        queue.commit_count(
                            NonNull::from(
                                &NonNull::new(Retained::as_ptr(&buffer) as *mut _).unwrap(),
                            ),
                            1,
                        );
                    }

                    drawable.present();
                    atlas.trim();
                });
            }

            WindowEvent::CloseRequested => event_loop.exit(),

            _ => {}
        }
    }
}

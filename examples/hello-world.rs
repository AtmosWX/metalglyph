use metalglyph::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use objc2::{
    rc::{autoreleasepool, Retained},
    runtime::ProtocolObject,
};
use objc2_app_kit::NSView;
use objc2_core_foundation::CGSize;
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer as _, MTLCommandEncoder as _, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLLoadAction, MTLPixelFormat,
    MTLRenderPassDescriptor, MTLStoreAction,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::Arc;
use winit::{dpi::LogicalSize, event::WindowEvent, event_loop::EventLoop, window::Window};

fn main() {
    let event_loop = EventLoop::new().unwrap();
    event_loop
        .run_app(&mut Application { window_state: None })
        .unwrap();
}

struct WindowState {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,

    surface: Retained<CAMetalLayer>,

    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    text_buffer: Buffer,

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

        let queue = device.newCommandQueue().expect("Create command queue");

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
        let text_renderer = TextRenderer::new(&mut atlas, &device, 1);
        let mut text_buffer = Buffer::new(&mut font_system, Metrics::new(30.0, 42.0));

        view.setWantsLayer(true);
        view.setLayer(Some(&surface));

        let physical_width = (physical_size.width as f64 * scale_factor) as f32;
        let physical_height = (physical_size.height as f64 * scale_factor) as f32;

        text_buffer.set_size(
            &mut font_system,
            Some(physical_width),
            Some(physical_height),
        );
        text_buffer.set_text(&mut font_system, "Hello world! 👋\nThis is rendered with 🦅 metalglyph 🦁\nThe text below should be partially clipped.\na b c d e f g h i j k l m n o p q r s t u v w x y z", &Attrs::new().family(Family::SansSerif), Shaping::Advanced);
        text_buffer.shape_until_scroll(&mut font_system, false);

        Self {
            device,
            queue,

            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text_buffer,

            surface,
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
            .with_title("metalglyph hello world");
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
            surface,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text_buffer,
            ..
        } = state;

        match event {
            WindowEvent::Resized(size) => {
                surface.setDrawableSize(CGSize {
                    width: size.width as f64,
                    height: size.height as f64,
                });
                window.request_redraw();
            }

            WindowEvent::RedrawRequested => {
                autoreleasepool(|_| {
                    let drawable = match surface.nextDrawable() {
                        Some(drawable) => drawable,
                        None => panic!("Failed to get next drawable"),
                    };

                    let resolution = Resolution {
                        width: surface.drawableSize().width as u32,
                        height: surface.drawableSize().height as u32,
                    };

                    viewport.update(resolution);

                    text_renderer
                        .prepare(
                            device,
                            font_system,
                            atlas,
                            viewport,
                            [TextArea {
                                buffer: text_buffer,
                                left: 10.0,
                                top: 10.0,
                                scale: 1.0,
                                bounds: TextBounds {
                                    left: 0,
                                    top: 0,
                                    right: 600,
                                    bottom: 160,
                                },
                                default_color: Color::rgb(255, 255, 255),
                                custom_glyphs: &[],
                            }],
                            swash_cache,
                        )
                        .unwrap();

                    let render_pass_descriptor = MTLRenderPassDescriptor::new();
                    let color_attachment = unsafe {
                        render_pass_descriptor
                            .colorAttachments()
                            .objectAtIndexedSubscript(0)
                    };

                    color_attachment.setTexture(Some(&drawable.texture()));
                    color_attachment.setLoadAction(MTLLoadAction::Clear);
                    color_attachment.setClearColor(MTLClearColor {
                        red: 0.0,
                        green: 0.0,
                        blue: 0.0,
                        alpha: 1.0,
                    });
                    color_attachment.setStoreAction(MTLStoreAction::Store);

                    let Some(buffer) = queue.commandBuffer() else {
                        return;
                    };

                    let Some(render_encoder) =
                        buffer.renderCommandEncoderWithDescriptor(&render_pass_descriptor)
                    else {
                        return;
                    };

                    text_renderer.render(atlas, viewport, &render_encoder);

                    render_encoder.endEncoding();

                    buffer.presentDrawable(drawable.as_ref());
                    buffer.commit();
                    atlas.trim();
                });
            }

            WindowEvent::CloseRequested => event_loop.exit(),

            _ => {}
        }
    }
}

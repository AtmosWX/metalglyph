use metalglyph::{
    Attrs, Buffer, Cache, Color, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport,
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
use objc2_quartz_core::{CAMetalDrawable as _, CAMetalLayer};
use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
use std::sync::Arc;
use winit::{dpi::LogicalSize, event::WindowEvent, event_loop::EventLoop, window::Window};

// Example SVG icons are from https://publicdomainvectors.org/
static LION_SVG: &[u8] = include_bytes!("./lion.svg");
static EAGLE_SVG: &[u8] = include_bytes!("./eagle.svg");

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
    rasterize_svg: Box<dyn Fn(RasterizeCustomGlyphRequest) -> Option<RasterizedCustomGlyph>>,

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
        let queue = device.newCommandQueue().expect("Create MTL command queue");

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
        text_buffer.set_text(
            &mut font_system,
            "SVG icons!     --->\n\nThe icons below should be partially clipped.",
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
        );
        text_buffer.shape_until_scroll(&mut font_system, false);

        // Set up custom svg renderer
        let svg_0 = resvg::usvg::Tree::from_data(LION_SVG, &Default::default()).unwrap();
        let svg_1 = resvg::usvg::Tree::from_data(EAGLE_SVG, &Default::default()).unwrap();

        let rasterize_svg =
            move |input: RasterizeCustomGlyphRequest| -> Option<RasterizedCustomGlyph> {
                // Select the svg data based on the custom glyph ID.
                let (svg, content_type) = match input.id {
                    0 => (&svg_0, ContentType::Mask),
                    1 => (&svg_1, ContentType::Color),
                    _ => return None,
                };

                // Calculate the scale based on the "glyph size".
                let svg_size = svg.size();
                let scale_x = input.width as f32 / svg_size.width();
                let scale_y = input.height as f32 / svg_size.height();

                let mut pixmap =
                    resvg::tiny_skia::Pixmap::new(input.width as u32, input.height as u32)?;

                let mut transform = resvg::usvg::Transform::from_scale(scale_x, scale_y);

                // Offset the glyph by the subpixel amount.
                let offset_x = input.x_bin.as_float();
                let offset_y = input.y_bin.as_float();
                if offset_x != 0.0 || offset_y != 0.0 {
                    transform = transform.post_translate(offset_x, offset_y);
                }

                resvg::render(svg, transform, &mut pixmap.as_mut());

                let data: Vec<u8> = if let ContentType::Mask = content_type {
                    // Only use the alpha channel for symbolic icons.
                    pixmap.data().iter().skip(3).step_by(4).copied().collect()
                } else {
                    pixmap.data().to_vec()
                };

                Some(RasterizedCustomGlyph { data, content_type })
            };

        Self {
            device,
            queue,

            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text_buffer,
            rasterize_svg: Box::new(rasterize_svg),

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
            rasterize_svg,
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
                        .prepare_with_custom(
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
                                    right: 650,
                                    bottom: 180,
                                },
                                default_color: Color::rgb(255, 255, 255),
                                custom_glyphs: &[
                                    CustomGlyph {
                                        id: 0,
                                        left: 300.0,
                                        top: 5.0,
                                        width: 64.0,
                                        height: 64.0,
                                        color: Some(Color::rgb(200, 200, 255)),
                                        snap_to_physical_pixel: true,
                                        metadata: 0,
                                    },
                                    CustomGlyph {
                                        id: 1,
                                        left: 400.0,
                                        top: 5.0,
                                        width: 64.0,
                                        height: 64.0,
                                        color: None,
                                        snap_to_physical_pixel: true,
                                        metadata: 0,
                                    },
                                    CustomGlyph {
                                        id: 0,
                                        left: 300.0,
                                        top: 130.0,
                                        width: 64.0,
                                        height: 64.0,
                                        color: Some(Color::rgb(200, 255, 200)),
                                        snap_to_physical_pixel: true,
                                        metadata: 0,
                                    },
                                    CustomGlyph {
                                        id: 1,
                                        left: 400.0,
                                        top: 130.0,
                                        width: 64.0,
                                        height: 64.0,
                                        color: None,
                                        snap_to_physical_pixel: true,
                                        metadata: 0,
                                    },
                                ],
                            }],
                            swash_cache,
                            rasterize_svg,
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
                        red: 0.02,
                        green: 0.02,
                        blue: 0.02,
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

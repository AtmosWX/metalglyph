use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::ns_string;
use objc2_metal::{
    MTL4BlendState, MTL4Compiler, MTL4LibraryFunctionDescriptor, MTL4RenderPipelineDescriptor,
    MTLBlendFactor, MTLDevice, MTLPixelFormat, MTLRenderPipelineState,
};
use std::{
    ops::Deref,
    sync::{Arc, Mutex},
};

/// A cache to share common resources (e.g., pipelines, shaders) between multiple text
/// renderers.
#[derive(Debug, Clone)]
pub struct Cache(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    pipeline_descriptor: Retained<MTL4RenderPipelineDescriptor>,
    cache: Mutex<
        Vec<(
            MTLPixelFormat,
            Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
        )>,
    >,
}

impl Cache {
    /// Creates a new `Cache` with the given `device`.
    pub fn new(device: &Retained<ProtocolObject<dyn MTLDevice>>) -> Self {
        let library = device
            .newLibraryWithSource_options_error(ns_string!(include_str!("./shader.metal")), None)
            .expect("Failed to create shader library.");

        let descriptor = MTL4RenderPipelineDescriptor::new();

        let vertex_function = MTL4LibraryFunctionDescriptor::new();
        vertex_function.setLibrary(Some(&library));
        vertex_function.setName(Some(ns_string!("vs_main")));
        descriptor.setVertexFunctionDescriptor(Some(&vertex_function));

        let fragment_function = MTL4LibraryFunctionDescriptor::new();
        fragment_function.setLibrary(Some(&library));
        fragment_function.setName(Some(ns_string!("fs_main")));
        descriptor.setFragmentFunctionDescriptor(Some(&fragment_function));

        let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };

        attachment.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        attachment.setBlendingState(MTL4BlendState::Enabled);
        attachment.setSourceRGBBlendFactor(MTLBlendFactor::SourceAlpha);
        attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        attachment.setSourceAlphaBlendFactor(MTLBlendFactor::SourceAlpha);
        attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);

        Self(Arc::new(Inner {
            pipeline_descriptor: descriptor,
            cache: Mutex::new(Vec::new()),
        }))
    }

    pub(crate) fn get_or_create_pipeline(
        &self,
        compiler: &Retained<ProtocolObject<dyn MTL4Compiler>>,
        format: MTLPixelFormat,
    ) -> Retained<ProtocolObject<dyn MTLRenderPipelineState>> {
        let Inner {
            pipeline_descriptor,
            cache,
            ..
        } = self.0.deref();

        let mut cache = cache.lock().expect("Write pipeline cache");

        cache
            .iter()
            .find(|(fmt, _)| fmt == &format)
            .map(|(_, p)| p.clone())
            .unwrap_or_else(|| {
                let attachment = unsafe {
                    pipeline_descriptor
                        .colorAttachments()
                        .objectAtIndexedSubscript(0)
                };

                attachment.setPixelFormat(format);

                let pipeline = compiler
                    .newRenderPipelineStateWithDescriptor_compilerTaskOptions_error(
                        &pipeline_descriptor,
                        None,
                    )
                    .expect("Failed to create pipeline descriptor");

                cache.push((format, pipeline.clone()));

                pipeline
            })
            .clone()
    }
}

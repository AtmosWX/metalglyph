use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::ns_string;
use objc2_metal::{
    MTLBlendFactor, MTLDevice, MTLLibrary, MTLPixelFormat, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState,
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
    pipeline_descriptor: Retained<MTLRenderPipelineDescriptor>,
    cache: Mutex<
        Vec<(
            MTLPixelFormat,
            usize,
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

        let descriptor = MTLRenderPipelineDescriptor::new();

        let vertex_function = library.newFunctionWithName(ns_string!("vs_main"));
        descriptor.setVertexFunction(vertex_function.as_deref());

        let fragment_function = library.newFunctionWithName(ns_string!("fs_main"));
        descriptor.setFragmentFunction(fragment_function.as_deref());

        let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };

        attachment.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        attachment.setBlendingEnabled(true);
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
        device: &Retained<ProtocolObject<dyn MTLDevice>>,
        format: MTLPixelFormat,
        sample_count: usize,
    ) -> Retained<ProtocolObject<dyn MTLRenderPipelineState>> {
        let Inner {
            pipeline_descriptor,
            cache,
            ..
        } = self.0.deref();

        let mut cache = cache.lock().expect("Write pipeline cache");

        cache
            .iter()
            .find(|(fmt, count, _)| fmt == &format && count == &sample_count)
            .map(|(_, _, p)| p.clone())
            .unwrap_or_else(|| {
                pipeline_descriptor.setRasterSampleCount(sample_count);

                let attachment = unsafe {
                    pipeline_descriptor
                        .colorAttachments()
                        .objectAtIndexedSubscript(0)
                };

                attachment.setPixelFormat(format);

                let pipeline = device
                    .newRenderPipelineStateWithDescriptor_error(&pipeline_descriptor)
                    .expect("Failed to create pipeline state");

                cache.push((format, sample_count, pipeline.clone()));

                pipeline
            })
            .clone()
    }
}

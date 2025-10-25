use crate::{
    text_render::GlyphonCacheKey, Cache, ContentType, FontSystem, GlyphDetails, GpuCacheStatus,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, SwashCache,
};
use etagere::{size2, Allocation, BucketedAtlasAllocator};
use lru::LruCache;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::ns_string;
use objc2_metal::{
    MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLRenderPipelineState, MTLResource as _,
    MTLSize, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};
use rustc_hash::FxHasher;
use std::{collections::HashSet, hash::BuildHasherDefault, ptr::NonNull};

type Hasher = BuildHasherDefault<FxHasher>;

#[allow(dead_code)]
pub(crate) struct InnerAtlas {
    pub kind: Kind,
    pub texture: Retained<ProtocolObject<dyn MTLTexture>>,
    pub packer: BucketedAtlasAllocator,
    pub size: u32,
    pub glyph_cache: LruCache<GlyphonCacheKey, GlyphDetails, Hasher>,
    pub glyphs_in_use: HashSet<GlyphonCacheKey, Hasher>,
}

impl InnerAtlas {
    const INITIAL_SIZE: u32 = 256;
    const MAX_TEXTURE_DIMENSION_2D: u32 = 16384;

    fn new(device: &Retained<ProtocolObject<dyn MTLDevice>>, kind: Kind) -> Self {
        let size = Self::INITIAL_SIZE;
        let packer = BucketedAtlasAllocator::new(size2(size as i32, size as i32));

        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                kind.texture_format(),
                size as usize,
                size as usize,
                false,
            )
        };

        descriptor.setUsage(MTLTextureUsage::ShaderRead);

        let texture = device
            .newTextureWithDescriptor(&descriptor)
            .expect("Failed to create texture");
        texture.setLabel(Some(ns_string!("Metalglyph Atlas")));

        let glyph_cache = LruCache::unbounded_with_hasher(Hasher::default());
        let glyphs_in_use = HashSet::with_hasher(Hasher::default());

        Self {
            kind,
            texture,
            packer,
            size,
            glyph_cache,
            glyphs_in_use,
        }
    }

    pub(crate) fn try_allocate(&mut self, width: usize, height: usize) -> Option<Allocation> {
        let size = size2(width as i32, height as i32);

        loop {
            let allocation = self.packer.allocate(size);

            if allocation.is_some() {
                return allocation;
            }

            // Try to free least recently used allocation
            let (mut key, mut value) = self.glyph_cache.peek_lru()?;

            // Find a glyph with an actual size
            while value.atlas_id.is_none() {
                // All sized glyphs are in use, cache is full
                if self.glyphs_in_use.contains(key) {
                    return None;
                }

                let _ = self.glyph_cache.pop_lru();

                (key, value) = self.glyph_cache.peek_lru()?;
            }

            // All sized glyphs are in use, cache is full
            if self.glyphs_in_use.contains(key) {
                return None;
            }

            let (_, value) = self.glyph_cache.pop_lru().unwrap();
            self.packer.deallocate(value.atlas_id.unwrap());
        }
    }

    pub fn num_channels(&self) -> usize {
        self.kind.num_channels()
    }

    pub(crate) fn grow(
        &mut self,
        device: &Retained<ProtocolObject<dyn MTLDevice>>,
        font_system: &mut FontSystem,
        cache: &mut SwashCache,
        scale_factor: f32,
        mut rasterize_custom_glyph: impl FnMut(
            RasterizeCustomGlyphRequest,
        ) -> Option<RasterizedCustomGlyph>,
    ) -> bool {
        if self.size >= Self::MAX_TEXTURE_DIMENSION_2D {
            return false;
        }

        // Grow each dimension by a factor of 2. The growth factor was chosen to match the growth
        // factor of `Vec`.`
        const GROWTH_FACTOR: u32 = 2;
        let new_size = (self.size * GROWTH_FACTOR).min(Self::MAX_TEXTURE_DIMENSION_2D);

        self.packer.grow(size2(new_size as i32, new_size as i32));

        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                self.kind.texture_format(),
                new_size as usize,
                new_size as usize,
                false,
            )
        };

        descriptor.setUsage(MTLTextureUsage::ShaderRead);

        self.texture = device
            .newTextureWithDescriptor(&descriptor)
            .expect("Failed to create texture");
        self.texture.setLabel(Some(ns_string!("Metalglyph Atlas")));

        // Re-upload glyphs
        for (&cache_key, glyph) in &self.glyph_cache {
            let (x, y) = match glyph.gpu_cache {
                GpuCacheStatus::InAtlas { x, y, .. } => (x, y),
                GpuCacheStatus::SkipRasterization => continue,
            };

            let (image_data, width, height) = match cache_key {
                GlyphonCacheKey::Text(cache_key) => {
                    let image = cache.get_image_uncached(font_system, cache_key).unwrap();
                    let width = image.placement.width as usize;
                    let height = image.placement.height as usize;

                    (image.data, width, height)
                }
                GlyphonCacheKey::Custom(cache_key) => {
                    let input = RasterizeCustomGlyphRequest {
                        id: cache_key.glyph_id,
                        width: cache_key.width,
                        height: cache_key.height,
                        x_bin: cache_key.x_bin,
                        y_bin: cache_key.y_bin,
                        scale: scale_factor,
                    };

                    let Some(rasterized_glyph) = (rasterize_custom_glyph)(input) else {
                        panic!("Custom glyph rasterizer returned `None` when it previously returned `Some` for the same input {:?}", &input);
                    };

                    // Sanity checks on the rasterizer output
                    rasterized_glyph.validate(&input, Some(self.kind.as_content_type()));

                    (
                        rasterized_glyph.data,
                        cache_key.width as usize,
                        cache_key.height as usize,
                    )
                }
            };

            unsafe {
                self.texture
                    .replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                        MTLRegion {
                            origin: MTLOrigin {
                                x: x.into(),
                                y: y.into(),
                                z: 0,
                            },
                            size: MTLSize {
                                width,
                                height,
                                depth: 1,
                            },
                        },
                        0,
                        NonNull::from(image_data.as_slice()).cast(),
                        width * self.kind.num_channels(),
                    );
            }
        }

        self.size = new_size;

        true
    }

    fn trim(&mut self) {
        self.glyphs_in_use.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Mask,
    Color { srgb: bool },
}

impl Kind {
    fn num_channels(self) -> usize {
        match self {
            Kind::Mask => 1,
            Kind::Color { .. } => 4,
        }
    }

    fn texture_format(self) -> MTLPixelFormat {
        match self {
            Kind::Mask => MTLPixelFormat::R8Unorm,
            Kind::Color { srgb } => {
                if srgb {
                    MTLPixelFormat::RGBA8Unorm_sRGB
                } else {
                    MTLPixelFormat::RGBA8Unorm
                }
            }
        }
    }

    fn as_content_type(&self) -> ContentType {
        match self {
            Self::Mask => ContentType::Mask,
            Self::Color { .. } => ContentType::Color,
        }
    }
}

/// The color mode of a [`TextAtlas`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// Accurate color management.
    ///
    /// This mode will use a proper sRGB texture for colored glyphs. This will
    /// produce physically accurate color blending when rendering.
    Accurate,

    /// Web color management.
    ///
    /// This mode reproduces the color management strategy used in the Web and
    /// implemented by browsers.
    ///
    /// This entails storing glyphs colored using the sRGB color space in a
    /// linear RGB texture. Blending will not be physically accurate, but will
    /// produce the same results as most UI toolkits.
    ///
    /// This mode should be used to render to a linear RGB texture containing
    /// sRGB colors.
    Web,
}

/// An atlas containing a cache of rasterized glyphs that can be rendered.
pub struct TextAtlas {
    cache: Cache,
    pub(crate) color_atlas: InnerAtlas,
    pub(crate) mask_atlas: InnerAtlas,
    pub(crate) format: MTLPixelFormat,
    pub(crate) color_mode: ColorMode,
}

impl TextAtlas {
    /// Creates a new [`TextAtlas`].
    pub fn new(
        device: &Retained<ProtocolObject<dyn MTLDevice>>,
        cache: &Cache,
        format: MTLPixelFormat,
    ) -> Self {
        Self::with_color_mode(device, cache, format, ColorMode::Accurate)
    }

    /// Creates a new [`TextAtlas`] with the given [`ColorMode`].
    pub fn with_color_mode(
        device: &Retained<ProtocolObject<dyn MTLDevice>>,
        cache: &Cache,
        format: MTLPixelFormat,
        color_mode: ColorMode,
    ) -> Self {
        let color_atlas = InnerAtlas::new(
            device,
            Kind::Color {
                srgb: match color_mode {
                    ColorMode::Accurate => true,
                    ColorMode::Web => false,
                },
            },
        );

        let mask_atlas = InnerAtlas::new(device, Kind::Mask);

        Self {
            cache: cache.clone(),
            color_atlas,
            mask_atlas,
            format,
            color_mode,
        }
    }

    pub fn trim(&mut self) {
        self.mask_atlas.trim();
        self.color_atlas.trim();
    }

    pub(crate) fn grow(
        &mut self,
        device: &Retained<ProtocolObject<dyn MTLDevice>>,
        font_system: &mut FontSystem,
        cache: &mut SwashCache,
        content_type: ContentType,
        scale_factor: f32,
        rasterize_custom_glyph: impl FnMut(RasterizeCustomGlyphRequest) -> Option<RasterizedCustomGlyph>,
    ) -> bool {
        let did_grow = match content_type {
            ContentType::Mask => self.mask_atlas.grow(
                device,
                font_system,
                cache,
                scale_factor,
                rasterize_custom_glyph,
            ),
            ContentType::Color => self.color_atlas.grow(
                device,
                font_system,
                cache,
                scale_factor,
                rasterize_custom_glyph,
            ),
        };

        did_grow
    }

    pub(crate) fn inner_for_content_mut(&mut self, content_type: ContentType) -> &mut InnerAtlas {
        match content_type {
            ContentType::Color => &mut self.color_atlas,
            ContentType::Mask => &mut self.mask_atlas,
        }
    }

    pub(crate) fn get_or_create_pipeline(
        &self,
        device: &Retained<ProtocolObject<dyn MTLDevice>>,
        sample_count: usize,
        // depth_stencil: Option<DepthStencilState>,
    ) -> Retained<ProtocolObject<dyn MTLRenderPipelineState>> {
        self.cache
            .get_or_create_pipeline(device, self.format, sample_count)
    }
}

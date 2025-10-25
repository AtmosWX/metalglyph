use crate::{Params, Resolution};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::ns_string;
use objc2_metal::{MTLBuffer, MTLDevice, MTLResource as _, MTLResourceOptions};
use std::{mem, ptr::NonNull};

/// Controls the visible area of all text for a given renderer. Any text outside of the visible
/// area will be clipped.
///
/// Many projects will only ever need a single `Viewport`, but it is possible to create multiple
/// `Viewport`s if you want to render text to specific areas within a window (without having to)
/// bound each `TextArea`).
#[derive(Debug)]
pub struct Viewport {
    params: Params,
    pub(crate) buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
}

impl Viewport {
    /// Creates a new `Viewport` with the given `device`.
    pub fn new(device: &Retained<ProtocolObject<dyn MTLDevice>>) -> Self {
        let params = Params {
            screen_resolution: Resolution {
                width: 0,
                height: 0,
            },
        };

        let buffer = device
            .newBufferWithLength_options(
                mem::size_of::<Params>(),
                MTLResourceOptions::StorageModeShared,
            )
            .unwrap();
        buffer.setLabel(Some(ns_string!("Metalglyph Viewport Buffer")));

        Self { params, buffer }
    }

    /// Updates the `Viewport` with the given `resolution`.
    pub fn update(&mut self, resolution: Resolution) {
        if self.params.screen_resolution != resolution {
            self.params.screen_resolution = resolution;

            unsafe {
                self.buffer.contents().copy_from(
                    NonNull::from(&self.params).cast(),
                    std::mem::size_of::<Params>(),
                );
            }
        }
    }

    /// Returns the current resolution of the `Viewport`.
    pub fn resolution(&self) -> Resolution {
        self.params.screen_resolution
    }
}

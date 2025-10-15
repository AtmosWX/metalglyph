use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice};

pub struct State {
    pub device: Retained<ProtocolObject<dyn MTLDevice>>,
}

impl State {
    pub fn new() -> Self {
        let device = MTLCreateSystemDefaultDevice().expect("Create MTL device");

        Self { device }
    }
}

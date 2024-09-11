use crate::qmdriver::QmDriver;
use crate::qmdevice::QmDevice;


pub struct XeDriver
{
}

impl QmDriver for XeDriver
{
    fn name(&self) -> &str
    {
        "xe"
    }

    fn add_device(&self, qmd: &QmDevice)
    {
        println!("Xe driver new device: {:?}", qmd.drm_card);
    }
}

impl XeDriver
{
    pub fn new() -> Box<dyn QmDriver> {
        Box::new(XeDriver{})
    }
}

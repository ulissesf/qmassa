use crate::qmdevice::QmDevice;

mod xe;
use xe::XeDriver;


pub trait QmDriver
{
    fn name(&self) -> &str
    {
        "No QmDriver implemented"
    }

    fn add_device(&self, _qmd: &QmDevice)
    {
    }
}

impl std::fmt::Debug for dyn QmDriver
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
    {
        write!(f, "QmDriver(name={:?})", self.name())
    }
}

struct NotImplDriver
{
}

impl QmDriver for NotImplDriver
{
}

pub fn find_driver(dname: &str) -> Box<dyn QmDriver>
{
   let drvs = [("xe", XeDriver::new),];

   for (drv_name, drv_new_fn) in drvs {
       if drv_name == dname {
           return drv_new_fn();
       }
   }

   Box::new(NotImplDriver{})
}

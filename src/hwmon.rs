use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::fs;

use anyhow::Result;
use log::debug;


#[derive(Debug)]
pub struct Sensor
{
    pub sensor: String,
    pub label: String,
    items: HashSet<String>,
}

impl Sensor
{
    pub fn has_item(&self, item: &str) -> bool
    {
        self.items.contains(item)
    }

    fn set_item(&mut self, item: &str, fpath: &Path) -> Result<()>
    {
        if item == "label" {
            self.label = fs::read_to_string(fpath)?.trim().to_string();
            return Ok(());
        }

        self.items.insert(item.to_string());

        Ok(())
    }

    fn new(stype: &str) -> Sensor
    {
        Sensor {
            sensor: String::from(stype),
            label: String::new(),
            items: HashSet::new(),
        }
    }
}

#[derive(Debug)]
pub struct Hwmon
{
    pub base_dir: PathBuf,
    sensors: HashMap<String, Sensor>,
}

impl Hwmon
{
    pub fn read_sensor(&self, sty: &str, item: &str) -> Result<u64>
    {
        let sfile = format!("{}_{}", sty, item);
        let spath = self.base_dir.join(sfile);
        let val: u64 = fs::read_to_string(spath)?.trim().parse()?;

        Ok(val)
    }

    pub fn sensors(&self, stype: &str) -> Vec<&Sensor>
    {
        let mut res = Vec::new();

        for (sty, sensor) in self.sensors.iter() {
            if sty.starts_with(stype) {
                res.push(sensor);
            }
        }

        res
    }

    pub fn refresh(&mut self) -> Result<()>
    {
        for et in self.base_dir.read_dir()? {
            let et = et?;
            let epath = et.path();

            if epath.is_symlink() || epath.is_dir() || !epath.is_file() ||
                epath.ends_with("name") || epath.ends_with("uevent") {
                continue;
            }

            let fname = epath.file_name().unwrap().to_str().unwrap();
            let st_item = fname.split_once('_');
            if st_item.is_none() {
                continue;
            }

            let (sty, item) = st_item.unwrap();
            if sty.is_empty() || item.is_empty() {
                continue;
            }

            if !self.sensors.contains_key(sty) {
                self.sensors.insert(sty.to_string(), Sensor::new(sty));
            }
            let sensor = self.sensors.get_mut(sty).unwrap();

            sensor.set_item(item, &epath)?;
        }

        Ok(())
    }

    fn find_path(root_dir: &PathBuf) -> Result<Option<PathBuf>>
    {
        let hwmon_path = fs::read_dir(root_dir)?
            .into_iter()
            .filter(|r| r.is_ok())
            .map(|r| r.unwrap().path())
            .find(|r| r.file_name().unwrap()
                .to_str().unwrap().starts_with("hwmon"));

        Ok(hwmon_path.map(|path| path.to_path_buf()))
    }

    pub fn from(root_dir: PathBuf) -> Result<Option<Hwmon>>
    {
        let hwmon_dir = Hwmon::find_path(&root_dir)?;
        if hwmon_dir.is_none() {
            debug!("INF: no hwmon* in {:?}, aborting.", root_dir);
            return Ok(None);
        }
        let hwmon_dir = hwmon_dir.unwrap();

        let npath = hwmon_dir.join("name");
        if !npath.exists() {
            debug!("ERR: no name file in hwmon path {:?}, aborting.",
                hwmon_dir);
            return Ok(None);
        }

        // ignoring content of "name" file for now
        let mut hwmon = Hwmon {
            base_dir: hwmon_dir,
            sensors: HashMap::new(),
        };

        hwmon.refresh()?;

        Ok(Some(hwmon))
    }
}

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::collections::HashMap;

pub struct QmHwMon {
    base_path: PathBuf,
    variable_map: HashMap<String, HashMap<String, String>>
}

impl QmHwMon {
    pub fn new(base_path: &str) -> io::Result<Self> {
        let hwmon_path = Path::new(base_path).join("hwmon");
        let hwmon_dir = fs::read_dir(hwmon_path)?
            .filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().starts_with("hwmon"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No hwmon directory found"))?;
            let test_file_path = hwmon_dir.path().join("name");
            if !test_file_path.exists() {
                return Err(io::Error::new(io::ErrorKind::NotFound, "HWmon required file 'name' not found"));
            }
            let mut test_file = fs::File::open(&test_file_path)?;
            let mut contents = String::new();
            test_file.read_to_string(&mut contents)?;
        
        let mut variable_map: HashMap<String, HashMap<String, String>> = HashMap::new();

        for entry in fs::read_dir(hwmon_dir.path())? {
            let entry = entry?;
            let path = entry.path();

            // Ensure the path is a file
            if path.is_file() {
                // Get the filename
                if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
                    // Split the filename at the first underscore
                    if let Some((variable_name, key)) = file_name.split_once('_') {
                        // Read the file content
                        let mut file_content = String::new();
                        fs::File::open(&path)?.read_to_string(&mut file_content)?;
    
                        // Insert into the nested HashMap
                        // Check if the variable (outer key) exists, if not, insert a new HashMap
                        let entry_map = variable_map.entry(variable_name.to_string()).or_insert_with(HashMap::new);
                        
                        // Insert the key and file content into the inner HashMap
                        entry_map.insert(key.to_string(), file_content);
                    }
                }
            }

        }

        Ok(QmHwMon {
            base_path: hwmon_dir.path(),
            variable_map: variable_map,
        })
    }

    fn print_variable_map(&self) {
        for (variable, map) in &self.variable_map {
            println!("{}:", variable);
            for (key, value) in map {
                println!("  {}: {}", key, value);
            }
        }
    }

    fn read_file(&self, file_name: &str) -> io::Result<String> {
        let path = self.base_path.join(file_name);
        let mut file = fs::File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        Ok(contents.trim().to_string())
    }

    pub fn power1_max(&self) -> io::Result<String> {
        self.read_file("power1_max")
    }

    pub fn power1_rated_max(&self) -> io::Result<String> {
        self.read_file("power1_rated_max")
    }

    pub fn energy1_input(&self) -> io::Result<String> {
        self.read_file("energy1_input")
    }

    pub fn power1_max_interval(&self) -> io::Result<String> {
        self.read_file("power1_max_interval")
    }

    pub fn power2_max(&self) -> io::Result<String> {
        self.read_file("power2_max")
    }

    pub fn power2_rated_max(&self) -> io::Result<String> {
        self.read_file("power2_rated_max")
    }

    pub fn power2_crit(&self) -> io::Result<String> {
        self.read_file("power2_crit")
    }

    pub fn curr2_crit(&self) -> io::Result<String> {
        self.read_file("curr2_crit")
    }

    pub fn energy2_input(&self) -> io::Result<String> {
        self.read_file("energy2_input")
    }

    pub fn power2_max_interval(&self) -> io::Result<String> {
        self.read_file("power2_max_interval")
    }

    pub fn in1_input(&self) -> io::Result<String> {
        self.read_file("in1_input")
    }
}

fn main() {
    match QmHwMon::new("/home/rdvivi/") {
        Ok(hwmon) => {
           hwmon.print_variable_map();
        }
        Err(e) => eprintln!("Error initializing QmHwMon: {}", e),
    }
}
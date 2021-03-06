
use std::io::{Read, Seek, SeekFrom};
use std::error::Error;
use std::fs::{self, File};

use std::io::copy;

use config::Config;
use errors::GenericError;
use zfs;

use reqwest;
use tempfile;
use serde_json;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use prettytable::Table;
use prettytable::format;
use prettytable::row::Row;
use prettytable::cell::Cell;
use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;


#[derive(Debug, Serialize, Deserialize, Clone)]
struct ManifestWrapper {
    zpool: String,
    manifest: Image
}

impl ManifestWrapper {
    pub fn from_reader<R>(reader: R) -> Result<Self, Box<Error>>
        where
        R: Read,
    {
        let manifest: ManifestWrapper = serde_json::from_reader(reader)?;
        return Ok(manifest);
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ImageFile {
    size: u64,
    compression: String,
    sha1: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Image {
    v: u32,
    uuid: Uuid,
    name: String,
    version: String,
    #[serde(rename = "type")]
    image_type: String,
    os: String,
    origin: Option<Uuid>,
    #[serde(default = "empty_files")]
    files: Vec<ImageFile>,
    published_at: Option<DateTime<Utc>>,
    public: bool,
    state: String,
    disabled: bool,
}

impl Image {
    pub fn from_reader<R>(reader: R) -> Result<Self, Box<Error>>
    where
        R: Read,
    {
        let image: Image = serde_json::from_reader(reader)?;
        return Ok(image);
    }

    pub fn list_from_reader<R>(reader: R) -> Result<Vec<Self>, Box<Error>>
    where
        R: Read,
    {
        let images: Vec<Image> = serde_json::from_reader(reader)?;
        return Ok(images);
    }
    fn print(&self, table: &mut Table, parsable: bool) {

        let date = match self.published_at{
            Some(published_at) =>
                format!("{}", published_at.format("%Y-%m-%d")),
            _ => String::from("-")
        };
        if parsable {
            println!(
                "{}:{}:{}:{}:{}:{}",
                self.uuid,
                self.name,
                self.version,
                self.os,
                self.image_type,
                date
            )
        } else {
            table.add_row(Row::new(vec![
                Cell::new(self.uuid.hyphenated().to_string().as_str()),
                Cell::new(self.name.as_str()),
                Cell::new(self.version.as_str()),
                Cell::new(self.os.as_str()),
                Cell::new(self.image_type.as_str()),
                Cell::new(date.as_str()),
            ]));

        }
    }
}

fn print_images(images: Vec<Image>, headerless: bool, parsable: bool) {
    let mut table = Table::new();
    table.set_format(*format::consts::FORMAT_CLEAN);
    if !headerless {
        if parsable {
            println!(
                "{}:{}:{}:{}:{}:{}",
                "UUID",
                "NAME",
                "VERSION",
                "OS",
                "TYPE",
                "PUB"
            );
        } else {
            table.add_row(row!["UUID", "NAME", "VERSION", "OS", "TYPE", "PUB"]);
        }
    }
    for image in images.iter() {
        image.print(&mut table, parsable)
    }
    if !parsable {
        table.printstd()
    };

}
pub fn list(config: &Config) -> Result<i32, Box<Error>> {
    let mut images: Vec<Image> = Vec::new();
    for entry in fs::read_dir(config.settings.image_dir.clone())? {
        let entry = entry?;
        let image_file = File::open(entry.path())?;
        let manifest = ManifestWrapper::from_reader(image_file)?;
        images.push(manifest.manifest);
    }
    print_images(images, false, false);
    Ok(0)
}
pub fn avail(config: &Config) -> Result<i32, Box<Error>> {
    debug!("Listing images"; "repo" => config.settings.repo.clone());
    let resp = reqwest::get(config.settings.repo.as_str())?;
    let images = Image::list_from_reader(resp)?;
    print_images(images, false, false);
    Ok(0)
}

pub fn get(config: &Config, uuid: Uuid) -> Result<i32, Box<Error>> {
    let mut file_name = config.settings.image_dir.clone();
    let uuid_str = uuid.hyphenated().to_string();
    file_name.push('/');
    file_name.push_str(uuid_str.as_str());
    file_name.push_str(".json");
    debug!("Get image"; "dir" => config.settings.image_dir.clone(),
           "uuid" => uuid_str.clone(), "filer" => file_name.clone());
    let file = File::open(file_name)?;
    let manifest = ManifestWrapper::from_reader(file)?;
    let j = serde_json::to_string_pretty(&manifest.manifest)?;
    println!("{}\n", j);
    //print_images(images, false, false);
    Ok(0)
}

pub fn show(config: &Config, uuid: Uuid) -> Result<i32, Box<Error>> {
    let mut url = config.settings.repo.clone();
    let uuid_str = uuid.hyphenated().to_string();
    url.push('/');
    url.push_str(uuid_str.as_str());
    debug!("Fethcing image"; "repo" => config.settings.repo.clone(),
           "uuid" => uuid_str.clone(), "url" => url.clone());
    let resp = reqwest::get(url.as_str())?;
    let image = Image::from_reader(resp)?;
    let j = serde_json::to_string_pretty(&image)?;
    println!("{}\n", j);
    //print_images(images, false, false);
    Ok(0)
}


pub fn import(config: &Config, uuid: Uuid) -> Result<i32, Box<Error>> {

    let mut url = config.settings.repo.clone();
    let uuid_str = uuid.hyphenated().to_string();
    let mut dataset = config.settings.pool.clone();
    dataset.push('/');
    dataset.push_str(uuid_str.as_str());
    url.push('/');
    url.push_str(uuid_str.as_str());

    if zfs::is_present(dataset.as_str()) {
        println!("Image {} already imported.", uuid_str);
        return Ok(0);
    };

    debug!("Fethcing image"; "repo" => config.settings.repo.clone(),
           "uuid" => uuid_str.clone(), "url" => url.clone());
    let resp = reqwest::get(url.as_str())?;
    let image = Image::from_reader(resp)?;

    match image.origin {
        None => (),
        Some(origin) => {
            let mut origin_dataset = config.settings.pool.clone();
            origin_dataset.push('/');
            origin_dataset.push_str(origin.hyphenated().to_string().as_str());
            if ! zfs::is_present(origin_dataset.as_str()) {
                import(config, origin)?;
            }
        }
    };
    let file_info = image.files[0].clone();
    url.push_str("/file");
    let mut out: File = tempfile::tempfile()?;
    let mut resp = reqwest::get(url.as_str())?;
    println!("Importing {} ...", uuid_str.as_str());
    copy(&mut resp, &mut out)?;
    println!("Importing {} ...", uuid_str.as_str());
    out.seek(SeekFrom::Start(0))?;
    match file_info.compression.as_str() {
        "bzip2" => {
            let mut decompressor = BzDecoder::new(out);
            zfs::receive(dataset.as_str(), &mut decompressor)?;
        }
        "gzip" => {
            let mut decompressor = GzDecoder::new(out)?;
            zfs::receive(dataset.as_str(), &mut decompressor)?;
        }
        compression => {
            println!("Encountered {} compression", compression);
            return Err(GenericError::bx("Only bzip2 compression is supporred for images."));
        }
    }
    let mut cfg_path = config.settings.image_dir.clone();
    cfg_path.push('/');
    cfg_path.push_str(config.settings.pool.replace("/", "-").as_str());
    cfg_path.push('-');
    cfg_path.push_str(uuid_str.as_str());
    cfg_path.push_str(".json");
    println!("Writing manifest file: {}", cfg_path);
    let cfg_file = File::create(cfg_path)?;
    let manifest = ManifestWrapper{
        manifest: image,
        zpool: config.settings.pool.clone()
    };
    serde_json::to_writer(cfg_file, &manifest)?;
    Ok(0)
}

fn empty_files() -> Vec<ImageFile> {
    Vec::new()
}

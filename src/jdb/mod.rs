//! Jail database

use std::error::Error;
use std::fs;
use std::fs::File;
use std::path::PathBuf;
use std::str;
use std::cmp::PartialEq;
use std::collections::HashMap;
use std::slice::Iter;

use prettytable::Table;
use prettytable::format;
use prettytable::row::Row;
use prettytable::cell::Cell;
use uuid::Uuid;
use serde_json;

use jails::Jail;
use jails;
use jail_config::JailConfig;

use errors::{NotFoundError, ConflictError, GenericError};
use config::Config;

/// `JailDB` index entry
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IdxEntry {
    version: u32,
    /// UUID of the jail
    pub uuid: Uuid,
    /// ZFS dataset root
    pub root: String,
    state: String,
    jail_type: String,
}

#[cfg(test)]
impl IdxEntry {
    pub fn empty() -> Self {
        IdxEntry {
            version: 1,
            uuid: Uuid::nil(),
            root: String::from("zroot"),
            state: String::from("stopped"),
            jail_type: String::from("jail"),
        }
    }
}


impl PartialEq for IdxEntry {
    fn eq(&self, other: &IdxEntry) -> bool {
        self.uuid == other.uuid
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Index {
    pub version: u32,
    pub entries: Vec<IdxEntry>,
}

/// `JailDB` main struct
#[derive(Debug)]
pub struct JDB<'a> {
    config: &'a Config,
    index: Index,
    jails: HashMap<String, jails::JailOSEntry>,
}

impl<'a> JDB<'a> {
    /// Opens an JDB index file.
    ///„ # Arguments
    ///
    /// * `path` - Path of the **index file**, the locatio of the
    ///            file is also where the seperate configs live.
    ///
    /// # Example
    ///
    /// ```
    /// // Open jail config folder in /usr/local/etc/vmadm
    /// use jdb::JDB;
    /// let db = JDB::open("/usr/local/etc/vmadm/index");
    /// ```

    pub fn open(config: &'a Config) -> Result<Self, Box<Error>> {
        let mut idx_file = PathBuf::from(config.settings.conf_dir.as_str());
        idx_file.push("index");
        debug!("Opening jdb"; "index" => idx_file.to_string_lossy().as_ref());
        match File::open(idx_file) {
            Ok(file) => {
                let index: Index = serde_json::from_reader(file)?;
                debug!("Found {} entries", index.entries.len());
                Ok(JDB {
                    index: index,
                    config: config,
                    jails: jails::list()?,
                })
            }
            Err(_) => {
                warn!("No database found creating new one.");
                let entries: Vec<IdxEntry> = Vec::new();
                let index: Index = Index {
                    version: 0,
                    entries: entries,
                };
                let db = JDB {
                    index: index,
                    config: config,
                    jails: jails::list()?,
                };
                db.save()?;
                Ok(db)

            }

        }
    }

    /// Inserts a config into the database, writes the config file
    /// and adds it to the index.
    pub fn insert(self: &'a mut JDB<'a>, config: JailConfig) -> Result<IdxEntry, Box<Error>> {
        debug!("Inserting new vm"; "vm" => &config.uuid.hyphenated().to_string());
        match self.find(&config.uuid) {
            None => {
                let mut path = PathBuf::from(self.config.settings.conf_dir.as_str());
                path.push(config.uuid.hyphenated().to_string());
                path.set_extension("json");
                let file = File::create(path)?;
                let mut root = String::from(self.config.settings.pool.as_str());
                root.push('/');
                root.push_str(&config.uuid.hyphenated().to_string());
                let e = IdxEntry {
                    version: 0,
                    uuid: config.uuid.clone(),
                    state: String::from("stopped"),
                    jail_type: String::from("base"),
                    root: root.clone(),
                };
                self.index.entries.push(e);
                self.save()?;
                serde_json::to_writer(file, &config)?;
                // This is ugly but I don't know any better.
                Ok(IdxEntry {
                    version: 0,
                    uuid: config.uuid.clone(),
                    state: String::from("stopped"),
                    jail_type: String::from("base"),
                    root: root.clone(),
                })
            }
            Some(_) => {
                warn!("Doublicate entry {}", config.uuid);
                Err(ConflictError::bx(&config.uuid))
            }
        }
    }

    /// Inserts a config into the database, writes the config file
    /// and adds it to the index.
    pub fn update(self: &'a mut JDB<'a>, config: JailConfig) -> Result<i32, Box<Error>> {
        debug!("Updating vm"; "vm" => &config.uuid.hyphenated().to_string());
        match self.find(&config.uuid) {
            None => {
                warn!("Missing entry {}", config.uuid; "vm" => &config.uuid.hyphenated().to_string());
                Err(NotFoundError::bx(&config.uuid))
            }

            Some(_) => {
                let mut path = PathBuf::from(self.config.settings.conf_dir.as_str());
                path.push(config.uuid.hyphenated().to_string());
                path.set_extension("json");
                debug!("Updating config file"; "file" => path.to_str(), "vm" => &config.uuid.hyphenated().to_string());
                let file = File::create(path)?;
                serde_json::to_writer(file, &config)?;
                // This is ugly but I don't know any better.
                Ok(0)
            }
        }
    }

    /// Removes a jail with a given uuid from the index and removes it's
    /// config file.
    pub fn remove(self: &'a mut JDB<'a>, uuid: &Uuid) -> Result<usize, Box<Error>> {
        debug!("Removing vm"; "vm" => uuid.hyphenated().to_string());
        match self.find(uuid) {
            None => Err(NotFoundError::bx(uuid)),
            Some(index) => {
                // remove the config file first
                let mut path = PathBuf::from(self.config.settings.conf_dir.as_str());
                path.push(uuid.hyphenated().to_string());
                path.set_extension("json");
                fs::remove_file(&path)?;
                self.index.entries.remove(index);
                self.save()?;
                Ok(index)
            }
        }
    }

    /// Reads the config file for a given entry
    fn config(self: &'a JDB<'a>, entry: &IdxEntry) -> Result<JailConfig, Box<Error>> {
        debug!("Loading vm config"; "vm" => &entry.uuid.hyphenated().to_string());
        let mut config_path = PathBuf::from(self.config.settings.conf_dir.as_str());
        config_path.push(entry.uuid.hyphenated().to_string());
        config_path.set_extension("json");
        match config_path.to_str() {
            Some(path) => JailConfig::from_file(path),
            None => Err(GenericError::bx("could not generate vm config path")),
        }
    }
    /// Saves the database
    fn save(self: &'a JDB<'a>) -> Result<usize, Box<Error>> {
        debug!("Saving database");
        let mut path = PathBuf::from(self.config.settings.conf_dir.as_str());
        path.push("index");
        let file = File::create(path)?;
        serde_json::to_writer(file, &self.index)?;
        Ok(self.index.entries.len())
    }

    /// Fetches a `Jail` from the `JDB`.
    pub fn get(self: &'a JDB<'a>, uuid: &Uuid) -> Result<Jail, Box<Error>> {
        match self.find(uuid) {
            None => Err(NotFoundError::bx(uuid)),
            Some(index) => {
                // with nested jails we need to
                let uuid_str = uuid.hyphenated().to_string();
                let mut inner_uuid = uuid_str.clone();
                inner_uuid.push('.');
                inner_uuid.push_str(uuid_str.as_str());
                let entry = &self.index.entries[index];
                let config = self.config(entry)?;
                let jail = Jail {
                    idx: entry,
                    inner: self.jails.get(inner_uuid.as_str()),
                    outer: self.jails.get(uuid_str.as_str()),
                    config: config,
                };
                Ok(jail)
            }
        }
    }

    /// Finds an entry for a given uuid
    fn find(self: &'a JDB<'a>, uuid: &Uuid) -> Option<usize> {
        self.index.entries.iter().position(|x| x.uuid == *uuid)

    }
    /// Iterator over index entries
    pub fn iter(self: &'a JDB<'a>) -> Iter<'a, IdxEntry> {
        self.index.entries.iter()
    }

    /// Prints the jdb database
    pub fn print(self: &'a JDB<'a>, headerless: bool, parsable: bool) -> Result<i32, Box<Error>> {
        let mut table = Table::new();
        table.set_format(*format::consts::FORMAT_CLEAN);
        if !headerless {
            if parsable {
                println!("{}:{}:{}:{}:{}", "UUID", "TYPE", "RAM", "STATE", "ALIAS");
            } else {
                table.add_row(row!["UUID", "TYPE", "RAM", "STATE", "ALIAS"]);
            }
        }
        for e in self.iter() {
            self.print_entry(e, &mut table, parsable)?;
        }
        if !parsable {
            table.printstd()
        };
        Ok(0)
    }

    /// Gets the config and prints an etry
    fn print_entry(
        self: &'a JDB<'a>,
        entry: &IdxEntry,
        table: &mut Table,
        parsable: bool,
    ) -> Result<i32, Box<Error>> {
        let conf = self.config(entry)?;
        let id = match self.jails.get(&conf.uuid.hyphenated().to_string()) {
            Some(jail) => jail.id,
            _ => 0,
        };
        let os = match conf.brand.as_str() {
            "jail" => "OS",
            "lx-jail" => "LX",
            brand => {
                warn!("Unknown brand: {}.", brand);
                "OS"
            }
        };
        let state = match id {
            0 => &entry.state,
            _ => "running",
        };
        if parsable {
            println!(
                "{}:{}:{}:{}:{}",
                conf.uuid,
                os,
                conf.max_physical_memory,
                state,
                conf.alias
            );
        } else {
            table.add_row(Row::new(vec![
                Cell::new(conf.uuid.hyphenated().to_string().as_str()),
                Cell::new(os),
                Cell::new(conf.max_physical_memory.to_string().as_str()),
                Cell::new(state),
                Cell::new(conf.alias.as_str()),
            ]));
        };
        Ok(0)
    }
}

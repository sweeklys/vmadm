//! Jail Configuration

use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::process::Command;
#[cfg(target_os = "freebsd")]
use errors::GenericError;

use errors::ValidationError;
use config::Config;

use serde_json;
use uuid::Uuid;
use regex::Regex;
use rand::{thread_rng, Rng};

use std::collections::BTreeMap as Map;

/// Jail configuration values
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NIC {
    /// Interface name
    pub interface: String,
    /// Mac address for the nic
    #[serde(default = "new_mac")]
    pub mac: String,
    /// VLAN id for the nic
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
    /// The nic_tag for the nic to uise
    pub nic_tag: String,
    /// The IP for the nic
    pub ip: String,
    /// The netmask for the nic
    pub netmask: String,
    /// The gateway for the nic
    pub gateway: String,
    #[serde(default = "dflt_false")]
    /// If this nic is the primary interface or not
    pub primary: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// The MTU for the nic
    pub mtu: Option<u32>,
    /// Network UUID for the nic
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_uuid: Option<Uuid>,
}

impl PartialEq for NIC {
    fn eq(&self, other: &NIC) -> bool {
        self.interface == other.interface &&
            self.mac == other.mac &&
            self.vlan == other.vlan &&
            self.nic_tag == other.nic_tag &&
            self.ip == self.ip &&
            self.netmask == other.netmask &&
            self.gateway == other.gateway &&
            self.primary == other.primary &&
            self.mtu == other.mtu &&
            self.network_uuid == other.network_uuid
    }
}

#[cfg(target_os = "freebsd")]
static IFCONFIG: &'static str = "/sbin/ifconfig";

/// Interface after creating
#[derive(Debug, Clone)]
pub struct IFace {
    /// The interface name
    pub iface: String,
    /// epair
    pub epair: String,
    /// Startup script for the jail
    pub start_script: String,
}

impl NIC {
    /// Creates the related interface
    #[cfg(target_os = "freebsd")]
    pub fn get_iface(&self, config: &Config, uuid: &Uuid) -> Result<IFace, Box<Error>> {
        let output = Command::new(IFCONFIG)
            .args(&["epair", "create", "up"])
            .output()
            .expect("failed ifconfig");
        if !output.status.success() {
            return Err(GenericError::bx("could not create interface"));
        }
        let reply = String::from_utf8_lossy(&output.stdout);
        let epaira = reply.trim();
        let mut epair = String::from(epaira);

        epair.pop();
        match config.settings.networks.get(&self.nic_tag) {
            Some(bridge) => {

                let output = Command::new(IFCONFIG)
                    .args(&[bridge.as_str(), "addm", epaira])
                    .output()
                    .expect("failed ifconfig");

                if !output.status.success() {
                    return Err(GenericError::bx("could not add epair to bridge"));
                }
            }
            None => return Err(GenericError::bx("bridge not configured")),
        }

        let mut script = if self.vlan.is_some() {
            // This may seem stupid but freebsd can't create a vlan interface
            // that is not named vlan<X> or <interface>.<X>
            // however once created it happiely renames it ...
            format!(
                "/sbin/ifconfig {epair}b.{vlan} create vlan {vlan} vlandev {epair}p; \
                /sbin/ifconfig {epair}b.{vlan} name {iface}; \
                /sbin/ifconfig {iface} inet {ip} netmask {mask}; ",
                epair = epair,
                ip = self.ip,
                mask = self.netmask,
                iface = self.interface,
                vlan = self.vlan.unwrap()
            )
        } else {
            format!(
                "/sbin/ifconfig {epair}b name {iface}; \
                /sbin/ifconfig {iface} inet {ip} netmask {mask}; ",
                epair = epair,
                ip = self.ip,
                mask = self.netmask,
                iface = self.interface
            )
        };
        let mut desc = String::from("VNic from jail ");
        desc.push_str(uuid.hyphenated().to_string().as_str());
        let output = Command::new(IFCONFIG)
            .args(&[epaira, "description", desc.as_str()])
            .output()
            .expect("failed to add descirption");
        if !output.status.success() {
            return Err(GenericError::bx("could not set description"));
        }
        Ok(IFace {
            iface: self.interface.clone(),
            epair: epair,
            start_script: script,
        })
    }
    /// Creates the related interface
    #[cfg(not(target_os = "freebsd"))]
    pub fn get_iface(&self, _config: &Config, _uuid: &Uuid) -> Result<IFace, Box<Error>> {
        let epair = "epair0";
        let script = if self.vlan.is_some() {
            // This may seem stupid but freebsd can't create a vlan interface
            // that is not named vlan<X> or <interface>.<X>
            // however once created it happiely renames it ...
            format!(
                "/sbin/ifconfig {epair}b name {iface}p; \
                /sbin/ifconfig {iface}p.{vlan} create vlan {vlan} vlandev {iface}p; \
                /sbin/ifconfig {iface}p.{vlan} name {iface}; \
                /sbin/ifconfig {iface} inet {ip} netmask {mask}; ",
                epair = epair,
                ip = self.ip,
                mask = self.netmask,
                iface = self.interface,
                vlan = self.vlan.unwrap()
            )
        } else {
            format!(
                "/sbin/ifconfig {epair}b name {iface}; \
                /sbin/ifconfig {iface} inet {ip} netmask {mask}; ",
                epair = epair,
                ip = self.ip,
                mask = self.netmask,
                iface = self.interface
            )
        };

        Ok(IFace {
            iface: self.interface.clone(),
            epair: String::from(epair),
            start_script: script,
        })
    }
}

/// Jail configuration values
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JailConfig {
    /// UUID of the jail
    #[serde(default = "dflt_brand")]
    pub brand: String,
    /// UUID of the jail
    #[serde(default = "new_uuid")]
    pub uuid: Uuid,
    /// UUID of the imaage
    pub image_uuid: Uuid,
    /// readable alias for the jail
    pub alias: String,
    /// hostname of the jail
    pub hostname: String,

    #[serde(default = "empty_resolvers")]
    pub resolvers: Vec<String>,
    /// weather to start this jail on --startup
    #[serde(default = "dflt_false")]
    pub autoboot: bool,

    // Resources
    /// max physical memory in MB (memoryuse)
    pub max_physical_memory: u64,
    /// mac cpu usage 100 = 1 core (pcpu)
    pub cpu_cap: u64,
    /// max quota (zfs quota)
    pub quota: u64,

    /// SysV shared memory size, in bytes (shmsize)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_shm_memory: Option<u64>,

    /// locked memory (memorylocked)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_locked_memory: Option<u64>,

    /// networks
    #[serde(default = "empty_nics")]
    pub nics: Vec<NIC>,

    /// maximum number of porocesses (maxproc)
    #[serde(default = "dflt_max_lwp")]
    pub max_lwps: u64,

    // Metadata fields w/o effect on vmadm at the moment
    /// Should be archived when deleted
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_on_delete: Option<bool>,
    /// Bulling ID for the jail
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_id: Option<Uuid>,
    /// This jail should not be be part of inventories
    #[serde(skip_serializing_if = "Option::is_none")]
    pub do_not_inventory: Option<bool>,
    // Currently has no effect
    /// dns domain for the jail
    #[serde(default = "dflt_dns_domain")]
    pub dns_domain: String,
    // currently no effect
    /// Prevent the jail delegate to be destroyed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indestructible_delegated: Option<bool>,
    // currenlty no effect
    /// Prevent the jail root to be destroyed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indestructible_zoneroot: Option<bool>,
    /// UUID of the owner of the jail
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_uuid: Option<Uuid>,
    /// Name of the package used for this jail
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    /// Version of the package used for this jail
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
    #[serde(default = "empty_map")]
    pub routes: Map<String, String>,
    // TODO:
    #[serde(default = "empty_map")]
    pub customer_metadata: Map<String, String>,
    #[serde(default = "empty_map")]
    pub internal_metadata: Map<String, String>,
    // internal_metadata_namespaces: Vec<String>
    // zfs_data_compression
}

impl PartialEq for JailConfig {
    fn eq(&self, other: &JailConfig) -> bool {
        self.brand == other.brand &&
            self.uuid == other.uuid &&
            self.image_uuid == other.image_uuid &&
            self.alias == other.alias &&
            self.hostname == other.hostname &&
            self.autoboot == other.autoboot &&
            self.max_physical_memory == other.max_physical_memory &&
            self.cpu_cap == other.cpu_cap &&
            self.quota == other.quota &&
            self.max_shm_memory == other.max_shm_memory &&
            self.max_locked_memory == other.max_locked_memory &&
            self.nics == other.nics &&
            self.max_lwps == other.max_lwps &&
            self.archive_on_delete == other.archive_on_delete &&
            self.billing_id == other.billing_id &&
            self.do_not_inventory == other.do_not_inventory &&
            self.dns_domain == other.dns_domain &&
            self.indestructible_delegated == other.indestructible_delegated &&
            self.indestructible_zoneroot == other.indestructible_zoneroot &&
            self.owner_uuid == other.owner_uuid &&
            self.package_name == other.package_name &&
            self.routes == other.routes &&
            self.package_version == other.package_version
    }
}

lazy_static! {
  static ref HOSTNAME_RE: Regex = Regex::new("^[a-zA-Z0-9]([a-zA-Z0-9-]{0,253}[a-zA-Z0-9])?$").unwrap();
  static ref ALIAS_RE: Regex = Regex::new("^[a-zA-Z0-9]([a-zA-Z0-9-]{0,253}[a-zA-Z0-9])?$").unwrap();
  static ref INTERFACE_RE: Regex = Regex::new("^[a-zA-Z]{1,4}[0-9]{0,3}$").unwrap();
    static ref IP_RE: Regex = Regex::new("^(([0-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-5])\\.){3}([0-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-5])$").unwrap();
    static ref NET_RE: Regex = Regex::new("^(([0-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-5])\\.){3}([0-9]|[1-9][0-9]|1[0-9]{2}|2[0-4][0-9]|25[0-5])(/((3[0-2])|([12][0-9])|[0-9]))?$").unwrap();
  static ref MAC_RE: Regex = Regex::new("^[a-fA-F0-9]{1,2}([:][a-fA-F0-9]{1,2}){5}$").unwrap();
}

impl JailConfig {
    /// Reads a new config from a file
    pub fn from_file(config_path: &str) -> Result<Self, Box<Error>> {
        let config_file = File::open(config_path)?;
        JailConfig::from_reader(config_file)
    }

    /// Reads the config from a reader
    pub fn from_reader<R>(reader: R) -> Result<Self, Box<Error>>
    where
        R: Read,
    {
        let mut conf: JailConfig = serde_json::from_reader(reader)?;
        let max_physical_memory = conf.max_physical_memory;
        if conf.max_shm_memory.is_none() {
            conf.max_shm_memory = Some(max_physical_memory);
        }
        if conf.max_locked_memory.is_none() {
            conf.max_locked_memory = Some(max_physical_memory);
        }
        Ok(conf)
    }
    /// checks the config for errors
    pub fn errors(&self, config: &Config) -> Option<Vec<ValidationError>> {
        let mut errors = Vec::new();
        if !HOSTNAME_RE.is_match(self.hostname.as_str()) {
            errors.push(ValidationError::new("hostname", "Invalid hostname"))
        }
        if !ALIAS_RE.is_match(self.alias.as_str()) {
            errors.push(ValidationError::new("alias", "Invalid alias"))
        }
        let mut i = 0;
        for nic in self.nics.clone() {
            if !INTERFACE_RE.is_match(nic.interface.as_str()) {
                errors.push(ValidationError::new(
                    format!("nic[{}]", i).as_str(),
                    "Invalid interface name",
                ))
            }
            if !IP_RE.is_match(nic.ip.as_str()) {
                errors.push(ValidationError::new(
                    format!("nic[{}]", i).as_str(),
                    "Invalid ip",
                ))
            }
            if !IP_RE.is_match(nic.netmask.as_str()) {
                errors.push(ValidationError::new(
                    format!("nic[{}]", i).as_str(),
                    "Invalid netmask",
                ))
            }
            if !IP_RE.is_match(nic.gateway.as_str()) {
                errors.push(ValidationError::new(
                    format!("nic[{}]", i).as_str(),
                    "Invalid gateway",
                ))
            }

            if checkip(nic.ip.as_str())  {
                errors.push(ValidationError::new(
                    format!("nic[{}]", i).as_str(),
                    "ip address already taken",
                ))
            }

            if !MAC_RE.is_match(nic.mac.as_str()) {
                errors.push(ValidationError::new(
                    format!("nic[{}]", i).as_str(),
                    "Invalid mac",
                ))
            }
            if !config.settings.networks.contains_key(&nic.nic_tag) {
                errors.push(ValidationError::new(
                    format!("nic[{}]", i).as_str(),
                    "Unknown nic_tag",
                ))

            }
            i = i + 1;
        }
        for (dest, gw) in self.routes.iter() {
            if !NET_RE.is_match(dest.as_str()) {
                errors.push(ValidationError::new(
                    format!("routes {} -> {}", dest, gw).as_str(),
                    "Invalid destination",
                ))
            }
            if !IP_RE.is_match(gw.as_str()) && !INTERFACE_RE.is_match(gw.as_str()) {
                errors.push(ValidationError::new(
                    format!("routes {} -> {}", dest, gw).as_str(),
                    "Invalid gateway",
                ))
            }
        }
        if errors.is_empty() {
            None
        } else {
            Some(errors)
        }

    }

    /// Translates the config into resource controle limts
    pub fn rctl_limits(&self) -> Vec<String> {
        let mut res = Vec::new();
        let uuid = self.uuid.clone();
        let mut base = String::from("jail:");
        base.push_str(uuid.hyphenated().to_string().as_str());

        res.push(String::from("-a"));

        let max_physical_memory = self.max_physical_memory.to_string();
        let mut mem = base.clone();
        mem.push_str(":memoryuse:deny=");
        mem.push_str(max_physical_memory.as_str());
        mem.push_str("M");
        res.push(mem);

        let mut memorylocked = base.clone();
        memorylocked.push_str(":memorylocked:deny=");
        match self.max_locked_memory {
            Some(max_locked_memory) => {
                memorylocked.push_str(max_locked_memory.to_string().as_str())
            }
            None => memorylocked.push_str(max_physical_memory.as_str()),
        }
        memorylocked.push_str("M");
        res.push(memorylocked);

        let mut shmsize = base.clone();
        shmsize.push_str(":shmsize:deny=");
        match self.max_shm_memory {
            Some(max_shm_memory) => shmsize.push_str(max_shm_memory.to_string().as_str()),
            None => shmsize.push_str(max_physical_memory.as_str()),
        }
        shmsize.push_str("M");
        res.push(shmsize);

        let mut pcpu = base.clone();
        pcpu.push_str(":pcpu:deny=");
        pcpu.push_str(self.cpu_cap.to_string().as_str());
        res.push(pcpu);


        let mut maxproc = base.clone();
        maxproc.push_str(":maxproc:deny=");
        maxproc.push_str(self.max_lwps.to_string().as_str());
        res.push(maxproc);

        res
    }
}

fn dflt_false() -> bool {
    false
}

fn dflt_max_lwp() -> u64 {
    2000
}

fn dflt_dns_domain() -> String {
    String::from("local")
}

fn new_uuid() -> Uuid {
    Uuid::new_v4()
}

fn empty_nics() -> Vec<NIC> {
    Vec::new()
}

fn empty_resolvers() -> Vec<String> {
    Vec::new()
}
fn dflt_brand() -> String {
    String::from("jail")
}

fn empty_map() -> Map<String, String> {
    Map::new()
}

fn new_mac() -> String {
    let mut rng = thread_rng();
    // the second half of the first ocet should be 02
    // to indicate this is a locally administered mac
    // address and not one assiged by a vendor.
    format!(
        "02:{:x}:{:x}:{:x}:{:x}:{:x}",
        rng.gen::<u8>(),
        rng.gen::<u8>(),
        rng.gen::<u8>(),
        rng.gen::<u8>(),
        rng.gen::<u8>()
    )
}

fn checkip(ipaddr: &str) -> bool {
    debug!("Checking if ip address {} is used up ",ipaddr);
    let output = Command::new("ping")
        .args(&["-o","-c 1",ipaddr])
        .output()
	.expect("failed to ping");
    output.status.success() 
}

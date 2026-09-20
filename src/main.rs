use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

mod equalizer;
mod hid;
mod pipewire;
mod spatial;
mod spatial_pipewire;
mod state;

const USB_ROOT: &str = "/sys/bus/usb/devices";
const JBL_VENDOR_ID: &str = "0ecb";
const QUANTUM_810_PRODUCT_ID: &str = "2069";
const QUANTUM_810_USB_C_PRODUCT_ID: &str = "206a";
const BATTERY_FEATURE_REPORT_ID: u8 = 0x49;
const BATTERY_FEATURE_REPORT_LEN: usize = 2;
// Read-only reports observed in the QuantumENGINE startup sequence. This list
// intentionally excludes every report the vendor software writes.
const STATUS_PROBE_REPORTS: &[(u8, usize)] = &[
    (0x68, 2),
    (0x67, 2),
    (0x62, 2),
    (0x5c, 2),
    (0x75, 2),
    (0x49, 2),
    (0x51, 13),
    (0x47, 2),
    (0x4a, 2),
    (0x45, 2),
];

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn ioctl(fd: std::ffi::c_int, request: std::ffi::c_ulong, data: *mut u8) -> std::ffi::c_int;
}

#[cfg(target_os = "linux")]
const fn hidiocgfeature(length: usize) -> std::ffi::c_ulong {
    const IOC_WRITE: std::ffi::c_ulong = 1;
    const IOC_READ: std::ffi::c_ulong = 2;
    const IOC_SIZE_SHIFT: u32 = 16;
    const IOC_DIR_SHIFT: u32 = 30;
    ((IOC_READ | IOC_WRITE) << IOC_DIR_SHIFT)
        | ((length as std::ffi::c_ulong) << IOC_SIZE_SHIFT)
        | ((b'H' as std::ffi::c_ulong) << 8)
        | 0x07
}

#[cfg(target_os = "linux")]
const fn hidiocsfeature(length: usize) -> std::ffi::c_ulong {
    const IOC_WRITE: std::ffi::c_ulong = 1;
    const IOC_READ: std::ffi::c_ulong = 2;
    const IOC_SIZE_SHIFT: u32 = 16;
    const IOC_DIR_SHIFT: u32 = 30;
    ((IOC_READ | IOC_WRITE) << IOC_DIR_SHIFT)
        | ((length as std::ffi::c_ulong) << IOC_SIZE_SHIFT)
        | ((b'H' as std::ffi::c_ulong) << 8)
        | 0x06
}

#[derive(Debug, PartialEq, Eq)]
struct Endpoint {
    address: u8,
    attributes: u8,
    max_packet_size: u16,
    interval: u8,
}

impl Serialize for Endpoint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Endpoint", 6)?;
        state.serialize_field("address", &format!("0x{:02x}", self.address))?;
        state.serialize_field("direction", self.direction())?;
        state.serialize_field("transfer_type", self.transfer_type())?;
        state.serialize_field("attributes", &format!("0x{:02x}", self.attributes))?;
        state.serialize_field("max_packet_size", &self.max_packet_size)?;
        state.serialize_field("interval", &self.interval)?;
        state.end()
    }
}

impl Endpoint {
    fn direction(&self) -> &'static str {
        if self.address & 0x80 != 0 {
            "IN"
        } else {
            "OUT"
        }
    }

    fn transfer_type(&self) -> &'static str {
        match self.attributes & 0x03 {
            0 => "control",
            1 => "isochronous",
            2 => "bulk",
            3 => "interrupt",
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Interface {
    number: u8,
    alternate_setting: u8,
    class: u8,
    subclass: u8,
    protocol: u8,
    endpoints: Vec<Endpoint>,
}

#[derive(Debug, Serialize)]
struct InterfaceReport {
    number: u8,
    alternate_setting: u8,
    active: bool,
    class: String,
    subclass: String,
    protocol: String,
    driver: String,
    endpoints: Vec<Endpoint>,
}

#[derive(Debug, Serialize)]
struct AlsaPcm {
    node: String,
    direction: String,
    role: Option<String>,
}

#[derive(Debug, Serialize)]
struct AlsaCard {
    number: u32,
    id: Option<String>,
    pcms: Vec<AlsaPcm>,
}

#[derive(Debug, Serialize)]
struct DeviceReport {
    vendor_id: String,
    product_id: String,
    manufacturer: String,
    product: String,
    revision: String,
    speed_mbps: String,
    sysfs: String,
    descriptor_bytes: usize,
    interfaces: Vec<InterfaceReport>,
    alsa: Vec<AlsaCard>,
    hid: Option<hid::DescriptorSummary>,
    confirmed_input_mappings: Vec<hid::KnownInputMapping>,
}

#[derive(Serialize)]
struct Export<'a> {
    schema: u8,
    devices: &'a [DeviceReport],
}

#[derive(Debug, Default, PartialEq, Eq)]
struct StatusOptions {
    dry_run: bool,
    json: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct SetCommand {
    feature: &'static str,
    value: String,
    reports: Vec<Vec<u8>>,
}

#[derive(Debug, Serialize)]
struct StatusOutput {
    schema: u8,
    device: &'static str,
    hidraw: String,
    access: &'static str,
    dry_run: bool,
    battery_percent: Option<u8>,
    charging: Option<bool>,
    raw_feature: Option<String>,
    headset_connected: Option<bool>,
    ambient_mode: Option<String>,
    microphone: Option<String>,
    lighting_enabled: Option<bool>,
    lighting_color: Option<String>,
    logo_color: Option<String>,
    ring_color: Option<String>,
    logo_colors: Option<Vec<String>>,
    ring_colors: Option<Vec<String>>,
    logo_effect: Option<String>,
    ring_effect: Option<String>,
    logo_speed: Option<String>,
    ring_speed: Option<String>,
    game_chat_value: Option<u8>,
    bluetooth: Option<String>,
    sidetone_level: Option<String>,
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn supported_devices() -> io::Result<Vec<PathBuf>> {
    let mut devices = Vec::new();
    for entry in fs::read_dir(USB_ROOT)? {
        let path = entry?.path();
        if read_trimmed(path.join("idVendor")).as_deref() == Some(JBL_VENDOR_ID)
            && read_trimmed(path.join("idProduct")).as_deref() == Some(QUANTUM_810_PRODUCT_ID)
        {
            devices.push(path);
        }
    }
    devices.sort();
    Ok(devices)
}

fn charging_usb_connected() -> io::Result<bool> {
    for entry in fs::read_dir(USB_ROOT)? {
        let path = entry?.path();
        if read_trimmed(path.join("idVendor")).as_deref() == Some(JBL_VENDOR_ID)
            && read_trimmed(path.join("idProduct")).as_deref() == Some(QUANTUM_810_USB_C_PRODUCT_ID)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn spawn_charging_watcher() -> Result<(), String> {
    thread::Builder::new()
        .name("soniccore-usb-power".into())
        .spawn(|| {
            let mut confirmed = state::load().and_then(|cached| cached.charging);
            let mut candidate = None;
            let mut candidate_samples = 0_u8;
            loop {
                thread::sleep(Duration::from_millis(250));
                let Ok(current) = charging_usb_connected() else {
                    continue;
                };
                if confirmed == Some(current) {
                    candidate = None;
                    candidate_samples = 0;
                    continue;
                }
                if candidate == Some(current) {
                    candidate_samples += 1;
                } else {
                    candidate = Some(current);
                    candidate_samples = 1;
                }
                if candidate_samples >= 3 {
                    let mut cached = state::load().unwrap_or_default();
                    cached.charging = Some(current);
                    if let Err(error) = state::save(&mut cached) {
                        eprintln!("failed to cache USB power state: {error}");
                    } else {
                        confirmed = Some(current);
                    }
                    candidate = None;
                    candidate_samples = 0;
                }
            }
        })
        .map(|_| ())
        .map_err(|error| format!("failed to start USB power watcher: {error}"))
}

fn parse_descriptors(data: &[u8]) -> Result<Vec<Interface>, String> {
    let mut interfaces: Vec<Interface> = Vec::new();
    let mut offset = 0;
    while offset + 2 <= data.len() {
        let length = data[offset] as usize;
        let descriptor_type = data[offset + 1];
        if length < 2 || offset + length > data.len() {
            return Err(format!(
                "invalid descriptor length {length} at offset {offset}"
            ));
        }
        let descriptor = &data[offset..offset + length];
        match descriptor_type {
            4 if length >= 9 => interfaces.push(Interface {
                number: descriptor[2],
                alternate_setting: descriptor[3],
                class: descriptor[5],
                subclass: descriptor[6],
                protocol: descriptor[7],
                endpoints: Vec::new(),
            }),
            5 if length >= 7 => {
                if let Some(interface) = interfaces.last_mut() {
                    interface.endpoints.push(Endpoint {
                        address: descriptor[2],
                        attributes: descriptor[3],
                        max_packet_size: u16::from_le_bytes([descriptor[4], descriptor[5]])
                            & 0x07ff,
                        interval: descriptor[6],
                    });
                }
            }
            _ => {}
        }
        offset += length;
    }
    if offset != data.len() {
        return Err(format!("trailing descriptor byte at offset {offset}"));
    }
    Ok(interfaces)
}

fn interface_path(device: &Path, number: u8) -> Option<PathBuf> {
    let name = device.file_name()?.to_str()?;
    Some(device.with_file_name(format!("{name}:1.{number}")))
}

fn driver_name(path: &Path) -> String {
    fs::read_link(path.join("driver"))
        .ok()
        .and_then(|driver| {
            driver
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "none".into())
}

fn active_alternate(path: &Path) -> Option<u8> {
    read_trimmed(path.join("bAlternateSetting"))?.parse().ok()
}

fn alsa_cards(device: &Path) -> Vec<AlsaCard> {
    let Ok(device_real) = fs::canonicalize(device) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir("/sys/class/sound") else {
        return Vec::new();
    };
    let mut cards = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(number) = name
            .strip_prefix("card")
            .and_then(|value| value.parse().ok())
        else {
            continue;
        };
        let Ok(card_real) = fs::canonicalize(entry.path()) else {
            continue;
        };
        if !card_real.starts_with(&device_real) {
            continue;
        }
        let mut pcms = Vec::new();
        if let Ok(sound_entries) = fs::read_dir("/sys/class/sound") {
            for pcm in sound_entries.flatten() {
                let pcm_name = pcm.file_name().to_string_lossy().into_owned();
                let prefix = format!("pcmC{number}D");
                if !pcm_name.starts_with(&prefix) {
                    continue;
                }
                let direction = match pcm_name.chars().last() {
                    Some('p') => "playback",
                    Some('c') => "capture",
                    _ => continue,
                };
                let role = if pcm_name == format!("pcmC{number}D0p") {
                    Some("game".into())
                } else if pcm_name == format!("pcmC{number}D1p") {
                    Some("chat".into())
                } else if pcm_name == format!("pcmC{number}D0c") {
                    Some("microphone".into())
                } else {
                    None
                };
                pcms.push(AlsaPcm {
                    node: pcm_name,
                    direction: direction.into(),
                    role,
                });
            }
        }
        pcms.sort_by(|left, right| left.node.cmp(&right.node));
        cards.push(AlsaCard {
            number,
            id: read_trimmed(entry.path().join("id")),
            pcms,
        });
    }
    cards.sort_by_key(|card| card.number);
    cards
}

fn device_reports() -> Result<Vec<DeviceReport>, String> {
    let devices = supported_devices().map_err(|error| error.to_string())?;
    let mut reports = Vec::new();
    for device in devices {
        let descriptor_data =
            fs::read(device.join("descriptors")).map_err(|error| error.to_string())?;
        let parsed = parse_descriptors(&descriptor_data)?;
        let mut interfaces = Vec::new();
        for interface in parsed {
            let path = interface_path(&device, interface.number)
                .ok_or_else(|| "invalid sysfs device path".to_owned())?;
            interfaces.push(InterfaceReport {
                number: interface.number,
                alternate_setting: interface.alternate_setting,
                active: active_alternate(&path) == Some(interface.alternate_setting),
                class: format!("{:02x}", interface.class),
                subclass: format!("{:02x}", interface.subclass),
                protocol: format!("{:02x}", interface.protocol),
                driver: driver_name(&path),
                endpoints: interface.endpoints,
            });
        }
        reports.push(DeviceReport {
            vendor_id: JBL_VENDOR_ID.into(),
            product_id: QUANTUM_810_PRODUCT_ID.into(),
            manufacturer: read_trimmed(device.join("manufacturer"))
                .unwrap_or_else(|| "unknown".into()),
            product: read_trimmed(device.join("product")).unwrap_or_else(|| "unknown".into()),
            revision: read_trimmed(device.join("bcdDevice")).unwrap_or_else(|| "unknown".into()),
            speed_mbps: read_trimmed(device.join("speed")).unwrap_or_else(|| "unknown".into()),
            sysfs: device.display().to_string(),
            descriptor_bytes: descriptor_data.len(),
            interfaces,
            alsa: alsa_cards(&device),
            hid: read_quantum_hid_summary()?,
            confirmed_input_mappings: hid::confirmed_mappings(),
        });
    }
    Ok(reports)
}

fn scan() -> io::Result<bool> {
    let devices = supported_devices()?;
    for path in &devices {
        let name = read_trimmed(path.join("product")).unwrap_or_else(|| "unknown".into());
        println!("found {JBL_VENDOR_ID}:{QUANTUM_810_PRODUCT_ID} {name}");
        println!("sysfs={}", path.display());
    }
    Ok(!devices.is_empty())
}

fn inspect() -> Result<bool, String> {
    let reports = device_reports()?;
    for report in &reports {
        println!("Device {JBL_VENDOR_ID}:{QUANTUM_810_PRODUCT_ID}");
        println!("  manufacturer: {}", report.manufacturer);
        println!("  product: {}", report.product);
        println!("  revision: {}", report.revision);
        println!("  speed: {} Mbit/s", report.speed_mbps);
        println!("  sysfs: {}", report.sysfs);
        println!("  descriptor bytes: {}", report.descriptor_bytes);
        println!("  interfaces:");
        for interface in &report.interfaces {
            let marker = if interface.active {
                "active"
            } else {
                "inactive"
            };
            println!(
                "    interface {} alt {} ({marker}): class {}/{}/{}, driver {}",
                interface.number,
                interface.alternate_setting,
                interface.class,
                interface.subclass,
                interface.protocol,
                interface.driver,
            );
            for endpoint in &interface.endpoints {
                println!(
                    "      endpoint 0x{:02x} {}: {}, max packet {} B, interval {}",
                    endpoint.address,
                    endpoint.direction(),
                    endpoint.transfer_type(),
                    endpoint.max_packet_size,
                    endpoint.interval,
                );
            }
        }
        println!("  ALSA:");
        for card in &report.alsa {
            println!(
                "    card {}: {}",
                card.number,
                card.id.as_deref().unwrap_or("unknown")
            );
            for pcm in &card.pcms {
                let role = pcm
                    .role
                    .as_deref()
                    .map(|role| format!(", role={role}"))
                    .unwrap_or_default();
                println!("      {}: {}{}", pcm.node, pcm.direction, role);
            }
        }
    }
    Ok(!reports.is_empty())
}

fn export_json() -> Result<bool, String> {
    let reports = device_reports()?;
    let export = Export {
        schema: 1,
        devices: &reports,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&export).map_err(|error| error.to_string())?
    );
    Ok(!reports.is_empty())
}

fn hid_descriptor() -> Result<bool, String> {
    if let Some(summary) = read_quantum_hid_summary()? {
        println!("HID report descriptor");
        println!("  descriptor bytes: {}", summary.descriptor_bytes);
        println!("  usage pages: {}", summary.usage_pages.join(", "));
        println!("  collections: {}", summary.collections);
        println!("  reports:");
        for report in summary.reports {
            let id = report
                .id
                .map_or_else(|| "none".into(), |id| format!("0x{id:02x}"));
            println!(
                "    ID {id}: {}, {} bits / {} payload bytes / {} wire bytes",
                report.kind.label(),
                report.payload_bits,
                report.payload_bytes,
                report.wire_bytes,
            );
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

fn read_quantum_hid_summary() -> Result<Option<hid::DescriptorSummary>, String> {
    let Some(device) = quantum_hid_device()? else {
        return Ok(None);
    };
    let bytes = fs::read(device.join("report_descriptor")).map_err(|error| error.to_string())?;
    hid::parse(&bytes).map(Some)
}

fn quantum_hid_device() -> Result<Option<PathBuf>, String> {
    let entries = fs::read_dir("/sys/bus/hid/devices").map_err(|error| error.to_string())?;
    for entry in entries.flatten() {
        let uevent = read_trimmed(entry.path().join("uevent")).unwrap_or_default();
        if uevent.contains("HID_ID=0003:00000ECB:00002069") {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

fn quantum_hidraw_node() -> Result<Option<PathBuf>, String> {
    let devices = fs::read_dir("/sys/bus/hid/devices").map_err(|error| error.to_string())?;
    for device in devices.flatten() {
        let uevent = read_trimmed(device.path().join("uevent")).unwrap_or_default();
        if !uevent.contains("HID_ID=0003:00000ECB:00002069") {
            continue;
        }
        let Ok(entries) = fs::read_dir(device.path().join("hidraw")) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let node = Path::new("/dev").join(name);
            if name.starts_with("hidraw")
                && name[6..]
                    .chars()
                    .all(|character| character.is_ascii_digit())
                && fs::metadata(&node).is_ok_and(|metadata| metadata.file_type().is_char_device())
            {
                return Ok(Some(node));
            }
        }
    }
    Ok(None)
}

fn format_hid_report(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn monitor(dry_run: bool) -> Result<bool, String> {
    let Some(node) = quantum_hidraw_node()? else {
        return Ok(false);
    };
    println!("matched JBL Quantum 810 HID node: {}", node.display());
    println!("access mode: read-only (no Output/Feature reports)");
    if dry_run {
        match fs::metadata(&node) {
            Ok(metadata) if metadata.file_type().is_char_device() => {
                println!("dry run: character device exists; no open/read performed");
            }
            Ok(_) => return Err(format!("refusing non-character device: {}", node.display())),
            Err(error) => println!("dry run: device node is not accessible here: {error}"),
        }
        return Ok(true);
    }

    let metadata = fs::metadata(&node).map_err(|error| format!("{}: {error}", node.display()))?;
    if !metadata.file_type().is_char_device() {
        return Err(format!("refusing non-character device: {}", node.display()));
    }
    // File::open uses read-only access. This code has no write handle and makes
    // no ioctl/control request; it can only consume spontaneous input reports.
    let mut device = fs::File::open(&node).map_err(|error| {
        if error.kind() == io::ErrorKind::PermissionDenied {
            format!(
                "{}: permission denied; install the repository's narrow udev rule, then reconnect the dongle",
                node.display()
            )
        } else {
            format!("{}: {error}", node.display())
        }
    })?;
    println!("waiting for spontaneous input reports; press Ctrl-C to stop");
    let mut buffer = [0_u8; 64];
    loop {
        let count = device
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err("hidraw returned end-of-file".into());
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?;
        let interpretation = hid::interpret_input(&buffer[..count])
            .map(|meaning| format!("  {meaning}"))
            .unwrap_or_default();
        println!(
            "{}.{:03}  len={}  {}{}",
            timestamp.as_secs(),
            timestamp.subsec_millis(),
            count,
            format_hid_report(&buffer[..count]),
            interpretation,
        );
    }
}

fn daemon() -> Result<bool, String> {
    state::init_signal_service()?;
    spawn_charging_watcher()?;
    let mut runtime = state::load().unwrap_or_default();
    loop {
        let Some(node) = quantum_hidraw_node()? else {
            if let Some(cached) = state::load() {
                runtime = cached;
            }
            if runtime.headset_connected != Some(false) {
                runtime.headset_connected = Some(false);
                state::save(&mut runtime)?;
            }
            thread::sleep(Duration::from_secs(2));
            continue;
        };

        if let Some(cached) = state::load() {
            runtime = cached;
        }
        if let Ok((battery, _)) = query_battery(&node) {
            runtime.battery_percent = Some(battery);
        }
        state::save(&mut runtime)?;

        let mut device = match fs::File::open(&node) {
            Ok(device) => device,
            Err(error) => {
                eprintln!("{}: {error}; retrying", node.display());
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        let mut buffer = [0_u8; 64];
        loop {
            match device.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if let Some(cached) = state::load() {
                        runtime = cached;
                    }
                    if runtime.apply_input(&buffer[..count]) {
                        state::save(&mut runtime)?;
                    }
                }
                Err(error) => {
                    eprintln!("{}: {error}; reconnecting", node.display());
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn read_feature_report_read_only(
    node: &Path,
    report_id: u8,
    length: usize,
) -> Result<Vec<u8>, String> {
    if length == 0 {
        return Err("Feature Report length must include the report ID".into());
    }
    let metadata = fs::metadata(node).map_err(|error| format!("{}: {error}", node.display()))?;
    if !metadata.file_type().is_char_device() {
        return Err(format!("refusing non-character device: {}", node.display()));
    }
    let device = OpenOptions::new().read(true).open(node).map_err(|error| {
        if error.kind() == io::ErrorKind::PermissionDenied {
            format!(
                "{}: permission denied; install the repository's narrow udev rule, then reconnect the dongle",
                node.display()
            )
        } else {
            format!("{}: {error}", node.display())
        }
    })?;
    let mut buffer = vec![0_u8; length];
    buffer[0] = report_id;
    #[cfg(target_os = "linux")]
    // SAFETY: `device` remains open for the call, `buffer` is writable for the
    // exact length encoded in the ioctl, and HIDIOCGFEATURE is a GET_REPORT
    // operation. This function has no SET_REPORT or write path.
    let count = unsafe {
        ioctl(
            device.as_raw_fd(),
            hidiocgfeature(buffer.len()),
            buffer.as_mut_ptr(),
        )
    };
    #[cfg(not(target_os = "linux"))]
    let count = -1;
    if count < 0 {
        return Err(format!(
            "HIDIOCGFEATURE 0x{report_id:02x} failed: {}",
            io::Error::last_os_error()
        ));
    }
    buffer.truncate(count as usize);
    Ok(buffer)
}

fn set_feature_report_allowlisted(node: &Path, report: &[u8]) -> Result<(), String> {
    let metadata = fs::metadata(node).map_err(|error| format!("{}: {error}", node.display()))?;
    if !metadata.file_type().is_char_device() {
        return Err(format!("refusing non-character device: {}", node.display()));
    }
    let device = OpenOptions::new()
        .read(true)
        .write(true)
        .open(node)
        .map_err(|error| format!("{}: {error}", node.display()))?;
    let mut buffer = report.to_vec();
    #[cfg(target_os = "linux")]
    // SAFETY: the device remains open, the buffer length matches the ioctl
    // size, and callers can only supply reports assembled by the strict
    // allowlist in `parse_set_command`.
    let count = unsafe {
        ioctl(
            device.as_raw_fd(),
            hidiocsfeature(buffer.len()),
            buffer.as_mut_ptr(),
        )
    };
    #[cfg(not(target_os = "linux"))]
    let count = -1;
    if count != buffer.len() as std::ffi::c_int {
        return Err(format!(
            "HIDIOCSFEATURE 0x{:02x} failed: {}",
            report[0],
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn preset_rgb(value: &str) -> Option<(u8, u8, u8)> {
    match value {
        "blue" => Some((0x00, 0x29, 0xff)),
        "cyan" => Some((0x33, 0xff, 0xcc)),
        "magenta" => Some((0xff, 0x00, 0xcc)),
        "red" => Some((0xff, 0x20, 0x20)),
        "green" => Some((0x20, 0xff, 0x66)),
        "white" => Some((0xff, 0xff, 0xff)),
        _ => None,
    }
}

fn color_rgb(value: &str) -> Option<(u8, u8, u8)> {
    if let Some(rgb) = preset_rgb(value) {
        return Some(rgb);
    }
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let packed = u32::from_str_radix(hex, 16).ok()?;
    Some(((packed >> 16) as u8, (packed >> 8) as u8, packed as u8))
}

fn canonical_color(value: &str) -> Option<String> {
    let (red, green, blue) = color_rgb(value)?;
    Some(format!("#{red:02x}{green:02x}{blue:02x}"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LightingZone {
    colors: Vec<String>,
    effect: String,
    speed: String,
}

fn effect_id(value: &str) -> Option<u8> {
    match value {
        "breathing" => Some(0x00),
        "solid" => Some(0x01),
        "wave" => Some(0x02),
        "glitch" => Some(0x03),
        _ => None,
    }
}

fn speed_byte(value: &str) -> Option<u8> {
    match value {
        "0.5" => Some(0x64),
        "1.0" => Some(0x4b),
        "1.5" => Some(0x32),
        "2.0" => Some(0x19),
        _ => None,
    }
}

fn zone_profile(zone: u8, profile: &LightingZone) -> Result<Vec<Vec<u8>>, String> {
    if profile.colors.len() != 5 {
        return Err("lighting profile must contain exactly five colors".into());
    }
    let speed = speed_byte(&profile.speed).ok_or("unsupported cached lighting speed")?;
    let effect = effect_id(&profile.effect).ok_or("unsupported cached lighting effect")?;
    let mut reports = Vec::with_capacity(6);
    reports.push(vec![0x4c, zone, speed, 0x05]);
    for (segment, color) in profile.colors.iter().enumerate() {
        let (red, green, blue) = color_rgb(color).ok_or("unsupported cached lighting color")?;
        reports.push(vec![
            0x4d,
            zone,
            segment as u8,
            red,
            green,
            blue,
            effect,
            segment as u8 * 2,
        ]);
    }
    Ok(reports)
}

fn solid_color_profile(logo: (u8, u8, u8), ring: (u8, u8, u8)) -> Vec<Vec<u8>> {
    let to_hex = |(red, green, blue)| format!("#{red:02x}{green:02x}{blue:02x}");
    let logo = LightingZone {
        colors: vec![to_hex(logo); 5],
        effect: "solid".into(),
        speed: "0.5".into(),
    };
    let ring = LightingZone {
        colors: vec![to_hex(ring); 5],
        effect: "solid".into(),
        speed: "0.5".into(),
    };
    let mut reports = zone_profile(0, &logo).expect("built-in solid profile is valid");
    reports.extend(zone_profile(1, &ring).expect("built-in solid profile is valid"));
    reports.push(vec![0x4b, 0x01]);
    reports
}

fn cached_zone(
    colors: Option<Vec<String>>,
    legacy_color: Option<String>,
    effect: Option<String>,
    speed: Option<String>,
    name: &str,
) -> Result<LightingZone, String> {
    let colors = match colors {
        Some(colors)
            if colors.len() == 5 && colors.iter().all(|color| color_rgb(color).is_some()) =>
        {
            colors
        }
        _ => vec![
            legacy_color.ok_or_else(|| {
                format!("{name} color is unknown; apply a synchronized color first")
            })?;
            5
        ],
    };
    Ok(LightingZone {
        colors,
        effect: effect.unwrap_or_else(|| "solid".into()),
        speed: speed.unwrap_or_else(|| "0.5".into()),
    })
}

fn lighting_reports(logo: &LightingZone, ring: &LightingZone) -> Result<Vec<Vec<u8>>, String> {
    let mut reports = zone_profile(0, logo)?;
    reports.extend(zone_profile(1, ring)?);
    reports.push(vec![0x4b, 0x01]);
    Ok(reports)
}

fn segment_feature(value: &str) -> Option<(&'static str, usize)> {
    match value {
        "segment-0" => Some(("segment-0", 0)),
        "segment-1" => Some(("segment-1", 1)),
        "segment-2" => Some(("segment-2", 2)),
        "segment-3" => Some(("segment-3", 3)),
        "segment-4" => Some(("segment-4", 4)),
        "logo-segment-0" => Some(("logo-segment-0", 0)),
        "logo-segment-1" => Some(("logo-segment-1", 1)),
        "logo-segment-2" => Some(("logo-segment-2", 2)),
        "logo-segment-3" => Some(("logo-segment-3", 3)),
        "logo-segment-4" => Some(("logo-segment-4", 4)),
        "ring-segment-0" => Some(("ring-segment-0", 0)),
        "ring-segment-1" => Some(("ring-segment-1", 1)),
        "ring-segment-2" => Some(("ring-segment-2", 2)),
        "ring-segment-3" => Some(("ring-segment-3", 3)),
        "ring-segment-4" => Some(("ring-segment-4", 4)),
        _ => None,
    }
}

fn parse_set_command(mut args: impl Iterator<Item = String>) -> Result<SetCommand, String> {
    let feature = args.next().ok_or("set requires a feature")?;
    let value = args.next().ok_or("set requires a value")?;
    if args.next().is_some() {
        return Err("set accepts exactly one feature and one value".into());
    }
    let command = match (feature.as_str(), value.as_str()) {
        ("ambient", "off") => SetCommand {
            feature: "ambient",
            value: "off".into(),
            reports: vec![vec![0x46, 0x00]],
        },
        ("ambient", "anc") => SetCommand {
            feature: "ambient",
            value: "anc".into(),
            reports: vec![vec![0x46, 0x01]],
        },
        ("ambient", "talkthru") => SetCommand {
            feature: "ambient",
            value: "talkthru".into(),
            reports: vec![vec![0x46, 0x02]],
        },
        ("lighting", "off") => SetCommand {
            feature: "lighting",
            value: "off".into(),
            reports: vec![vec![0x4b, 0x00]],
        },
        ("lighting", "on") => SetCommand {
            feature: "lighting",
            value: "on".into(),
            reports: vec![vec![0x4b, 0x01]],
        },
        ("color", value) if color_rgb(value).is_some() => {
            let rgb = color_rgb(value).expect("validated RGB color");
            SetCommand {
                feature: "color",
                value: canonical_color(value).expect("validated RGB color"),
                reports: solid_color_profile(rgb, rgb),
            }
        }
        (feature @ ("logo-color" | "ring-color"), value) if color_rgb(value).is_some() => {
            SetCommand {
                feature: if feature == "logo-color" {
                    "logo-color"
                } else {
                    "ring-color"
                },
                value: canonical_color(value).expect("validated RGB color"),
                reports: Vec::new(),
            }
        }
        (feature @ ("effect" | "logo-effect" | "ring-effect"), value)
            if effect_id(value).is_some() =>
        {
            SetCommand {
                feature: match feature {
                    "effect" => "effect",
                    "logo-effect" => "logo-effect",
                    _ => "ring-effect",
                },
                value: value.into(),
                reports: Vec::new(),
            }
        }
        (feature @ ("speed" | "logo-speed" | "ring-speed"), value)
            if speed_byte(value).is_some() =>
        {
            SetCommand {
                feature: match feature {
                    "speed" => "speed",
                    "logo-speed" => "logo-speed",
                    _ => "ring-speed",
                },
                value: value.into(),
                reports: Vec::new(),
            }
        }
        (feature, value) if segment_feature(feature).is_some() && color_rgb(value).is_some() => {
            SetCommand {
                feature: segment_feature(feature)
                    .expect("validated segment feature")
                    .0,
                value: canonical_color(value).expect("validated RGB color"),
                reports: Vec::new(),
            }
        }
        ("sidetone", "off") => SetCommand {
            feature: "sidetone",
            value: "off".into(),
            reports: vec![vec![0x5d, 0x00]],
        },
        ("sidetone", "low") => SetCommand {
            feature: "sidetone",
            value: "low".into(),
            reports: vec![vec![0x5d, 0x01]],
        },
        ("sidetone", "medium") => SetCommand {
            feature: "sidetone",
            value: "medium".into(),
            reports: vec![vec![0x5d, 0x02]],
        },
        ("sidetone", "high") => SetCommand {
            feature: "sidetone",
            value: "high".into(),
            reports: vec![vec![0x5d, 0x03]],
        },
        _ => return Err(format!("unsupported control: {feature} {value}")),
    };
    Ok(command)
}

fn set_control(command: SetCommand) -> Result<bool, String> {
    let Some(node) = quantum_hidraw_node()? else {
        return Ok(false);
    };
    let is_profile_control = matches!(
        command.feature,
        "color"
            | "logo-color"
            | "ring-color"
            | "effect"
            | "logo-effect"
            | "ring-effect"
            | "speed"
            | "logo-speed"
            | "ring-speed"
    ) || segment_feature(command.feature).is_some();
    let mut resolved_profile = None;
    let reports = if is_profile_control {
        let cached = state::load().unwrap_or_default();
        let (mut logo, mut ring) = if command.feature == "color" {
            let initial = LightingZone {
                colors: vec![command.value.clone(); 5],
                effect: "solid".into(),
                speed: "0.5".into(),
            };
            (initial.clone(), initial)
        } else {
            let legacy = cached.lighting_color.clone();
            (
                cached_zone(
                    cached.logo_colors,
                    cached.logo_color.or_else(|| legacy.clone()),
                    cached.logo_effect,
                    cached.logo_speed,
                    "logo",
                )?,
                cached_zone(
                    cached.ring_colors,
                    cached.ring_color.or(legacy),
                    cached.ring_effect,
                    cached.ring_speed,
                    "ring",
                )?,
            )
        };

        match command.feature {
            "color" => {
                logo.colors.fill(command.value.clone());
                ring.colors.fill(command.value.clone());
            }
            "logo-color" => logo.colors.fill(command.value.clone()),
            "ring-color" => ring.colors.fill(command.value.clone()),
            "effect" => {
                logo.effect.clone_from(&command.value);
                ring.effect.clone_from(&command.value);
            }
            "logo-effect" => logo.effect.clone_from(&command.value),
            "ring-effect" => ring.effect.clone_from(&command.value),
            "speed" => {
                logo.speed.clone_from(&command.value);
                ring.speed.clone_from(&command.value);
            }
            "logo-speed" => logo.speed.clone_from(&command.value),
            "ring-speed" => ring.speed.clone_from(&command.value),
            feature => {
                let (_, segment) = segment_feature(feature).ok_or("invalid segment control")?;
                if feature.starts_with("logo-") {
                    logo.colors[segment] = command.value.clone();
                } else if feature.starts_with("ring-") {
                    ring.colors[segment] = command.value.clone();
                } else {
                    logo.colors[segment] = command.value.clone();
                    ring.colors[segment] = command.value.clone();
                }
            }
        }
        let reports = lighting_reports(&logo, &ring)?;
        resolved_profile = Some((logo, ring));
        reports
    } else {
        command.reports.clone()
    };
    for report in &reports {
        set_feature_report_allowlisted(&node, report)?;
    }
    if let Some((logo, ring)) = resolved_profile {
        state::update_lighting_profile(
            &logo.colors,
            &ring.colors,
            &logo.effect,
            &ring.effect,
            &logo.speed,
            &ring.speed,
        )?;
    } else {
        state::update_control(command.feature, &command.value)?;
    }
    println!("{} set to {}", command.feature, command.value);
    Ok(true)
}

fn parse_status_options(args: impl Iterator<Item = String>) -> Result<StatusOptions, String> {
    let mut options = StatusOptions::default();
    let mut args = args.peekable();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--dry-run" if !options.dry_run => options.dry_run = true,
            "--format" if !options.json => match args.next().as_deref() {
                Some("json") => options.json = true,
                Some(value) => return Err(format!("unsupported status format: {value}")),
                None => return Err("status --format requires a value".into()),
            },
            "--dry-run" | "--format" => {
                return Err(format!("duplicate status option: {argument}"));
            }
            _ => return Err(format!("unknown status option: {argument}")),
        }
    }
    Ok(options)
}

fn print_status_json(output: &StatusOutput) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(output).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn cached_status_json() -> Result<(), String> {
    let cached = state::load().ok_or(
        "runtime state is unavailable; start jambalinux-soniccore.service or use status for a direct read",
    )?;
    print_status_json(&StatusOutput {
        schema: 1,
        device: "0ecb:2069",
        hidraw: "runtime-cache".into(),
        access: "runtime-cache-read-only",
        dry_run: false,
        battery_percent: cached.battery_percent,
        charging: cached.charging,
        raw_feature: None,
        headset_connected: cached.headset_connected,
        ambient_mode: cached.ambient_mode,
        microphone: cached.microphone,
        lighting_enabled: cached.lighting_enabled,
        lighting_color: cached.lighting_color,
        logo_color: cached.logo_color,
        ring_color: cached.ring_color,
        logo_colors: cached.logo_colors,
        ring_colors: cached.ring_colors,
        logo_effect: cached.logo_effect,
        ring_effect: cached.ring_effect,
        logo_speed: cached.logo_speed,
        ring_speed: cached.ring_speed,
        game_chat_value: cached.game_chat_value,
        bluetooth: cached.bluetooth,
        sidetone_level: cached.sidetone_level,
    })
}

fn cached_status(mut args: impl Iterator<Item = String>) -> Result<bool, String> {
    match (args.next().as_deref(), args.next()) {
        (Some("--format"), Some(format)) if format == "json" => {
            if args.next().is_some() {
                return Err("cached-status --format accepts only `json`".into());
            }
            cached_status_json()?;
            Ok(true)
        }
        _ => Err("usage: soniccore cached-status --format json".into()),
    }
}

fn query_battery(node: &Path) -> Result<(u8, String), String> {
    let report =
        read_feature_report_read_only(node, BATTERY_FEATURE_REPORT_ID, BATTERY_FEATURE_REPORT_LEN)?;
    let battery = hid::battery_from_feature(&report)?;
    Ok((battery, format_hid_report(&report)))
}

fn probe_status() -> Result<bool, String> {
    let Some(node) = quantum_hidraw_node()? else {
        return Ok(false);
    };
    println!("matched JBL Quantum 810 HID node: {}", node.display());
    println!("access mode: read-only allowlisted HID GET_FEATURE probe");
    for &(report_id, length) in STATUS_PROBE_REPORTS {
        let report = read_feature_report_read_only(&node, report_id, length)?;
        println!("0x{report_id:02x}: {}", format_hid_report(&report));
    }
    Ok(true)
}

fn status(options: StatusOptions) -> Result<bool, String> {
    let Some(node) = quantum_hidraw_node()? else {
        return Ok(false);
    };
    if !options.json {
        println!("matched JBL Quantum 810 HID node: {}", node.display());
        println!("access mode: read-only HID GET_FEATURE (no SET_FEATURE/output reports)");
    }
    if options.dry_run {
        let metadata =
            fs::metadata(&node).map_err(|error| format!("{}: {error}", node.display()))?;
        if !metadata.file_type().is_char_device() {
            return Err(format!("refusing non-character device: {}", node.display()));
        }
        if options.json {
            print_status_json(&StatusOutput {
                schema: 1,
                device: "0ecb:2069",
                hidraw: node.display().to_string(),
                access: "hid-get-feature-read-only",
                dry_run: true,
                battery_percent: None,
                charging: None,
                raw_feature: None,
                headset_connected: None,
                ambient_mode: None,
                microphone: None,
                lighting_enabled: None,
                lighting_color: None,
                logo_color: None,
                ring_color: None,
                logo_colors: None,
                ring_colors: None,
                logo_effect: None,
                ring_effect: None,
                logo_speed: None,
                ring_speed: None,
                game_chat_value: None,
                bluetooth: None,
                sidetone_level: None,
            })?;
        } else {
            println!(
                "dry run: would read Feature Report 0x{BATTERY_FEATURE_REPORT_ID:02x} ({BATTERY_FEATURE_REPORT_LEN} bytes); no device opened"
            );
        }
        return Ok(true);
    }
    let (battery, raw_feature) = query_battery(&node)?;
    let cached = state::load().unwrap_or_default();
    let charging = match cached.charging {
        Some(charging) => charging,
        None => charging_usb_connected().map_err(|error| error.to_string())?,
    };
    if options.json {
        print_status_json(&StatusOutput {
            schema: 1,
            device: "0ecb:2069",
            hidraw: node.display().to_string(),
            access: "hid-get-feature-read-only",
            dry_run: false,
            battery_percent: Some(battery),
            charging: Some(charging),
            raw_feature: Some(raw_feature),
            headset_connected: cached.headset_connected,
            ambient_mode: cached.ambient_mode,
            microphone: cached.microphone,
            lighting_enabled: cached.lighting_enabled,
            lighting_color: cached.lighting_color,
            logo_color: cached.logo_color,
            ring_color: cached.ring_color,
            logo_colors: cached.logo_colors,
            ring_colors: cached.ring_colors,
            logo_effect: cached.logo_effect,
            ring_effect: cached.ring_effect,
            logo_speed: cached.logo_speed,
            ring_speed: cached.ring_speed,
            game_chat_value: cached.game_chat_value,
            bluetooth: cached.bluetooth,
            sidetone_level: cached.sidetone_level,
        })?;
    } else {
        println!("battery: {battery}%");
        println!("charging: {}", if charging { "yes" } else { "no" });
        println!("raw feature: {raw_feature}");
    }
    Ok(true)
}

fn notify(dry_run: bool) -> Result<bool, String> {
    let Some(node) = quantum_hidraw_node()? else {
        return Ok(false);
    };
    if dry_run {
        println!(
            "dry run: would read Feature Report 0x{BATTERY_FEATURE_REPORT_ID:02x} and show a KDE notification; no device opened"
        );
        return Ok(true);
    }
    let (battery, _) = query_battery(&node)?;
    let urgency = if battery <= 20 { "critical" } else { "normal" };
    let process = Command::new("notify-send")
        .args([
            "--app-name=JamBaLinux SonicCore",
            "--icon=audio-headphones",
            &format!("--urgency={urgency}"),
            "JBL Quantum 810",
            &format!("Bateria: {battery}%"),
        ])
        .status()
        .map_err(|error| format!("could not start notify-send: {error}"))?;
    if !process.success() {
        return Err(format!("notify-send exited with {process}"));
    }
    println!("notification sent: battery {battery}%");
    Ok(true)
}

fn show(dry_run: bool) -> Result<bool, String> {
    let Some(node) = quantum_hidraw_node()? else {
        return Ok(false);
    };
    if dry_run {
        println!(
            "dry run: would read Feature Report 0x{BATTERY_FEATURE_REPORT_ID:02x} and open a KDE dialog; no device opened"
        );
        return Ok(true);
    }
    let (battery, _) = query_battery(&node)?;
    let process = Command::new("kdialog")
        .args([
            "--title",
            "JamBaLinux SonicCore",
            "--msgbox",
            &format!("JBL Quantum 810 Wireless\nBateria: {battery}%"),
        ])
        .status()
        .map_err(|error| format!("could not start kdialog: {error}"))?;
    if !process.success() {
        return Err(format!("kdialog exited with {process}"));
    }
    Ok(true)
}

fn usage() {
    eprintln!("JamBaLinux SonicCore — gaming headset control for Linux");
    eprintln!(
        "usage: soniccore <scan|inspect|hid-descriptor|monitor [--dry-run]|daemon|status [--dry-run] [--format json]|cached-status --format json|probe-status|set <feature> <value>|equalizer [--format json|status [--format json]|set <hz> <db>|preset <id>|reset]|spatial [--format json|status [--format json]|preflight [--format json]|mode <off|binaural-stereo>|enable|disable]|notify [--dry-run]|show [--dry-run]|export --format json>"
    );
    eprintln!(
        "spatial is an experimental, disabled-by-default open binaural foundation; it applies no audio processing"
    );
    eprintln!(
        "set permits only confirmed controls and complete lighting profiles from the built-in allowlist"
    );
}

fn print_equalizer(profile: &equalizer::EqualizerProfile, json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(profile).map_err(|error| error.to_string())?
        );
    } else {
        println!("Host-side equalizer profile (dB):");
        for band in &profile.bands {
            let frequency = if band.frequency_hz >= 1_000 {
                format!("{} kHz", band.frequency_hz / 1_000)
            } else {
                format!("{} Hz", band.frequency_hz)
            };
            println!("  {frequency:>6}: {:+.1}", band.gain_db);
        }
        println!(
            "Preset: {}",
            equalizer::matching_preset(profile)
                .map(|preset| preset.name)
                .unwrap_or("Custom")
        );
    }
    Ok(())
}

#[derive(Serialize)]
struct EqualizerStatusOutput {
    schema: u8,
    bands: Vec<equalizer::Band>,
    preset: Option<&'static str>,
    pipewire: pipewire::Status,
}

fn print_equalizer_status(json: bool) -> Result<(), String> {
    let profile = equalizer::load()?;
    let pipewire = pipewire::status(&profile)?;
    let preset = equalizer::matching_preset(&profile).map(|preset| preset.id);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&EqualizerStatusOutput {
                schema: profile.schema,
                bands: profile.bands,
                preset,
                pipewire,
            })
            .map_err(|error| error.to_string())?
        );
    } else {
        print_equalizer(&profile, false)?;
        println!(
            "PipeWire: {}{}",
            if pipewire.active {
                "active"
            } else {
                "inactive"
            },
            pipewire
                .target_node_name
                .as_deref()
                .map(|target| format!(" (Game target: {target})"))
                .unwrap_or_default()
        );
        println!(
            "Routing: proven Game streams are routed automatically to {}",
            pipewire.virtual_sink_name
        );
        if let Some(error) = pipewire.error.as_deref() {
            println!("Action required: {error}");
        }
    }
    Ok(())
}

fn commit_equalizer_profile(
    previous: &equalizer::EqualizerProfile,
    requested: &equalizer::EqualizerProfile,
) -> Result<(), String> {
    pipewire::apply_profile(requested, previous)?;
    if let Err(error) = equalizer::save(requested) {
        let rollback = pipewire::apply_profile(previous, requested)
            .map(|_| "live DSP rolled back".to_owned())
            .unwrap_or_else(|rollback| format!("DSP rollback also failed: {rollback}"));
        return Err(format!(
            "could not persist the equalizer profile ({error}); {rollback}"
        ));
    }
    Ok(())
}

fn equalizer(mut args: impl Iterator<Item = String>) -> Result<bool, String> {
    match args.next().as_deref() {
        None => {
            let profile = equalizer::load()?;
            print_equalizer(&profile, false)?;
            Ok(true)
        }
        Some("--format") => match args.next().as_deref() {
            Some("json") if args.next().is_none() => {
                let profile = equalizer::load()?;
                print_equalizer(&profile, true)?;
                Ok(true)
            }
            _ => Err("equalizer --format requires exactly `json`".into()),
        },
        Some("status") => match args.next().as_deref() {
            None => {
                print_equalizer_status(false)?;
                Ok(true)
            }
            Some("--format") if args.next().as_deref() == Some("json") && args.next().is_none() => {
                print_equalizer_status(true)?;
                Ok(true)
            }
            _ => Err("equalizer status accepts only --format json".into()),
        },
        Some("enable") if args.next().is_none() => {
            let profile = equalizer::load()?;
            let status = pipewire::enable(&profile)?;
            println!(
                "automatic PipeWire Game equalizer active at {}",
                status.virtual_sink_name
            );
            Ok(true)
        }
        Some("enable") => Err("equalizer enable accepts no value".into()),
        Some("disable") if args.next().is_none() => {
            pipewire::disable()?;
            println!("PipeWire Game equalizer disabled");
            Ok(true)
        }
        Some("disable") => Err("equalizer disable accepts no value".into()),
        Some("prepare") if args.next().is_none() => {
            pipewire::prepare(&equalizer::load()?)?;
            Ok(true)
        }
        Some("prepare") => Err("equalizer prepare accepts no value".into()),
        Some("run") if args.next().is_none() => {
            pipewire::run(&equalizer::load()?)?;
            Ok(true)
        }
        Some("run") => Err("equalizer run accepts no value".into()),
        Some("restore") if args.next().is_none() => {
            pipewire::restore_routes()?;
            Ok(true)
        }
        Some("restore") => Err("equalizer restore accepts no value".into()),
        Some("set") => {
            let frequency_hz = args
                .next()
                .ok_or("equalizer set requires a frequency in Hz")?
                .parse::<u32>()
                .map_err(|_| "equalizer frequency must be an integer in Hz")?;
            let gain_db = args
                .next()
                .ok_or("equalizer set requires a gain in dB")?
                .parse::<f32>()
                .map_err(|_| "equalizer gain must be a number in dB")?;
            if args.next().is_some() {
                return Err("equalizer set accepts only frequency and gain".into());
            }
            let _mutation_lock = equalizer::mutation_lock()?;
            let previous = equalizer::load()?;
            let requested = equalizer::updated_profile(&previous, frequency_hz, gain_db)?;
            commit_equalizer_profile(&previous, &requested)?;
            print_equalizer(&requested, false)?;
            Ok(true)
        }
        Some("preset") => {
            let id = args.next().ok_or("equalizer preset requires a preset id")?;
            if args.next().is_some() {
                return Err("equalizer preset accepts exactly one preset id".into());
            }
            let requested = equalizer::preset_profile(&id)?;
            let _mutation_lock = equalizer::mutation_lock()?;
            let previous = equalizer::load()?;
            commit_equalizer_profile(&previous, &requested)?;
            print_equalizer(&requested, false)?;
            Ok(true)
        }
        Some("reset") if args.next().is_none() => {
            let _mutation_lock = equalizer::mutation_lock()?;
            let previous = equalizer::load()?;
            let requested = equalizer::reset_profile();
            commit_equalizer_profile(&previous, &requested)?;
            print_equalizer(&requested, false)?;
            Ok(true)
        }
        Some("reset") => Err("equalizer reset accepts no value".into()),
        _ => Err("unknown equalizer command".into()),
    }
}

fn print_spatial(profile: &spatial::SpatialProfile, json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(profile).map_err(|error| error.to_string())?
        );
    } else {
        println!("Experimental spatial/binaural audio:");
        println!("  enabled: {}", profile.enabled);
        println!("  mode: {}", profile.mode.as_str());
    }
    Ok(())
}

fn print_spatial_preflight(json: bool) -> Result<(), String> {
    let capability = spatial::preflight()?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&capability).map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "pipewire binary present: {}",
            capability.pipewire_binary_present
        );
        println!(
            "filter-chain module present: {}",
            capability.filter_chain_module_present
        );
        println!(
            "open HRTF/binaural dataset present: {}",
            capability.hrtf_dataset_present
        );
        println!("ready: {}", capability.ready);
        if let Some(error) = capability.error.as_deref() {
            println!("Action required: {error}");
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct SpatialStatusOutput {
    schema: u8,
    mode: spatial::SpatialMode,
    experimental: bool,
    #[serde(flatten)]
    health: pipewire::SpatialStatus,
    capability: spatial::CapabilityReport,
}

fn print_spatial_status(json: bool) -> Result<(), String> {
    let profile = spatial::load()?;
    let capability = spatial::preflight()?;
    let health = pipewire::spatial_status(&profile, &capability);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&SpatialStatusOutput {
                schema: profile.schema,
                mode: profile.mode,
                experimental: true,
                health: health.clone(),
                capability,
            })
            .map_err(|error| error.to_string())?
        );
    } else {
        print_spatial(&profile, false)?;
        println!("configured: {}", health.configured);
        println!("active: {}", health.active);
        println!("service active: {}", health.service_active);
        println!("dataset valid: {}", health.dataset_valid);
        println!(
            "target equalizer connected: {}",
            health.target_equalizer_connected
        );
        println!("routing healthy: {}", health.routing_healthy);
        if let Some(error) = health.error.as_deref() {
            println!("Action required: {error}");
        }
    }
    Ok(())
}

fn spatial(mut args: impl Iterator<Item = String>) -> Result<bool, String> {
    match args.next().as_deref() {
        None => {
            print_spatial(&spatial::load()?, false)?;
            Ok(true)
        }
        Some("--format") => match args.next().as_deref() {
            Some("json") if args.next().is_none() => {
                print_spatial(&spatial::load()?, true)?;
                Ok(true)
            }
            _ => Err("spatial --format requires exactly `json`".into()),
        },
        Some("status") => match args.next().as_deref() {
            None => {
                print_spatial_status(false)?;
                Ok(true)
            }
            Some("--format") if args.next().as_deref() == Some("json") && args.next().is_none() => {
                print_spatial_status(true)?;
                Ok(true)
            }
            _ => Err("spatial status accepts only --format json".into()),
        },
        Some("preflight") => match args.next().as_deref() {
            None => {
                print_spatial_preflight(false)?;
                Ok(true)
            }
            Some("--format") if args.next().as_deref() == Some("json") && args.next().is_none() => {
                print_spatial_preflight(true)?;
                Ok(true)
            }
            _ => Err("spatial preflight accepts only --format json".into()),
        },
        Some("mode") => {
            let value = args.next().ok_or("spatial mode requires a value")?;
            if args.next().is_some() {
                return Err("spatial mode accepts only one value".into());
            }
            let mode = spatial::SpatialMode::parse(&value)?;
            let profile = spatial::set_mode(mode)?;
            print_spatial(&profile, false)?;
            Ok(true)
        }
        Some("enable") if args.next().is_none() => {
            let profile = spatial::set_enabled(true)?;
            println!(
                "spatial gate enabled (mode: {}); the persistent supervisor will create the \
                 graph only while the equalizer target and preflight remain valid",
                profile.mode.as_str()
            );
            Ok(true)
        }
        Some("enable") => Err("spatial enable accepts no value".into()),
        Some("disable") if args.next().is_none() => {
            spatial::set_enabled(false)?;
            println!("spatial gate disabled");
            Ok(true)
        }
        Some("disable") => Err("spatial disable accepts no value".into()),
        Some("run") if args.next().is_none() => {
            spatial_pipewire::run()?;
            Ok(true)
        }
        Some("run") => Err("spatial run accepts no value".into()),
        Some("restore") if args.next().is_none() => {
            spatial_pipewire::restore_streams()?;
            Ok(true)
        }
        Some("restore") => Err("spatial restore accepts no value".into()),
        _ => Err("unknown spatial command".into()),
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "scan".into());
    let result = match command.as_str() {
        "scan" => scan().map_err(|error| error.to_string()),
        "inspect" => inspect(),
        "hid-descriptor" => hid_descriptor(),
        "monitor" => monitor(args.next().as_deref() == Some("--dry-run")),
        "daemon" => daemon(),
        "status" => parse_status_options(args).and_then(status),
        "cached-status" => cached_status(args),
        "probe-status" => probe_status(),
        "set" => parse_set_command(args).and_then(set_control),
        "equalizer" | "eq" => equalizer(args),
        "spatial" => spatial(args),
        "notify" => notify(args.next().as_deref() == Some("--dry-run")),
        "show" => show(args.next().as_deref() == Some("--dry-run")),
        "export"
            if args.next().as_deref() == Some("--format")
                && args.next().as_deref() == Some("json") =>
        {
            export_json()
        }
        "help" | "--help" | "-h" => {
            usage();
            return;
        }
        _ => {
            usage();
            std::process::exit(2);
        }
    };
    match result {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("JBL Quantum 810 dongle (0ecb:2069) not found");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("inspection failed: {error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_interface_and_endpoint() {
        let bytes = [9, 4, 5, 0, 1, 3, 0, 0, 3, 7, 5, 0x83, 0x03, 0x40, 0x00, 3];
        assert_eq!(
            parse_descriptors(&bytes).unwrap(),
            vec![Interface {
                number: 5,
                alternate_setting: 0,
                class: 3,
                subclass: 0,
                protocol: 0,
                endpoints: vec![Endpoint {
                    address: 0x83,
                    attributes: 3,
                    max_packet_size: 64,
                    interval: 3
                }],
            }]
        );
    }

    #[test]
    fn rejects_invalid_descriptor_length() {
        assert!(parse_descriptors(&[1, 4]).is_err());
        assert!(parse_descriptors(&[9, 4, 0]).is_err());
    }

    #[test]
    fn endpoint_helpers_decode_direction_and_type() {
        let endpoint = Endpoint {
            address: 0x81,
            attributes: 1,
            max_packet_size: 512,
            interval: 4,
        };
        assert_eq!(endpoint.direction(), "IN");
        assert_eq!(endpoint.transfer_type(), "isochronous");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn builds_linux_get_feature_ioctl_number() {
        assert_eq!(hidiocgfeature(2), 0xc002_4807);
        assert_eq!(hidiocsfeature(2), 0xc002_4806);
    }

    #[test]
    fn set_commands_are_strictly_allowlisted() {
        let parse = |items: &[&str]| parse_set_command(items.iter().map(|item| (*item).into()));
        assert_eq!(
            parse(&["ambient", "anc"]).unwrap().reports,
            vec![vec![0x46, 0x01]]
        );
        assert_eq!(
            parse(&["lighting", "off"]).unwrap().reports,
            vec![vec![0x4b, 0x00]]
        );
        assert_eq!(
            parse(&["sidetone", "high"]).unwrap().reports,
            vec![vec![0x5d, 0x03]]
        );
        let color = parse(&["color", "cyan"]).unwrap();
        assert_eq!(color.reports.len(), 13);
        assert_eq!(color.reports[0], vec![0x4c, 0x00, 0x64, 0x05]);
        assert_eq!(
            color.reports[1],
            vec![0x4d, 0x00, 0x00, 0x33, 0xff, 0xcc, 0x01, 0x00]
        );
        assert_eq!(color.reports[12], vec![0x4b, 0x01]);
        let custom = parse(&["color", "#112233"]).unwrap();
        assert_eq!(custom.value, "#112233");
        assert_eq!(
            custom.reports[1],
            vec![0x4d, 0x00, 0x00, 0x11, 0x22, 0x33, 0x01, 0x00]
        );
        assert!(parse(&["logo-color", "blue"]).unwrap().reports.is_empty());
        assert!(
            parse(&["ring-color", "magenta"])
                .unwrap()
                .reports
                .is_empty()
        );
        assert!(parse(&["color", "112233"]).is_err());
        assert!(parse(&["color", "#11223g"]).is_err());
        assert!(parse(&["color", "#11223344"]).is_err());
        assert_eq!(parse(&["effect", "wave"]).unwrap().feature, "effect");
        assert_eq!(parse(&["logo-speed", "1.5"]).unwrap().value, "1.5");
        assert_eq!(
            parse(&["ring-segment-4", "#abcdef"]).unwrap().feature,
            "ring-segment-4"
        );
        assert!(parse(&["effect", "rainbow"]).is_err());
        assert!(parse(&["speed", "3.0"]).is_err());
        assert!(parse(&["logo-segment-5", "#abcdef"]).is_err());
        assert!(parse(&["raw", "46ff"]).is_err());
        assert!(parse(&["ambient", "invalid"]).is_err());
        assert!(parse(&["ambient", "anc", "extra"]).is_err());
    }

    #[test]
    fn builds_complete_animated_lighting_profile() {
        let zone = LightingZone {
            colors: vec![
                "#112233".into(),
                "#223344".into(),
                "#334455".into(),
                "#445566".into(),
                "#556677".into(),
            ],
            effect: "wave".into(),
            speed: "1.5".into(),
        };
        let reports = lighting_reports(&zone, &zone).unwrap();
        assert_eq!(reports.len(), 13);
        assert_eq!(reports[0], vec![0x4c, 0x00, 0x32, 0x05]);
        assert_eq!(
            reports[1],
            vec![0x4d, 0x00, 0x00, 0x11, 0x22, 0x33, 0x02, 0x00]
        );
        assert_eq!(
            reports[5],
            vec![0x4d, 0x00, 0x04, 0x55, 0x66, 0x77, 0x02, 0x08]
        );
        assert_eq!(reports[12], vec![0x4b, 0x01]);
    }

    #[test]
    fn parses_status_options_in_any_order() {
        assert_eq!(
            parse_status_options(
                ["--format", "json", "--dry-run"]
                    .into_iter()
                    .map(String::from)
            ),
            Ok(StatusOptions {
                dry_run: true,
                json: true
            })
        );
        assert!(parse_status_options(["--format"].into_iter().map(String::from)).is_err());
        assert!(parse_status_options(["--xml"].into_iter().map(String::from)).is_err());
    }

    #[test]
    fn cached_status_requires_json_format() {
        assert!(cached_status(std::iter::empty()).is_err());
        assert!(cached_status(["--format", "text"].into_iter().map(String::from)).is_err());
        assert!(
            cached_status(["--format", "json", "extra"].into_iter().map(String::from)).is_err()
        );
    }

    #[test]
    fn serializes_versioned_status_json() {
        let output = StatusOutput {
            schema: 1,
            device: "0ecb:2069",
            hidraw: "/dev/hidraw7".into(),
            access: "hid-get-feature-read-only",
            dry_run: false,
            battery_percent: Some(60),
            charging: Some(true),
            raw_feature: Some("49 3c".into()),
            headset_connected: Some(true),
            ambient_mode: Some("anc".into()),
            microphone: Some("active".into()),
            lighting_enabled: Some(true),
            lighting_color: Some("cyan".into()),
            logo_color: Some("cyan".into()),
            ring_color: Some("cyan".into()),
            logo_colors: Some(vec!["#33ffcc".into(); 5]),
            ring_colors: Some(vec!["#33ffcc".into(); 5]),
            logo_effect: Some("solid".into()),
            ring_effect: Some("solid".into()),
            logo_speed: Some("0.5".into()),
            ring_speed: Some("0.5".into()),
            game_chat_value: Some(8),
            bluetooth: Some("connected".into()),
            sidetone_level: Some("low".into()),
        };
        let json = serde_json::to_value(output).unwrap();
        assert_eq!(json["schema"], 1);
        assert_eq!(json["battery_percent"], 60);
        assert_eq!(json["charging"], true);
        assert_eq!(json["raw_feature"], "49 3c");
    }
}

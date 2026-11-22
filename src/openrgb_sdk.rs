//! A client of OpenRGB's SDK server ("Servidor SDK", TCP 127.0.0.1:6742),
//! the official, versioned interface OpenRGB offers other programs to drive
//! the lights of every device it supports (memory, motherboard, video card,
//! keyboards, fans...).
//!
//! Only OpenRGB talks to the hardware: this client lists its devices and
//! hands it colors, so no device protocol is added here. It speaks protocol
//! version 4 (OpenRGB 0.9) at most, which every newer server also accepts:
//! devices are addressed by index and no acknowledgement packets are sent.
//! See OpenRGB's Documentation/OpenRGBSDK.md.

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 6742;
/// The newest protocol this client parses.
pub const CLIENT_PROTOCOL: u32 = 4;

const MAGIC: &[u8; 4] = b"ORGB";
const HEADER_LEN: usize = 16;
/// No answer of the server is this large; anything bigger is not OpenRGB.
const MAX_PACKET: usize = 16 * 1024 * 1024;

const REQUEST_CONTROLLER_COUNT: u32 = 0;
const REQUEST_CONTROLLER_DATA: u32 = 1;
const REQUEST_PROTOCOL_VERSION: u32 = 40;
const SET_CLIENT_NAME: u32 = 50;
pub const DEVICE_LIST_UPDATED: u32 = 100;
const REQUEST_PROFILE_LIST: u32 = 150;
const REQUEST_LOAD_PROFILE: u32 = 152;
const RGBCONTROLLER_UPDATELEDS: u32 = 1050;
const RGBCONTROLLER_SETCUSTOMMODE: u32 = 1100;
const RGBCONTROLLER_UPDATEMODE: u32 = 1101;
const REQUEST_PLUGIN_LIST: u32 = 200;
const PLUGIN_SPECIFIC: u32 = 201;

/// The OpenRGB Effects plugin's own SDK commands (its OnSDKCommand).
pub const EFFECTS_PLUGIN: &str = "OpenRGB Effects Plugin";
const EFFECTS_REQUEST_LIST: u32 = 0;
const EFFECTS_START: u32 = 20;
const EFFECTS_STOP: u32 = 21;

/// A plugin loaded in OpenRGB, as its plugin list reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Plugin {
    pub name: String,
    pub version: String,
    pub index: u32,
    /// Its own SDK commands' version; 0 when it has none.
    pub sdk_version: u32,
}

/// One effect of the OpenRGB Effects plugin and whether it runs now.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub name: String,
    pub running: bool,
}

/// OpenRGB's device types (RGBController.h), the ones worth naming.
pub fn device_kind(kind: i32) -> &'static str {
    match kind {
        0 => "motherboard",
        1 => "memory",
        2 => "gpu",
        3 => "cooler",
        4 => "ledstrip",
        5 => "keyboard",
        6 => "mouse",
        7 => "mousemat",
        8 => "headset",
        9 => "headset-stand",
        10 => "gamepad",
        11 => "light",
        12 => "speaker",
        13 => "virtual",
        14 => "storage",
        15 => "case",
        16 => "microphone",
        17 => "accessory",
        18 => "keypad",
        19 => "laptop",
        20 => "monitor",
        _ => "other",
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    pub fn parse(text: &str) -> Option<Color> {
        let digits = text.trim().strip_prefix('#')?;
        if digits.len() != 6 || !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
        Some(Color {
            r: channel(0)?,
            g: channel(2)?,
            b: channel(4)?,
        })
    }

    /// OpenRGB's RGBColor: 0x00BBGGRR, sent little-endian.
    fn wire(self) -> [u8; 4] {
        [self.r, self.g, self.b, 0]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Matrix {
    pub height: u32,
    pub width: u32,
    /// LED index (within the zone) per cell, row by row; 0xFFFFFFFF is empty.
    pub map: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub name: String,
    /// 0 single, 1 linear, 2 matrix.
    pub kind: i32,
    pub leds_count: u32,
    pub matrix: Option<Matrix>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mode {
    pub name: String,
    /// 1: one color per LED (a direct mode, or a static one set LED by LED).
    pub color_mode: u32,
    /// The mode's block as the server sent it, to send back in UpdateMode.
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Controller {
    /// Its index in the server's list (how protocols up to 5 address it).
    pub index: u32,
    pub kind: i32,
    pub name: String,
    pub vendor: String,
    pub description: String,
    pub location: String,
    pub serial: String,
    pub modes: Vec<Mode>,
    pub active_mode: i32,
    pub zones: Vec<Zone>,
    pub led_count: usize,
    /// The colors it shows now.
    pub colors: Vec<Color>,
}

impl Controller {
    /// A stable name for it across restarts (the index is not): what the
    /// user's choices are remembered by.
    pub fn key(&self) -> String {
        format!("{}|{}|{}", self.name, self.location, self.serial)
    }
}

/// Bounds-checked reader of the server's little-endian blocks.
struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(length)
            .filter(|&end| end <= self.data.len())
            .ok_or("OpenRGB sent a truncated device description")?;
        let bytes = &self.data[self.at..end];
        self.at = end;
        Ok(bytes)
    }

    fn u16(&mut self) -> Result<u16, String> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }

    /// A length-prefixed string, null terminator included in the length.
    fn string(&mut self) -> Result<String, String> {
        let length = usize::from(self.u16()?);
        let bytes = self.take(length)?;
        let text = bytes.split(|&byte| byte == 0).next().unwrap_or_default();
        Ok(String::from_utf8_lossy(text).into_owned())
    }

    fn color(&mut self) -> Result<Color, String> {
        let bytes = self.take(4)?;
        Ok(Color {
            r: bytes[0],
            g: bytes[1],
            b: bytes[2],
        })
    }
}

fn parse_mode(cursor: &mut Cursor, protocol: u32) -> Result<Mode, String> {
    let start = cursor.at;
    let name = cursor.string()?;
    cursor.i32()?; // value
    cursor.u32()?; // flags
    cursor.u32()?; // speed_min
    cursor.u32()?; // speed_max
    if protocol >= 3 {
        cursor.u32()?; // brightness_min
        cursor.u32()?; // brightness_max
    }
    cursor.u32()?; // colors_min
    cursor.u32()?; // colors_max
    cursor.u32()?; // speed
    if protocol >= 3 {
        cursor.u32()?; // brightness
    }
    cursor.u32()?; // direction
    let color_mode = cursor.u32()?;
    let colors = usize::from(cursor.u16()?);
    cursor.take(colors * 4)?;
    Ok(Mode {
        name,
        color_mode,
        raw: cursor.data[start..cursor.at].to_vec(),
    })
}

fn parse_zone(cursor: &mut Cursor, protocol: u32) -> Result<Zone, String> {
    let name = cursor.string()?;
    let kind = cursor.i32()?;
    cursor.u32()?; // leds_min
    cursor.u32()?; // leds_max
    let leds_count = cursor.u32()?;
    let matrix_length = usize::from(cursor.u16()?);
    let matrix = if matrix_length > 0 {
        let mut block = Cursor::new(cursor.take(matrix_length)?);
        let height = block.u32()?;
        let width = block.u32()?;
        let cells = (height as usize)
            .checked_mul(width as usize)
            .filter(|&cells| cells * 4 + 8 <= matrix_length)
            .ok_or("OpenRGB sent an invalid matrix map")?;
        let map = (0..cells).map(|_| block.u32()).collect::<Result<Vec<_>, _>>()?;
        Some(Matrix { height, width, map })
    } else {
        None
    };
    if protocol >= 4 {
        let segments = cursor.u16()?;
        for _ in 0..segments {
            cursor.string()?; // name
            cursor.i32()?; // type
            cursor.u32()?; // start_idx
            cursor.u32()?; // leds_count
        }
    }
    if protocol >= 5 {
        cursor.u32()?; // flags
    }
    Ok(Zone {
        name,
        kind,
        leds_count,
        matrix,
    })
}

/// One answer to REQUEST_CONTROLLER_DATA (its leading size included),
/// formatted for `protocol`.
pub fn parse_controller(index: u32, reply: &[u8], protocol: u32) -> Result<Controller, String> {
    let mut cursor = Cursor::new(reply);
    let size = cursor.u32()? as usize;
    if size != reply.len() {
        return Err("OpenRGB sent a device description of the wrong size".into());
    }
    let kind = cursor.i32()?;
    let name = cursor.string()?;
    let vendor = if protocol >= 1 {
        cursor.string()?
    } else {
        String::new()
    };
    let description = cursor.string()?;
    cursor.string()?; // version
    let serial = cursor.string()?;
    let location = cursor.string()?;
    let mode_count = cursor.u16()?;
    let active_mode = cursor.i32()?;
    let modes = (0..mode_count)
        .map(|_| parse_mode(&mut cursor, protocol))
        .collect::<Result<Vec<_>, _>>()?;
    let zone_count = cursor.u16()?;
    let zones = (0..zone_count)
        .map(|_| parse_zone(&mut cursor, protocol))
        .collect::<Result<Vec<_>, _>>()?;
    let led_count = usize::from(cursor.u16()?);
    for _ in 0..led_count {
        cursor.string()?; // name
        cursor.u32()?; // value
    }
    let color_count = usize::from(cursor.u16()?);
    let colors = (0..color_count)
        .map(|_| cursor.color())
        .collect::<Result<Vec<_>, _>>()?;
    // Up to protocol 4 the colors end the block: anything left means the
    // fields were not where this protocol puts them.
    if protocol <= CLIENT_PROTOCOL && cursor.at != reply.len() {
        return Err("OpenRGB sent a device description this client cannot read".into());
    }
    Ok(Controller {
        index,
        kind,
        name,
        vendor,
        description,
        location,
        serial,
        modes,
        active_mode,
        zones,
        led_count,
        colors,
    })
}

/// The answer to REQUEST_PLUGIN_LIST.
pub fn parse_plugins(reply: &[u8]) -> Result<Vec<Plugin>, String> {
    let mut cursor = Cursor::new(reply);
    cursor.u32()?; // data_size
    let count = cursor.u16()?;
    (0..count)
        .map(|_| {
            let name = cursor.string()?;
            cursor.string()?; // description
            let version = cursor.string()?;
            Ok(Plugin {
                name,
                version,
                index: cursor.u32()?,
                sdk_version: cursor.u32()?,
            })
        })
        .collect()
}

/// The Effects plugin's answer to its effect list request (after the
/// plugin packet id).
pub fn parse_effects(reply: &[u8]) -> Result<Vec<Effect>, String> {
    let mut cursor = Cursor::new(reply);
    cursor.u32()?; // data_size
    let count = cursor.u16()?;
    (0..count)
        .map(|_| {
            let name = cursor.string()?;
            cursor.string()?; // description
            Ok(Effect {
                name,
                running: cursor.take(1)?[0] != 0,
            })
        })
        .collect()
}

/// The answer to REQUEST_PROFILE_LIST.
pub fn parse_profiles(reply: &[u8]) -> Result<Vec<String>, String> {
    let mut cursor = Cursor::new(reply);
    cursor.u32()?; // data_size
    let count = cursor.u16()?;
    (0..count).map(|_| cursor.string()).collect()
}

/// UpdateLEDs: every LED's color, in the device's LED order.
pub fn update_leds_payload(colors: &[Color]) -> Vec<u8> {
    let count = colors.len().min(usize::from(u16::MAX));
    let size = 4 + 2 + 4 * count;
    let mut payload = Vec::with_capacity(size);
    payload.extend_from_slice(&(size as u32).to_le_bytes());
    payload.extend_from_slice(&(count as u16).to_le_bytes());
    for color in &colors[..count] {
        payload.extend_from_slice(&color.wire());
    }
    payload
}

fn update_mode_payload(mode_index: i32, mode: &Mode) -> Vec<u8> {
    let size = 4 + 4 + mode.raw.len();
    let mut payload = Vec::with_capacity(size);
    payload.extend_from_slice(&(size as u32).to_le_bytes());
    payload.extend_from_slice(&mode_index.to_le_bytes());
    payload.extend_from_slice(&mode.raw);
    payload
}

fn header(device: u32, id: u32, size: usize) -> [u8; HEADER_LEN] {
    let mut bytes = [0; HEADER_LEN];
    bytes[..4].copy_from_slice(MAGIC);
    bytes[4..8].copy_from_slice(&device.to_le_bytes());
    bytes[8..12].copy_from_slice(&id.to_le_bytes());
    bytes[12..16].copy_from_slice(&(size as u32).to_le_bytes());
    bytes
}

/// A packet taken from the stream: device, id and data.
struct Packet {
    device: u32,
    id: u32,
    data: Vec<u8>,
}

/// The first packet complete in `inbox` and how many bytes it used.
fn next_packet(inbox: &[u8]) -> Result<Option<(Packet, usize)>, String> {
    if inbox.len() < HEADER_LEN {
        return Ok(None);
    }
    if &inbox[..4] != MAGIC {
        return Err("the SDK server did not answer like OpenRGB".into());
    }
    let word = |at: usize| u32::from_le_bytes([inbox[at], inbox[at + 1], inbox[at + 2], inbox[at + 3]]);
    let (device, id, size) = (word(4), word(8), word(12) as usize);
    if size > MAX_PACKET {
        return Err("the SDK server sent an oversized packet".into());
    }
    if inbox.len() < HEADER_LEN + size {
        return Ok(None);
    }
    Ok(Some((
        Packet {
            device,
            id,
            data: inbox[HEADER_LEN..HEADER_LEN + size].to_vec(),
        },
        HEADER_LEN + size,
    )))
}

pub struct Client {
    stream: TcpStream,
    protocol: u32,
    inbox: Vec<u8>,
    /// The server announced a new device list: read it again.
    pub devices_changed: bool,
}

impl Client {
    /// Connects to the SDK server on this computer and agrees on a protocol.
    pub fn connect(port: u16, name: &str) -> Result<Self, String> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let stream = TcpStream::connect_timeout(&address, Duration::from_millis(800))
            .map_err(|error| match error.kind() {
                ErrorKind::ConnectionRefused | ErrorKind::TimedOut => {
                    format!("the OpenRGB SDK server is not running on port {port}")
                }
                _ => format!("could not reach the OpenRGB SDK server: {error}"),
            })?;
        stream.set_nodelay(true).ok();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
        let mut client = Self {
            stream,
            protocol: 0,
            inbox: Vec::new(),
            devices_changed: false,
        };
        client.send(0, REQUEST_PROTOCOL_VERSION, &CLIENT_PROTOCOL.to_le_bytes())?;
        // A protocol 0 server does not answer this request at all.
        client.protocol = match client.receive(REQUEST_PROTOCOL_VERSION, Duration::from_secs(1)) {
            Ok((_, data)) if data.len() >= 4 => {
                u32::from_le_bytes([data[0], data[1], data[2], data[3]]).min(CLIENT_PROTOCOL)
            }
            Ok(_) => return Err("the SDK server sent an invalid protocol version".into()),
            Err(_) => 0,
        };
        let mut client_name = name.as_bytes().to_vec();
        client_name.push(0);
        client.send(0, SET_CLIENT_NAME, &client_name)?;
        Ok(client)
    }

    pub fn protocol(&self) -> u32 {
        self.protocol
    }

    fn send(&mut self, device: u32, id: u32, payload: &[u8]) -> Result<(), String> {
        let mut packet = Vec::with_capacity(HEADER_LEN + payload.len());
        packet.extend_from_slice(&header(device, id, payload.len()));
        packet.extend_from_slice(payload);
        self.stream
            .write_all(&packet)
            .map_err(|error| format!("lost the OpenRGB SDK server: {error}"))
    }

    /// Reads what the server sent; `wait` blocks up to that long for data.
    fn fill(&mut self, wait: Option<Duration>) -> Result<bool, String> {
        let mut buffer = [0u8; 65536];
        match wait {
            Some(wait) => {
                self.stream.set_nonblocking(false).ok();
                self.stream
                    .set_read_timeout(Some(wait.max(Duration::from_millis(1))))
                    .map_err(|error| error.to_string())?;
            }
            None => {
                self.stream.set_nonblocking(true).ok();
            }
        }
        let result = self.stream.read(&mut buffer);
        self.stream.set_nonblocking(false).ok();
        match result {
            Ok(0) => Err("the OpenRGB SDK server closed the connection".into()),
            Ok(count) => {
                self.inbox.extend_from_slice(&buffer[..count]);
                Ok(true)
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                Ok(false)
            }
            Err(error) => Err(format!("lost the OpenRGB SDK server: {error}")),
        }
    }

    /// Takes the complete packets out of the inbox; notices are noted, and
    /// the first packet with `id` (if any) is returned.
    fn drain(&mut self, id: Option<u32>) -> Result<Option<(u32, Vec<u8>)>, String> {
        while let Some((packet, used)) = next_packet(&self.inbox)? {
            self.inbox.drain(..used);
            if packet.id == DEVICE_LIST_UPDATED {
                self.devices_changed = true;
                continue;
            }
            if Some(packet.id) == id {
                return Ok(Some((packet.device, packet.data)));
            }
        }
        Ok(None)
    }

    fn receive(&mut self, id: u32, timeout: Duration) -> Result<(u32, Vec<u8>), String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(found) = self.drain(Some(id))? {
                return Ok(found);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("the OpenRGB SDK server did not answer".into());
            }
            self.fill(Some(left))?;
        }
    }

    /// Notices sent by the server since the last call (a device list
    /// change), without waiting.
    pub fn poll(&mut self) -> Result<(), String> {
        while self.fill(None)? {}
        self.drain(None).map(|_| ())
    }

    pub fn controllers(&mut self) -> Result<Vec<Controller>, String> {
        self.devices_changed = false;
        self.send(0, REQUEST_CONTROLLER_COUNT, &[])?;
        let (_, data) = self.receive(REQUEST_CONTROLLER_COUNT, Duration::from_secs(3))?;
        if data.len() < 4 {
            return Err("the SDK server sent an invalid device count".into());
        }
        let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]).min(1024);
        let request = if self.protocol >= 1 {
            self.protocol.to_le_bytes().to_vec()
        } else {
            Vec::new()
        };
        let mut controllers = Vec::new();
        for index in 0..count {
            self.send(index, REQUEST_CONTROLLER_DATA, &request)?;
            let (_, reply) = self.receive(REQUEST_CONTROLLER_DATA, Duration::from_secs(5))?;
            controllers.push(parse_controller(index, &reply, self.protocol)?);
        }
        Ok(controllers)
    }

    /// Puts the device in its direct ("Custom"/"Direct") mode, where it
    /// shows the colors sent to it.
    pub fn set_custom_mode(&mut self, index: u32) -> Result<(), String> {
        self.send(index, RGBCONTROLLER_SETCUSTOMMODE, &[])
    }

    pub fn update_leds(&mut self, index: u32, colors: &[Color]) -> Result<(), String> {
        self.send(index, RGBCONTROLLER_UPDATELEDS, &update_leds_payload(colors))
    }

    pub fn profiles(&mut self) -> Result<Vec<String>, String> {
        if self.protocol < 2 {
            return Ok(Vec::new());
        }
        self.send(0, REQUEST_PROFILE_LIST, &[])?;
        let (_, data) = self.receive(REQUEST_PROFILE_LIST, Duration::from_secs(3))?;
        parse_profiles(&data)
    }

    pub fn plugins(&mut self) -> Result<Vec<Plugin>, String> {
        if self.protocol < 4 {
            return Ok(Vec::new());
        }
        self.send(0, REQUEST_PLUGIN_LIST, &[])?;
        let (_, data) = self.receive(REQUEST_PLUGIN_LIST, Duration::from_secs(3))?;
        parse_plugins(&data)
    }

    /// One of a plugin's own commands; `answer` waits for its reply.
    fn plugin_command(
        &mut self,
        plugin: u32,
        command: u32,
        data: &[u8],
        answer: bool,
    ) -> Result<Vec<u8>, String> {
        let mut payload = command.to_le_bytes().to_vec();
        payload.extend_from_slice(data);
        self.send(plugin, PLUGIN_SPECIFIC, &payload)?;
        if !answer {
            return Ok(Vec::new());
        }
        let (_, reply) = self.receive(PLUGIN_SPECIFIC, Duration::from_secs(3))?;
        if reply.len() < 4 || reply[..4] != command.to_le_bytes() {
            return Err("the plugin sent an unexpected answer".into());
        }
        Ok(reply[4..].to_vec())
    }

    /// The OpenRGB Effects plugin's effects (none without the plugin).
    pub fn effects(&mut self, plugin: u32) -> Result<Vec<Effect>, String> {
        parse_effects(&self.plugin_command(plugin, EFFECTS_REQUEST_LIST, &[], true)?)
    }

    /// Starts or stops one of the Effects plugin's effects.
    pub fn set_effect(&mut self, plugin: u32, name: &str, running: bool) -> Result<(), String> {
        let mut data = ((name.len() + 1) as u16).to_le_bytes().to_vec();
        data.extend_from_slice(name.as_bytes());
        data.push(0);
        let command = if running { EFFECTS_START } else { EFFECTS_STOP };
        self.plugin_command(plugin, command, &data, false).map(|_| ())
    }

    /// Puts a device back in one of its modes, as the server describes it.
    pub fn update_mode(&mut self, controller: &Controller, mode_index: i32) -> Result<(), String> {
        let mode = usize::try_from(mode_index)
            .ok()
            .and_then(|index| controller.modes.get(index))
            .ok_or("the device has no such mode")?;
        let payload = update_mode_payload(mode_index, mode);
        self.send(controller.index, RGBCONTROLLER_UPDATEMODE, &payload)
    }

    /// Loads one of OpenRGB's saved profiles. The server only updates its
    /// records; as the protocol asks, the modes are then applied to every
    /// device from the state it reports.
    pub fn load_profile(&mut self, name: &str) -> Result<(), String> {
        if self.protocol < 2 {
            return Err("this OpenRGB is too old to load profiles".into());
        }
        let mut payload = name.as_bytes().to_vec();
        payload.push(0);
        self.send(0, REQUEST_LOAD_PROFILE, &payload)?;
        for controller in self.controllers()? {
            let Ok(active) = usize::try_from(controller.active_mode) else {
                continue;
            };
            if let Some(mode) = controller.modes.get(active) {
                let payload = update_mode_payload(controller.active_mode, mode);
                self.send(controller.index, RGBCONTROLLER_UPDATEMODE, &payload)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Writes a device description the way OpenRGB's server does
    /// (RGBController::GetDeviceDescriptionData), for one protocol version.
    pub(crate) struct DescriptionWriter {
        bytes: Vec<u8>,
    }

    /// A zone to describe: name, type, LED count and matrix (height, width).
    pub(crate) type ZoneSpec<'a> = (&'a str, i32, u32, Option<(u32, u32)>);

    impl DescriptionWriter {
        fn string(&mut self, text: &str) {
            self.bytes
                .extend_from_slice(&((text.len() + 1) as u16).to_le_bytes());
            self.bytes.extend_from_slice(text.as_bytes());
            self.bytes.push(0);
        }

        fn u32(&mut self, value: u32) {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }

        fn u16(&mut self, value: u16) {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }

        pub(crate) fn device(
            protocol: u32,
            kind: i32,
            name: &str,
            zones: &[ZoneSpec],
        ) -> Vec<u8> {
            let mut writer = Self { bytes: Vec::new() };
            writer.u32(kind as u32);
            writer.string(name);
            if protocol >= 1 {
                writer.string("Vendor");
            }
            writer.string("Description");
            writer.string("1.0");
            writer.string("SERIAL");
            writer.string(&format!("I2C: /dev/i2c-0, address 0x{kind:02x}"));
            writer.u16(2);
            writer.u32(1);
            for (mode, colors) in [("Static", 1u16), ("Direct", 0)] {
                writer.string(mode);
                writer.u32(7); // value
                writer.u32(0); // flags
                writer.u32(0);
                writer.u32(4);
                if protocol >= 3 {
                    writer.u32(0);
                    writer.u32(100);
                }
                writer.u32(colors.into());
                writer.u32(colors.into());
                writer.u32(2);
                if protocol >= 3 {
                    writer.u32(100);
                }
                writer.u32(0);
                writer.u32(1);
                writer.u16(colors);
                for _ in 0..colors {
                    writer.u32(0x00ff_0000);
                }
            }
            writer.u16(zones.len() as u16);
            let mut leds = 0;
            for &(zone, zone_kind, count, matrix) in zones {
                writer.string(zone);
                writer.u32(zone_kind as u32);
                writer.u32(count);
                writer.u32(count);
                writer.u32(count);
                match matrix {
                    Some((height, width)) => {
                        writer.u16((8 + 4 * height * width) as u16);
                        writer.u32(height);
                        writer.u32(width);
                        for cell in 0..height * width {
                            writer.u32(if cell < count { cell } else { u32::MAX });
                        }
                    }
                    None => writer.u16(0),
                }
                if protocol >= 4 {
                    writer.u16(1);
                    writer.string("Segment");
                    writer.u32(1);
                    writer.u32(0);
                    writer.u32(count);
                }
                if protocol >= 5 {
                    writer.u32(0);
                }
                leds += count;
            }
            writer.u16(leds as u16);
            for led in 0..leds {
                writer.string(&format!("LED {led}"));
                writer.u32(led);
            }
            writer.u16(leds as u16);
            for led in 0..leds {
                writer.u32(led); // 0x00BBGGRR
            }
            let mut reply = ((writer.bytes.len() + 4) as u32).to_le_bytes().to_vec();
            reply.extend_from_slice(&writer.bytes);
            reply
        }
    }

    #[test]
    fn a_device_description_is_read_for_each_protocol_version() {
        for protocol in 0..=CLIENT_PROTOCOL {
            let reply = DescriptionWriter::device(
                protocol,
                1,
                "Corsair Dominator Platinum",
                &[("DRAM", 1, 12, None), ("Keys", 2, 6, Some((2, 4)))],
            );
            let controller = parse_controller(3, &reply, protocol).unwrap();
            assert_eq!(controller.index, 3);
            assert_eq!(controller.name, "Corsair Dominator Platinum");
            assert_eq!(controller.vendor, if protocol >= 1 { "Vendor" } else { "" });
            assert_eq!(controller.location, "I2C: /dev/i2c-0, address 0x01");
            assert_eq!(controller.modes.len(), 2);
            assert_eq!(controller.modes[1].name, "Direct");
            assert_eq!(controller.active_mode, 1);
            assert_eq!(controller.zones.len(), 2);
            assert_eq!(controller.zones[0].leds_count, 12);
            let matrix = controller.zones[1].matrix.as_ref().unwrap();
            assert_eq!((matrix.height, matrix.width), (2, 4));
            assert_eq!(matrix.map[5], 5);
            assert_eq!(matrix.map[7], u32::MAX);
            assert_eq!(controller.led_count, 18);
            assert_eq!(controller.colors.len(), 18);
            assert_eq!(controller.colors[2], Color { r: 2, g: 0, b: 0 });
        }
    }

    #[test]
    fn a_truncated_or_resized_description_is_refused() {
        let reply = DescriptionWriter::device(4, 0, "Board", &[("Header", 1, 8, None)]);
        assert!(parse_controller(0, &reply[..reply.len() - 3], 4).is_err());
        let mut wrong = reply.clone();
        wrong[0] = wrong[0].wrapping_add(1);
        assert!(parse_controller(0, &wrong, 4).is_err());
        // Read with the wrong protocol, the fields do not line up.
        assert!(parse_controller(0, &reply, 3).is_err());
    }

    #[test]
    fn the_mode_block_goes_back_unchanged_in_update_mode() {
        let reply = DescriptionWriter::device(3, 0, "Board", &[("Header", 1, 8, None)]);
        let controller = parse_controller(0, &reply, 3).unwrap();
        let payload = update_mode_payload(1, &controller.modes[1]);
        assert_eq!(
            u32::from_le_bytes(payload[..4].try_into().unwrap()) as usize,
            payload.len()
        );
        assert_eq!(&payload[4..8], &1i32.to_le_bytes());
        assert_eq!(&payload[8..], controller.modes[1].raw.as_slice());
    }

    #[test]
    fn update_leds_sends_its_own_size_the_count_and_rgb_colors() {
        let payload = update_leds_payload(&[Color { r: 1, g: 2, b: 3 }, Color { r: 255, g: 0, b: 128 }]);
        assert_eq!(payload.len(), 4 + 2 + 8);
        assert_eq!(u32::from_le_bytes(payload[..4].try_into().unwrap()), 14);
        assert_eq!(u16::from_le_bytes(payload[4..6].try_into().unwrap()), 2);
        assert_eq!(&payload[6..], &[1, 2, 3, 0, 255, 0, 128, 0]);
    }

    #[test]
    fn packets_are_framed_and_partial_ones_wait() {
        let mut stream = header(0, DEVICE_LIST_UPDATED, 0).to_vec();
        stream.extend_from_slice(&header(2, REQUEST_PROTOCOL_VERSION, 4));
        stream.extend_from_slice(&6u32.to_le_bytes());
        let (packet, used) = next_packet(&stream).unwrap().unwrap();
        assert_eq!(
            (packet.device, packet.id, packet.data.len(), used),
            (0, DEVICE_LIST_UPDATED, 0, 16)
        );
        let rest = &stream[used..];
        assert!(next_packet(&rest[..18]).unwrap().is_none());
        let (packet, _) = next_packet(rest).unwrap().unwrap();
        assert_eq!(
            (packet.device, packet.id, packet.data),
            (2, REQUEST_PROTOCOL_VERSION, 6u32.to_le_bytes().to_vec())
        );
        assert!(next_packet(b"HTTP/1.1 400 Bad Request\r\n").is_err());
    }

    #[test]
    fn plugins_and_their_effects_are_read() {
        // As OpenRGB 1.1 and the Effects plugin answered on a real system.
        let mut reply = 0u32.to_le_bytes().to_vec();
        reply.extend_from_slice(&2u16.to_le_bytes());
        for (name, version, index, sdk) in [
            ("JamBaLinux Sonic Core", "0.1.0", 0u32, 0u32),
            (EFFECTS_PLUGIN, "1.1+ (git5)", 1, 2),
        ] {
            for text in [name, "description", version] {
                reply.extend_from_slice(&((text.len() + 1) as u16).to_le_bytes());
                reply.extend_from_slice(text.as_bytes());
                reply.push(0);
            }
            reply.extend_from_slice(&index.to_le_bytes());
            reply.extend_from_slice(&sdk.to_le_bytes());
        }
        let plugins = parse_plugins(&reply).unwrap();
        assert_eq!(plugins[1].name, EFFECTS_PLUGIN);
        assert_eq!((plugins[1].index, plugins[1].sdk_version), (1, 2));

        let mut effects = 0u32.to_le_bytes().to_vec();
        effects.extend_from_slice(&2u16.to_le_bytes());
        for (name, running) in [("Rainbow wave", 1u8), ("Breathing", 0)] {
            for text in [name, "description"] {
                effects.extend_from_slice(&((text.len() + 1) as u16).to_le_bytes());
                effects.extend_from_slice(text.as_bytes());
                effects.push(0);
            }
            effects.push(running);
        }
        let effects = parse_effects(&effects).unwrap();
        assert_eq!(effects.len(), 2);
        assert!(effects[0].running && !effects[1].running);
        assert_eq!(effects[0].name, "Rainbow wave");
    }

    #[test]
    fn profile_lists_and_colors_are_read() {
        let mut reply = 0u32.to_le_bytes().to_vec();
        reply.extend_from_slice(&2u16.to_le_bytes());
        for name in ["blue", "AzulRoxo"] {
            reply.extend_from_slice(&((name.len() + 1) as u16).to_le_bytes());
            reply.extend_from_slice(name.as_bytes());
            reply.push(0);
        }
        assert_eq!(parse_profiles(&reply).unwrap(), ["blue", "AzulRoxo"]);
        assert_eq!(Color::parse("#FF8000"), Some(Color { r: 255, g: 128, b: 0 }));
        assert_eq!(Color::parse("ff8000"), None);
        assert_eq!(Color { r: 255, g: 128, b: 0 }.hex(), "#ff8000");
    }
}

use std::arch::asm;
use std::env;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::raw::{c_int, c_ulong};

const I2C_SLAVE: c_ulong = 0x0703;
const I2C_SMBUS: c_ulong = 0x0720;
const I2C_SMBUS_WRITE: u8 = 0;
const I2C_SMBUS_READ: u8 = 1;
const I2C_SMBUS_BYTE_DATA: u32 = 2;
const O_RDWR: c_int = 2;
const DEFAULT_ADDRESS: u16 = 0x54;
const FAN_REGISTER: u8 = 0xf0;
const ADAPTER_PREFIX: &str = "SMBus I801 adapter at ";
const SUPERIO_PORT: u16 = 0xa8f0;
const SUPERIO_FAN_REGISTERS: [u8; 3] = [0x6b, 0x73, 0xa3];

#[repr(C)]
union I2cSmbusData {
    byte: u8,
    _padding: [u8; 34],
}

#[repr(C)]
struct I2cSmbusIoctlData {
    read_write: u8,
    command: u8,
    size: u32,
    data: *mut I2cSmbusData,
}

unsafe extern "C" {
    fn close(fd: c_int) -> c_int;
    fn iopl(level: c_int) -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn open(path: *const i8, flags: c_int, ...) -> c_int;
}

fn usage() {
    eprintln!(
        "Usage: superio-fanctl --value 50..255 [--i2c-only] [--legacy-max] [--bus auto|N|/dev/i2c-N] [--address 0x54] [--channel all]"
    );
    eprintln!("       superio-fanctl --read [--bus auto|N|/dev/i2c-N] [--address 0x54]");
    eprintln!("       superio-fanctl --read-superio");
    eprintln!("       superio-fanctl --probe-superio");
}

fn parse_number(value: &str, name: &str) -> Result<u16, String> {
    let parsed = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map(|hex| u16::from_str_radix(hex, 16))
        .unwrap_or_else(|| value.parse::<u16>());
    parsed.map_err(|_| format!("invalid {name}: {value}"))
}

fn clamp_pwm(value: u16) -> u8 {
    value.clamp(50, 250) as u8
}

#[inline]
unsafe fn superio_outb(value: u8, port: u16) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
}

#[inline]
unsafe fn superio_inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", in("dx") port, out("al") value, options(nostack, preserves_flags));
    value
}

unsafe fn superio_get_value(register: u8) -> u8 {
    superio_outb(0x4e, SUPERIO_PORT);
    let current = superio_inb(SUPERIO_PORT);
    superio_outb(current & 0xf8, SUPERIO_PORT);
    superio_outb(register, SUPERIO_PORT);
    superio_inb(SUPERIO_PORT)
}

fn discover_i801_bus() -> Result<String, String> {
    let entries = fs::read_dir("/sys/class/i2c-dev")
        .map_err(|error| format!("cannot scan /sys/class/i2c-dev: {error}"))?;
    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if !name.starts_with("i2c-") {
            continue;
        }
        let adapter_name = fs::read_to_string(entry.path().join("name")).unwrap_or_default();
        if adapter_name.trim().starts_with(ADAPTER_PREFIX) {
            matches.push(name.into_owned());
        }
    }
    matches.sort();
    matches
        .into_iter()
        .next()
        .ok_or_else(|| format!("no I2C adapter matching {ADAPTER_PREFIX:?} was found"))
}

fn device_path(bus: Option<&str>) -> Result<String, String> {
    match bus {
        None | Some("auto") => Ok(format!("/dev/{}", discover_i801_bus()?)),
        Some(value) if value.starts_with("/dev/") => Ok(value.to_string()),
        Some(value) if value.starts_with("i2c-") => Ok(format!("/dev/{value}")),
        Some(value) => {
            let number = parse_number(value, "bus")?;
            Ok(format!("/dev/i2c-{number}"))
        }
    }
}

struct I2cDevice {
    fd: c_int,
    path: String,
}

impl I2cDevice {
    fn open(path: String, address: u16) -> Result<Self, Box<dyn std::error::Error>> {
        if address > 0x7f {
            return Err("I2C address must be in 0..0x7f".into());
        }
        let cpath = CString::new(path.as_str())?;
        let fd = unsafe { open(cpath.as_ptr(), O_RDWR) };
        if fd < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let result = unsafe { ioctl(fd, I2C_SLAVE, address as c_ulong) };
        if result < 0 {
            let error = io::Error::last_os_error();
            unsafe { close(fd) };
            return Err(error.into());
        }
        Ok(Self { fd, path })
    }

    fn transfer(&self, read: bool, value: u8) -> Result<u8, Box<dyn std::error::Error>> {
        let mut data = I2cSmbusData { byte: value };
        let mut args = I2cSmbusIoctlData {
            read_write: if read {
                I2C_SMBUS_READ
            } else {
                I2C_SMBUS_WRITE
            },
            command: FAN_REGISTER,
            size: I2C_SMBUS_BYTE_DATA,
            data: &mut data,
        };
        let result = unsafe { ioctl(self.fd, I2C_SMBUS, &mut args as *mut I2cSmbusIoctlData) };
        if result < 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(unsafe { data.byte })
    }
}

impl Drop for I2cDevice {
    fn drop(&mut self) {
        unsafe { close(self.fd) };
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        usage();
        return Ok(());
    }

    let mut value = None;
    let mut read = false;
    let mut read_superio = false;
    let mut probe_superio = false;
    let mut bus = None;
    let mut address = DEFAULT_ADDRESS;
    let mut channel = String::from("all");
    let mut i2c_only = false;
    let mut legacy_max = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--value" | "-v" => {
                index += 1;
                value = Some(parse_number(
                    args.get(index).ok_or("missing value")?,
                    "value",
                )?);
            }
            "--read" | "-r" => read = true,
            "--read-superio" => read_superio = true,
            "--probe-superio" => probe_superio = true,
            "--i2c-only" => i2c_only = true,
            "--legacy-max" => legacy_max = true,
            "--bus" | "-b" => {
                index += 1;
                bus = Some(args.get(index).ok_or("missing bus")?.clone());
            }
            "--address" | "-a" => {
                index += 1;
                address = parse_number(args.get(index).ok_or("missing address")?, "address")?;
            }
            "--channel" | "-c" => {
                index += 1;
                channel = args.get(index).ok_or("missing channel")?.clone();
            }
            _ => {
                usage();
                return Err(format!("unknown argument: {}", args[index]).into());
            }
        }
        index += 1;
    }

    if channel != "all" {
        return Err(
            "the official I2C controller has one shared fan PWM; --channel must be all".into(),
        );
    }
    if read_superio {
        if read || value.is_some() {
            return Err("--read-superio cannot be combined with --read or --value".into());
        }
        if unsafe { iopl(3) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        for register in SUPERIO_FAN_REGISTERS {
            let current = unsafe { superio_get_value(register) };
            println!("superio_port=0xa8f0 register=0x{register:02x} value={current}");
        }
        return Ok(());
    }
    if probe_superio {
        if read || value.is_some() {
            return Err("--probe-superio cannot be combined with --read or --value".into());
        }
        if unsafe { iopl(3) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        // These are read-only diagnostics for the vendor Super I/O window:
        // chip ID/revision, logical-device selector, base-address bytes,
        // official selector registers, and the three legacy fan registers.
        for register in [
            0x20u8, 0x21, 0x07, 0x30, 0x60, 0x61, 0x62, 0x63, 0x16, 0x17, 0x1f, 0x6b, 0x73, 0xa3,
        ] {
            let current = unsafe { superio_get_value(register) };
            println!("superio_port=0xa8f0 register=0x{register:02x} value={current}");
        }
        return Ok(());
    }
    if i2c_only {
        let requested = value.ok_or("--value is required")?;
        if i2c_only && requested > 250 && requested != 255 {
            return Err("--i2c-only accepts 50..250 or 255".into());
        }
        let pwm = if i2c_only && requested == 255 {
            255
        } else {
            clamp_pwm(requested)
        };
        let path = device_path(bus.as_deref())?;
        let device = I2cDevice::open(path, address)?;
        device.transfer(false, pwm)?;
        return Ok(());
    }
    if read == value.is_some() {
        return Err("specify exactly one of --read or --value".into());
    }
    let path = device_path(bus.as_deref())?;
    let device = I2cDevice::open(path, address)?;
    if read {
        let current = device.transfer(true, 0)?;
        println!(
            "bus={} address=0x{address:02x} register=0xf0 value={current}",
            device.path
        );
    } else {
        let requested = value.expect("value was checked");
        let boost = legacy_max || requested > 250;
        let pwm = if boost { 255 } else { clamp_pwm(requested) };
        // Super I/O control is intentionally disabled. The legacy flag only
        // requests the I2C maximum value (255).
        device.transfer(false, pwm)?;
    }
    Ok(())
}

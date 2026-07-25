use std::arch::asm;
use std::env;
use std::io;

const IO_PORT: u16 = 0xa8f0;
const FAN_REGISTERS: [u8; 3] = [0x6b, 0x73, 0xa3];

unsafe extern "C" {
    fn iopl(level: i32) -> i32;
}

#[inline]
unsafe fn outb(value: u8, port: u16) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nostack, preserves_flags));
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", in("dx") port, out("al") value, options(nostack, preserves_flags));
    value
}

unsafe fn set_value(register: u8, value: u8) {
    // This is the exact index/data sequence used by unasfan-ctl::SetValueWithIO.
    outb(0x4e, IO_PORT);
    let current = inb(IO_PORT);
    outb(current & 0xf8, IO_PORT);
    outb(register, IO_PORT);
    outb(value, IO_PORT);
}

fn usage() {
    eprintln!("Usage: superio-fanctl --value 0..255 [--channel 0|1|2|all]");
}

fn parse_u8(value: &str, name: &str) -> Result<u8, String> {
    let parsed = value.parse::<u16>().map_err(|_| format!("invalid {name}: {value}"))?;
    u8::try_from(parsed).map_err(|_| format!("{name} must be 0..255"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        usage();
        return Ok(());
    }

    let mut value = None;
    let mut channel = String::from("all");
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--value" | "-v" => {
                index += 1;
                let raw = args.get(index).ok_or("missing value")?;
                value = Some(parse_u8(raw, "value")?);
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

    let value = value.ok_or("--value is required")?;
    let channels: Vec<usize> = match channel.as_str() {
        "all" => vec![0, 1, 2],
        "0" | "1" | "2" => vec![channel.parse()?],
        _ => return Err(format!("invalid channel: {channel}").into()),
    };

    let result = unsafe { iopl(3) };
    if result != 0 {
        return Err(io::Error::last_os_error().into());
    }

    for channel in channels {
        unsafe {
            // The original helper resets these selector registers before each fan write.
            set_value(0x16, 0);
            set_value(0x17, 0);
            set_value(0x1f, 0);
            set_value(FAN_REGISTERS[channel], value);
        }
    }
    Ok(())
}

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

static STOP: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
}

extern "C" fn stop_handler(_: i32) {
    STOP.store(true, Ordering::Relaxed);
}

fn env_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

fn log_line(log_path: &Path, message: &str) {
    let line = format!("{} - {}\n", chrono_like_now(), message);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = file.write_all(line.as_bytes());
    }
    print!("{line}");
    let _ = io::stdout().flush();
}

fn chrono_like_now() -> String {
    match Command::new("date").args(["+%Y-%m-%d %H:%M:%S"]).output() {
        Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_string(),
        Err(_) => "1970-01-01 00:00:00".to_string(),
    }
}

fn read_temp(path: &str) -> Option<f64> {
    let raw = fs::read_to_string(path).ok()?.trim().parse::<f64>().ok()?;
    Some(if raw > 200.0 { raw / 1000.0 } else { raw })
}

fn read_temp_inputs(dir: &Path) -> Option<f64> {
    let mut maximum = None;
    let sensors = fs::read_dir(dir).ok()?;
    for sensor in sensors.flatten() {
        let file_name = sensor.file_name();
        let file_name = file_name.to_string_lossy();
        if !file_name.starts_with("temp") || !file_name.ends_with("_input") {
            continue;
        }
        if let Some(temp) = read_temp(sensor.path().to_string_lossy().as_ref()) {
            maximum = Some(maximum.map_or(temp, |current: f64| current.max(temp)));
        }
    }
    maximum
}

fn read_hwmon_max(name_filter: impl Fn(&str) -> bool) -> Option<f64> {
    let mut maximum = None;
    let entries = fs::read_dir("/sys/class/hwmon").ok()?;
    for entry in entries.flatten() {
        let hwmon = entry.path();
        let name = fs::read_to_string(hwmon.join("name"))
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        if name_filter(&name) {
            if let Some(temp) = read_temp_inputs(&hwmon) {
                maximum = Some(maximum.map_or(temp, |current: f64| current.max(temp)));
            }
        }
    }
    maximum
}

fn read_cpu_temp_official() -> Option<f64> {
    // sensors -j exposes this adapter as coretemp-isa-0000. The JSON values
    // used by wygpio are the *_input fields under that adapter, which map
    // directly to the coretemp hwmon files.
    read_hwmon_max(|name| name == "coretemp" || name == "coretemp-isa-0000")
}

fn read_disk_temp_official() -> Option<f64> {
    // Prefer the kernel's drivetemp/nvme hwmon adapters. These expose the
    // same physical temperatures that smartctl reports, without spawning
    // fdisk or smartctl and without adding package dependencies.
    let mut maximum = None;
    if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            let hwmon = entry.path();
            let name = fs::read_to_string(hwmon.join("name"))
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if !(name.contains("drivetemp")
                || name.contains("nvme")
                || name.contains("ata")
                || name.contains("scsi"))
            {
                continue;
            }
            if let Some(temp) = read_temp_inputs(&hwmon) {
                maximum = Some(maximum.map_or(temp, |current: f64| current.max(temp)));
            }
        }
    }
    if let Ok(entries) = fs::read_dir("/sys/class/thermal") {
        for entry in entries.flatten() {
            let zone = entry.path();
            let kind = fs::read_to_string(zone.join("type"))
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if !(kind.contains("disk")
                || kind.contains("drive")
                || kind.contains("ssd")
                || kind.contains("nvme"))
            {
                continue;
            }
            if let Some(temp) = read_temp(&zone.join("temp").to_string_lossy()) {
                maximum = Some(maximum.map_or(temp, |current: f64| current.max(temp)));
            }
        }
    }
    maximum
}

fn clamp_pwm(value: i32) -> u8 {
    value.clamp(50, 250) as u8
}

#[derive(Debug)]
struct OfficialController {
    state: u8,
    counter: i32,
    current_pwm: i32,
    previous_temp: f64,
    average_temp: f64,
    history: Vec<f64>,
}

impl OfficialController {
    fn new() -> Self {
        // MyGpio's constructor initializes state=1, counter=0, current PWM=25.
        Self {
            state: 1,
            counter: 0,
            current_pwm: 25,
            previous_temp: 0.0,
            average_temp: 0.0,
            history: Vec::with_capacity(6),
        }
    }

    fn update(&mut self, cpu_temp: f64, disk_temp: f64) -> (u8, u8, f64) {
        let effective = (cpu_temp - 10.0).max(disk_temp);

        // These two states return immediately in the original aiFanSpeed().
        if effective >= 60.0 {
            self.state = 5;
            self.current_pwm = 255;
            return (self.state, clamp_pwm(self.current_pwm), effective);
        }
        if effective < 30.0 {
            self.state = 1;
            self.current_pwm = 50;
            return (self.state, clamp_pwm(self.current_pwm), effective);
        }
        self.state = if effective >= 55.0 {
            4
        } else if effective >= 45.0 {
            3
        } else {
            2
        };

        if self.previous_temp <= 0.0 {
            self.previous_temp = effective;
            self.average_temp = effective;
        }
        let previous = self.previous_temp;
        let average = self.average_temp;

        // The constants and ordering below are taken from the disassembled
        // MyGpio::aiFanSpeed(double, double), including its truncation rules.
        match self.state {
            2 => {
                if effective - 3.0 > previous {
                    self.current_pwm =
                        (self.current_pwm as f64 + (effective - previous) * 5.0) as i32;
                    self.counter = 0;
                } else if effective > average {
                    self.current_pwm =
                        (self.current_pwm as f64 + (effective - average) * 3.0) as i32;
                    self.counter = 0;
                } else if effective.trunc() < previous.trunc()
                    && effective.trunc() < average.trunc()
                {
                    self.current_pwm =
                        (self.current_pwm as f64 - (previous - effective + 1.0)) as i32;
                    self.counter = 0;
                } else if effective == average {
                    self.counter += 1;
                    if self.counter > 10 {
                        self.current_pwm -= 1;
                        self.counter = 0;
                    }
                }
            }
            3 => {
                if effective - 2.0 > previous {
                    self.current_pwm =
                        (self.current_pwm as f64 + (effective - previous) * 5.0) as i32;
                    self.counter = 0;
                } else if effective > average {
                    self.current_pwm =
                        (self.current_pwm as f64 + (effective - average) * 5.0) as i32;
                    self.counter = 0;
                } else if effective.trunc() < previous.trunc()
                    && effective.trunc() < average.trunc()
                {
                    self.current_pwm =
                        (self.current_pwm as f64 - (previous - effective + 1.0)) as i32;
                    self.counter = 0;
                } else if effective == average {
                    self.counter += 1;
                    if self.counter > 5 {
                        self.current_pwm -= 1;
                        self.counter = 0;
                    }
                }
            }
            4 => {
                if effective - 2.0 > previous {
                    self.current_pwm =
                        (self.current_pwm as f64 + (effective - previous) * 10.0) as i32;
                    self.counter = 0;
                } else if effective > average {
                    self.current_pwm =
                        (self.current_pwm as f64 + (effective - average) * 5.0) as i32;
                    self.counter = 0;
                } else if effective.trunc() < previous.trunc()
                    && effective.trunc() < average.trunc()
                {
                    self.current_pwm =
                        (self.current_pwm as f64 - (previous - effective + 1.0)) as i32;
                    self.counter = 0;
                } else if effective == average {
                    self.counter += 1;
                    if self.counter > 5 {
                        self.current_pwm -= 1;
                        self.counter = 0;
                    }
                }
            }
            _ => unreachable!(),
        }

        // Original code updates previous/history only after calculating PWM.
        self.previous_temp = effective;
        self.history.push(effective);
        if self.history.len() > 6 {
            self.history.remove(0);
        }
        self.average_temp = self.history.iter().sum::<f64>() / self.history.len() as f64;

        // The jump table after aiFanSpeed() applies an additional per-state
        // output range before onFanIntervalEvent() writes the value.
        match self.state {
            2 => {
                self.current_pwm = if effective <= 40.0 {
                    50
                } else {
                    self.current_pwm.clamp(50, 125)
                };
            }
            3 => {
                self.current_pwm = if effective <= 50.0 {
                    80
                } else {
                    self.current_pwm.clamp(80, 180)
                };
            }
            4 => self.current_pwm = self.current_pwm.clamp(150, 255),
            _ => {}
        }
        (self.state, clamp_pwm(self.current_pwm), effective)
    }
}

fn write_pwm(appdest: &str, pwm: u8) -> bool {
    Command::new(format!("{appdest}/bin/superio-fanctl"))
        .args(["--value", &pwm.to_string(), "--channel", "all"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn main() {
    unsafe {
        signal(2, stop_handler);
        signal(15, stop_handler);
    }
    let temp_path = env_or("temp_path", "/sys/class/thermal/thermal_zone1/temp");
    let disk_temp_path = env::var("disk_temp_path")
        .ok()
        .filter(|path| !path.trim().is_empty());
    let poll_seconds = env_or("poll_seconds", "10")
        .parse::<u64>()
        .unwrap_or(10)
        .max(1);
    let appdest = env_or("TRIM_APPDEST", ".");
    let log_path = Path::new(&env_or("TRIM_PKGVAR", ".")).join("fancontrol.log");
    let mut last_pwm: Option<u8> = None;
    let mut controller = OfficialController::new();

    log_line(&log_path, &format!("Rust fan daemon started: backend=superio, algorithm=wygpio-exact, temp={temp_path}, poll={poll_seconds}s"));
    while !STOP.load(Ordering::Relaxed) {
        let cpu = read_cpu_temp_official().or_else(|| read_temp(&temp_path));
        let disk = disk_temp_path
            .as_deref()
            .and_then(read_temp)
            .or_else(read_disk_temp_official);
        let (cpu, disk) = match (cpu, disk) {
            (Some(cpu), Some(disk)) => (cpu, disk),
            (Some(cpu), None) => (cpu, 0.0),
            (None, Some(disk)) => (0.0, disk),
            (None, None) => {
                if last_pwm != Some(250) {
                    log_line(
                        &log_path,
                        "Temperature read failed; applying fail-safe PWM 250",
                    );
                    let _ = write_pwm(&appdest, 250);
                    last_pwm = Some(250);
                }
                thread::sleep(Duration::from_secs(poll_seconds));
                continue;
            }
        };
        let (state, pwm, effective) = controller.update(cpu, disk);
        if last_pwm != Some(pwm) {
            log_line(&log_path, &format!("temperature cpu={cpu:.1} disk={disk:.1} effective={effective:.1}C average={:.1}C state={state} raw_pwm={} -> PWM {pwm}", controller.average_temp, controller.current_pwm));
            if !write_pwm(&appdest, pwm) {
                log_line(&log_path, &format!("Fan write failed at PWM {pwm}"));
            }
            last_pwm = Some(pwm);
        }
        thread::sleep(Duration::from_secs(poll_seconds));
    }
    let _ = write_pwm(&appdest, 250);
    log_line(
        &log_path,
        "Rust fan daemon stopped; fail-safe PWM 250 applied",
    );
}

#[cfg(test)]
mod tests {
    use super::{clamp_pwm, OfficialController};

    #[test]
    fn follows_wygpio_states() {
        let mut controller = OfficialController::new();
        assert_eq!(controller.update(20.0, 0.0).0, 1);
        assert_eq!(controller.update(40.0, 0.0).0, 2);
        assert_eq!(controller.update(55.0, 0.0).0, 3);
        assert_eq!(controller.update(65.0, 0.0).0, 4);
        assert_eq!(controller.update(70.0, 0.0).0, 5);
        assert_eq!(controller.update(20.0, 0.0).1, 50);
    }

    #[test]
    fn clamps_controller_range() {
        assert_eq!(clamp_pwm(0), 50);
        assert_eq!(clamp_pwm(255), 250);
    }

    #[test]
    fn uses_official_state_coefficients() {
        let mut state2 = OfficialController::new();
        state2.update(51.0, 0.0); // effective=41, initializes the history
        state2.update(52.0, 0.0); // delta=1, state 2 uses coefficient 3
        assert_eq!(state2.current_pwm, 53);

        let mut sharp_state2 = OfficialController::new();
        sharp_state2.update(46.0, 0.0); // effective=36
        sharp_state2.update(54.0, 0.0); // effective=44, delta=8 uses coefficient 5
        assert_eq!(sharp_state2.current_pwm, 90);

        let mut state4 = OfficialController::new();
        state4.update(65.0, 0.0); // effective=55
        state4.update(68.0, 0.0); // delta=3, state 4 uses coefficient 10
        assert_eq!(state4.current_pwm, 180);

        let mut state3 = OfficialController::new();
        state3.update(55.0, 0.0); // effective=45
        assert_eq!(state3.current_pwm, 80); // state 3 minimum output range
    }
}

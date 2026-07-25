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

fn read_temp(path: &str) -> Option<i32> {
    let raw = fs::read_to_string(path).ok()?.trim().parse::<i64>().ok()?;
    Some(if raw > 200 { (raw / 1000) as i32 } else { raw as i32 })
}

fn clamp_pwm(value: i32) -> u8 {
    value.clamp(50, 250) as u8
}

fn percent_for_temp(temp: i32, idle: i32) -> i32 {
    if temp >= 80 { 100 } else if temp >= 75 { 55 } else if temp >= 65 { 39 } else if temp >= 55 { 31 } else { idle }
}

fn pwm_for_temp(temp: i32, idle: i32) -> u8 {
    clamp_pwm(percent_for_temp(temp, idle) * 254 / 100)
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
    let disk_temp_path = env::var("disk_temp_path").ok();
    let poll_seconds = env_or("poll_seconds", "10").parse::<u64>().unwrap_or(10).max(1);
    let idle = env_or("idle_percent", "31").parse::<i32>().unwrap_or(31).clamp(0, 100);
    let appdest = env_or("TRIM_APPDEST", ".");
    let log_path = Path::new(&env_or("TRIM_PKGVAR", ".")).join("fancontrol.log");
    let mut last_pwm: Option<u8> = None;
    let mut history: Vec<i32> = Vec::with_capacity(5);

    log_line(&log_path, &format!("Rust fan daemon started: backend=superio, temp={temp_path}, poll={poll_seconds}s, idle={idle}%"));
    while !STOP.load(Ordering::Relaxed) {
        let cpu = read_temp(&temp_path);
        let disk = disk_temp_path.as_deref().and_then(read_temp);
        let temp = match (cpu, disk) {
            (Some(cpu), Some(disk)) => cpu.max(disk),
            (Some(cpu), None) => cpu,
            (None, Some(disk)) => disk,
            (None, None) => {
                if last_pwm != Some(250) {
                    log_line(&log_path, "Temperature read failed; applying fail-safe PWM 250");
                    let _ = write_pwm(&appdest, 250);
                    last_pwm = Some(250);
                }
                thread::sleep(Duration::from_secs(poll_seconds));
                continue;
            }
        };
        history.push(temp);
        if history.len() > 5 { history.remove(0); }
        let average = history.iter().sum::<i32>() / history.len() as i32;
        let pwm = pwm_for_temp(average, idle);
        if last_pwm != Some(pwm) {
            let percent = percent_for_temp(average, idle);
            log_line(&log_path, &format!("temperature cpu={cpu:?} disk={disk:?} average={average}C -> {percent}% -> PWM {pwm}"));
            if !write_pwm(&appdest, pwm) { log_line(&log_path, &format!("Fan write failed at PWM {pwm}")); }
            last_pwm = Some(pwm);
        }
        thread::sleep(Duration::from_secs(poll_seconds));
    }
    let _ = write_pwm(&appdest, 250);
    log_line(&log_path, "Rust fan daemon stopped; fail-safe PWM 250 applied");
}

#[cfg(test)]
mod tests {
    use super::{clamp_pwm, percent_for_temp, pwm_for_temp};

    #[test]
    fn follows_unas_thresholds() {
        assert_eq!(percent_for_temp(54, 31), 31);
        assert_eq!(percent_for_temp(55, 31), 31);
        assert_eq!(percent_for_temp(65, 31), 39);
        assert_eq!(percent_for_temp(75, 31), 55);
        assert_eq!(percent_for_temp(80, 31), 100);
    }

    #[test]
    fn clamps_controller_range() {
        assert_eq!(clamp_pwm(0), 50);
        assert_eq!(pwm_for_temp(80, 31), 250);
        assert_eq!(clamp_pwm(255), 250);
    }
}

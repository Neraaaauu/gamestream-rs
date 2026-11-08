use std::fs;
use std::time::Instant;

pub struct ProcessMetrics {
    last_check: Instant,
    last_cpu_time_ticks: u64,
    clock_ticks_per_sec: f64,
    page_size_bytes: usize,
}

impl ProcessMetrics {
    pub fn new() -> Self {
        let clock_ticks_per_sec = unsafe { libc::sysconf(libc::_SC_CLK_TCK) as f64 };
        let page_size_bytes = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize };
        let initial_ticks = Self::read_cpu_ticks().unwrap_or(0);

        Self {
            last_check: Instant::now(),
            last_cpu_time_ticks: initial_ticks,
            clock_ticks_per_sec: if clock_ticks_per_sec > 0.0 { clock_ticks_per_sec } else { 100.0 },
            page_size_bytes: if page_size_bytes > 0 { page_size_bytes } else { 4096 },
        }
    }

    fn read_cpu_ticks() -> Option<u64> {
        let stat_str = fs::read_to_string("/proc/self/stat").ok()?;
        // Format of /proc/self/stat: pid (comm) state ppid ... utime(14) stime(15) ...
        // Note that comm can contain spaces and parentheses, so split after the last ')'
        let rparen = stat_str.rfind(')')?;
        let rest = &stat_str[rparen + 2..];
        let fields: Vec<&str> = rest.split_whitespace().collect();
        // fields[0] is state (index 3 in whole stat line)
        // fields[11] is utime (field 14), fields[12] is stime (field 15)
        if fields.len() > 12 {
            let utime: u64 = fields[11].parse().ok()?;
            let stime: u64 = fields[12].parse().ok()?;
            Some(utime + stime)
        } else {
            None
        }
    }

    /// Retorna (cpu_usage_pct, rss_memory_mb)
    pub fn sample(&mut self) -> (f64, f64) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_check).as_secs_f64();
        let current_ticks = Self::read_cpu_ticks().unwrap_or(self.last_cpu_time_ticks);

        let delta_ticks = current_ticks.saturating_sub(self.last_cpu_time_ticks) as f64;
        let cpu_pct = if elapsed > 0.0 && self.clock_ticks_per_sec > 0.0 {
            (delta_ticks / self.clock_ticks_per_sec / elapsed) * 100.0
        } else {
            0.0
        };

        self.last_check = now;
        self.last_cpu_time_ticks = current_ticks;

        // Memory RSS
        let rss_mb = if let Ok(statm) = fs::read_to_string("/proc/self/statm") {
            let parts: Vec<&str> = statm.split_whitespace().collect();
            if parts.len() >= 2 {
                let resident_pages: usize = parts[1].parse().unwrap_or(0);
                (resident_pages * self.page_size_bytes) as f64 / (1024.0 * 1024.0)
            } else {
                0.0
            }
        } else {
            0.0
        };

        (cpu_pct, rss_mb)
    }
}

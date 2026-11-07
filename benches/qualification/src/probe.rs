use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub struct Levels {
    pub fds: Option<usize>,
    pub tasks: usize,
}

pub fn open_fds() -> Option<usize> {
    std::fs::read_dir("/proc/self/fd").ok().map(Iterator::count)
}

pub fn levels() -> Levels {
    Levels {
        fds: open_fds(),
        tasks: tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
    }
}

pub fn drift(start: Levels, end: Levels, fd_slack: usize, task_slack: usize) -> Vec<String> {
    let mut wrong = Vec::new();
    if let (Some(a), Some(b)) = (start.fds, end.fds) {
        if b > a + fd_slack {
            wrong.push(format!(
                "leak: open fds {a} grew to {b}, allowed {fd_slack} more"
            ));
        }
    }
    if end.tasks > start.tasks + task_slack {
        wrong.push(format!(
            "leak: alive tasks {} grew to {}, allowed {task_slack} more",
            start.tasks, end.tasks
        ));
    }
    wrong
}

pub fn describe(levels: Levels) -> Value {
    json!({"fds": levels.fds, "tasks": levels.tasks})
}

pub fn rss_kib() -> u64 {
    let text = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    text.split_whitespace()
        .nth(1)
        .and_then(|p| p.parse::<u64>().ok())
        .unwrap_or(0)
        * 4
}

pub fn cpu_ticks() -> u64 {
    let text = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields: Vec<&str> = text
        .rsplit(')')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    [11, 12]
        .iter()
        .filter_map(|&i| fields.get(i)?.parse::<u64>().ok())
        .sum()
}

pub fn cpu_seconds() -> f64 {
    cpu_ticks() as f64 / 100.0
}

pub fn tcp_counter(name: &str) -> u64 {
    let text = std::fs::read_to_string("/proc/net/netstat").unwrap_or_default();
    let mut lines = text.lines().filter(|line| line.starts_with("TcpExt:"));
    let (Some(names), Some(values)) = (lines.next(), lines.next()) else {
        return 0;
    };
    names
        .split_whitespace()
        .zip(values.split_whitespace())
        .find(|(held, _)| *held == name)
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0)
}

use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub struct Levels {
    pub fds: Option<usize>,
    pub tasks: usize,
}

fn open_fds() -> Option<usize> {
    std::fs::read_dir("/proc/self/fd").ok().map(Iterator::count)
}

pub fn levels() -> Levels {
    Levels {
        fds: open_fds(),
        tasks: tokio::runtime::Handle::current().metrics().num_alive_tasks(),
    }
}

pub fn drift(start: Levels, end: Levels, fd_slack: usize, task_slack: usize) -> Vec<String> {
    let mut wrong = Vec::new();
    if let (Some(a), Some(b)) = (start.fds, end.fds) {
        if b > a + fd_slack {
            wrong.push(format!("leak: open fds {a} grew to {b}, allowed {fd_slack} more"));
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

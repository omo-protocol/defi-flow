use std::path::Path;

use anyhow::{Result, bail};

use crate::run::registry::Registry;

pub fn run(name: &str, registry_dir: Option<&Path>) -> Result<()> {
    let reg = Registry::load(registry_dir)?;

    let entry = match reg.daemons.get(name) {
        Some(e) => e,
        None => bail!(
            "No strategy '{}' in registry. Run `defi-flow ps` to see registered strategies.",
            name
        ),
    };

    if !Registry::is_pid_alive(entry.pid) {
        println!(
            "Strategy '{}' (PID {}) is already dead. Cleaning up registry.",
            name, entry.pid
        );
        Registry::deregister(registry_dir, name)?;
        return Ok(());
    }

    println!("Stopping '{}' (PID {})...", name, entry.pid);

    // Send SIGTERM for graceful shutdown
    unsafe {
        libc::kill(entry.pid as i32, libc::SIGTERM);
    }

    // Wait up to 10 seconds for process to exit
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if !Registry::is_pid_alive(entry.pid) {
            println!("Strategy '{}' stopped. Deregistering.", name);
            Registry::deregister(registry_dir, name)?;
            return Ok(());
        }
    }

    // Force kill if still alive
    println!("Process didn't exit cleanly, sending SIGKILL...");
    unsafe {
        libc::kill(entry.pid as i32, libc::SIGKILL);
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    Registry::deregister(registry_dir, name)?;
    println!("Strategy '{}' killed and deregistered.", name);

    Ok(())
}

use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Result, bail};

use crate::run::registry::Registry;

pub fn run(name: &str, lines: usize, follow: bool, registry_dir: Option<&Path>) -> Result<()> {
    let reg = Registry::load(registry_dir)?;

    let entry = match reg.daemons.get(name) {
        Some(e) => e,
        None => bail!(
            "No strategy '{}' in registry. Run `defi-flow ps` to see registered strategies.",
            name
        ),
    };

    let log_path = &entry.log_file;
    if !log_path.exists() {
        bail!("Log file not found: {}", log_path.display());
    }

    if follow {
        // Tail -f style: print last N lines then follow
        print_tail(log_path, lines)?;
        follow_file(log_path)?;
    } else {
        print_tail(log_path, lines)?;
    }

    Ok(())
}

fn print_tail(path: &Path, n: usize) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let all_lines: Vec<&str> = content.lines().collect();
    let start = if all_lines.len() > n {
        all_lines.len() - n
    } else {
        0
    };
    for line in &all_lines[start..] {
        println!("{}", line);
    }
    Ok(())
}

fn follow_file(path: &Path) -> Result<()> {
    use std::io::Seek;

    let mut file = std::fs::File::open(path)?;
    file.seek(std::io::SeekFrom::End(0))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // No new data, sleep briefly
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Ok(_) => {
                print!("{}", line);
            }
            Err(e) => {
                eprintln!("Error reading log: {}", e);
                break;
            }
        }
    }

    #[allow(unreachable_code)]
    Ok(())
}

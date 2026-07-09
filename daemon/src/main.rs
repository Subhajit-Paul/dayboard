use std::error::Error;
use std::io::Write as _;
use std::time::Duration;

use caldav_core::Db;

fn main() {
    let mut args = std::env::args().skip(1);
    if let Some(arg) = args.next() {
        match arg.as_str() {
            "--install" => return install_service(),
            "--auth" => {
                if let Err(e) = caldav_core::auth::authenticate() {
                    eprintln!("caldavd: authentication failed: {e}");
                    std::process::exit(1);
                }
                return;
            }
            _ => {
                eprintln!("unknown argument: {arg}\nusage: caldavd [--install | --auth]");
                std::process::exit(1);
            }
        }
    }

    run();
}

fn run() {
    let db = Db::open_default().expect("failed to open database");
    let poll_secs: u64 = std::env::var("CALDAV_POLL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let sync_secs: u64 = std::env::var("CALDAV_SYNC_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);

    println!("caldavd: polling reminders every {poll_secs}s, syncing every {sync_secs}s (once connected via --auth)");
    let mut next_sync_at = caldav_core::db::now();
    loop {
        if let Err(e) = fire_due_reminders(&db) {
            eprintln!("caldavd: error checking reminders: {e}");
        }

        let now = caldav_core::db::now();
        if now >= next_sync_at {
            if caldav_core::auth::is_authenticated()
                && let Err(e) = caldav_core::sync::run(&db)
            {
                eprintln!("caldavd: sync error: {e}");
            }
            next_sync_at = now + sync_secs as i64;
        }

        std::thread::sleep(Duration::from_secs(poll_secs));
    }
}

fn fire_due_reminders(db: &Db) -> Result<(), Box<dyn Error>> {
    let due = db.list_due_reminders(caldav_core::db::now())?;
    for (reminder, task_title) in due {
        if let Err(e) = notify_rust::Notification::new()
            .summary("Reminder")
            .body(&task_title)
            .show()
        {
            eprintln!("caldavd: failed to show notification: {e}");
            continue;
        }
        db.mark_reminder_fired(reminder.id)?;
    }
    Ok(())
}

fn install_service() {
    let exe = std::env::current_exe().expect("failed to resolve current executable path");

    let unit = format!(
        "[Unit]\n\
         Description=caldav reminder daemon\n\n\
         [Service]\n\
         ExecStart={}\n\
         Restart=on-failure\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        exe.display()
    );

    let home = std::env::var("HOME").expect("HOME not set");
    let unit_dir = std::path::PathBuf::from(home).join(".config/systemd/user");
    std::fs::create_dir_all(&unit_dir).expect("failed to create systemd user unit dir");
    let unit_path = unit_dir.join("caldavd.service");

    std::fs::File::create(&unit_path)
        .and_then(|mut f| f.write_all(unit.as_bytes()))
        .expect("failed to write unit file");

    println!("caldavd: wrote {}", unit_path.display());

    run_systemctl(&["--user", "daemon-reload"]);
    run_systemctl(&["--user", "enable", "--now", "caldavd"]);

    println!("caldavd: installed and started. Check with `systemctl --user status caldavd`.");
}

fn run_systemctl(args: &[&str]) {
    let status = std::process::Command::new("systemctl")
        .args(args)
        .status()
        .expect("failed to run systemctl");
    if !status.success() {
        eprintln!("caldavd: `systemctl {}` failed", args.join(" "));
        std::process::exit(1);
    }
}

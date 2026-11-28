//! Launching Linux launchers and AppImages. Nothing is made executable or run with more rights
//! than the user has: a launcher that isn't trusted, or an AppImage that isn't allowed to run,
//! is refused with the reason.

use super::desktop;
use std::path::Path;
use std::process::{Command, Stdio};

/// Start a program on its own, so it outlives Filescape and doesn't share its terminal.
fn start(mut cmd: Command) -> Result<(), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let program = cmd.get_program().to_string_lossy().into_owned();
    cmd.spawn().map(drop).map_err(|e| format!("Couldn't start {program}: {e}"))
}

/// A terminal to run a command in: the user's choice ($TERMINAL), else the usual ones.
fn terminal(args: &[String]) -> Option<Command> {
    let user = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty());
    let known: [(&str, &[&str]); 6] = [("x-terminal-emulator", &["-e"]), ("gnome-terminal", &["--"]), ("konsole", &["-e"]), ("xfce4-terminal", &["-x"]), ("kitty", &[]), ("xterm", &["-e"])];
    let (prog, flag): (String, Vec<&str>) = match user {
        Some(t) if desktop::which(&t).is_some() => (t, vec!["-e"]),
        _ => known.iter().find(|(t, _)| desktop::which(t).is_some()).map(|(t, f)| (t.to_string(), f.to_vec()))?,
    };
    let mut c = Command::new(prog);
    c.args(flag).args(args);
    Some(c)
}

/// Launch a `.desktop` launcher, as the desktop's own menu would.
pub fn launch_entry(file: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("Couldn't read the launcher: {e}"))?;
    let e = desktop::entry(&text).ok_or("This launcher is damaged: it has no [Desktop Entry] section.")?;
    if let Some(why) = super::launcher_problem(file, &e) {
        return Err(why);
    }
    if e.kind == "Link" {
        return crate::actions::open(Path::new(e.url.as_deref().unwrap_or_default()));
    }
    let cmd = e.exec.as_deref().and_then(|x| desktop::command(x, &e, file)).ok_or("It doesn't say which program to run.")?;
    let mut c = if e.terminal {
        terminal(&cmd).ok_or("It runs in a terminal, and no terminal app was found.")?
    } else {
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..]);
        c
    };
    // Its own folder, else the home folder, as the desktop's menu starts programs.
    if let Some(dir) = e.path.clone().filter(|d| Path::new(d).is_dir()).or_else(|| std::env::var("HOME").ok()) {
        c.current_dir(dir);
    }
    start(c)
}

/// Run an AppImage, when the user has allowed it to run.
pub fn launch_appimage(file: &Path) -> Result<(), String> {
    if desktop::elf_file(file).is_none() {
        return Err("This isn't a Linux program, or it's damaged.".into());
    }
    if !desktop::executable(file) {
        return Err("It isn't allowed to run as a program yet. If you trust it, allow that in its Properties (or run chmod +x on it), then launch it.".into());
    }
    let mut c = Command::new(file);
    if let Some(dir) = file.parent() {
        c.current_dir(dir);
    }
    start(c)
}

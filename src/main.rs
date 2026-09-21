use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

#[derive(Parser)]
#[command(name = "px", bin_name = "px", version, about = "tiny windows helpers")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Delete files/folders (parallel)
    Rm { paths: Vec<PathBuf> },
    /// Uninstall apps by name
    Un { apps: Vec<String> },
    /// Copy file content to clipboard
    Cp { file: PathBuf },
    /// List installed apps with size
    Apps,
    /// List active ports and services
    Ports,
    /// Kill process on a port
    Kill { port: u16 },
    /// Update px to latest version
    Update,
    /// Uninstall px itself
    Destroy,
}

fn main() -> ExitCode {
    // Clean errors only: no panic dump.
    std::panic::set_hook(Box::new(|_| {
        eprintln!("px: something went wrong");
    }));

    let cli = Cli::parse();
    let err = match cli.command {
        Cmd::Rm { paths } => rm(&paths),
        Cmd::Un { apps } => un(&apps),
        Cmd::Cp { file } => cp(&file),
        Cmd::Apps => apps(),
        Cmd::Ports => ports(),
        Cmd::Kill { port } => kill_port(port),
        Cmd::Update => update(),
        Cmd::Destroy => destroy(),
    };

    match err {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("px: {msg}");
            ExitCode::FAILURE
        }
    }
}

// ---------- rm ----------

fn rm(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("rm: give at least one path".into());
    }

    let mut failed = 0;
    std::thread::scope(|s| {
        let mut jobs = Vec::new();
        for p in paths {
            jobs.push(s.spawn(|| delete_one(p)));
        }
        for (p, r) in paths.iter().zip(jobs) {
            if let Err(msg) = r.join().unwrap_or(Err("rm: thread failed".into())) {
                eprintln!("px: {}: {}", p.display(), msg);
                failed += 1;
            }
        }
    });

    if failed > 0 {
        return Err(format!("rm: {failed} path(s) failed"));
    }
    Ok(())
}

fn delete_one(path: &PathBuf) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|_| "not found or no permission".to_string())?;
    if meta.is_dir() && !meta.is_symlink() {
        std::fs::remove_dir_all(path).map_err(|e| clean_io(e))?;
    } else {
        std::fs::remove_file(path).map_err(|e| clean_io(e))?;
    }
    Ok(())
}

// ---------- cp ----------

fn cp(file: &PathBuf) -> Result<(), String> {
    let bytes = std::fs::read(file).map_err(|e| format!("cp: {}: {}", file.display(), clean_io(e)))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("cp: {}: not a text file", file.display()))?;
    clipboard_win::set_clipboard(clipboard_win::formats::Unicode, &text)
        .map_err(|_| "cp: can't open clipboard".to_string())?;
    Ok(())
}

// ---------- un ----------

fn un(apps: &[String]) -> Result<(), String> {
    // Pasted names often carry stray spaces.
    let apps: Vec<&str> = apps
        .iter()
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .collect();
    if apps.is_empty() {
        return Err("un: give at least one app name".into());
    }
    // Multi-word names typed without quotes arrive split up.
    // If the joined args exactly match one app, uninstall just that once.
    if apps.len() > 1 {
        let joined = apps.join(" ");
        if let Some(cmd) = find_exact_uninstall_cmd(&joined) {
            run_cmd(&cmd).map_err(|e| format!("un: {joined}: {e}"))?;
            return Ok(());
        }
    }
    // Sequential: MSI doesn't like parallel uninstalls.
    for app in apps {
        uninstall_one(app)?;
    }
    Ok(())
}

fn uninstall_one(name: &str) -> Result<(), String> {
    if let Some(cmd) = find_uninstall_cmd(name) {
        run_cmd(&cmd).map_err(|e| format!("un: {name}: {e}"))?;
        return Ok(());
    }

    // Fallback: winget (missing on some machines).
    match Command::new("winget")
        .args([
            "uninstall",
            "--exact",
            "--silent",
            "--accept-source-agreements",
            "--accept-package-agreements",
            name,
        ])
        .status()
    {
        Err(_) => Err(format!(
            "un: '{name}': not found in registry, and winget is not available"
        )),
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(format!("un: '{name}': not found")),
    }
}

fn find_uninstall_cmd(name: &str) -> Option<String> {
    lookup_uninstall(name, false)
}

fn find_exact_uninstall_cmd(name: &str) -> Option<String> {
    lookup_uninstall(name, true)
}

fn lookup_uninstall(name: &str, exact_only: bool) -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let roots = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];

    let want = name.to_lowercase();
    let mut contains_match: Option<String> = None;

    for (hkey, path) in roots {
        // A bad root must not abort the remaining roots.
        let base = match RegKey::predef(hkey).open_subkey(path) {
            Ok(k) => k,
            Err(_) => continue,
        };
        for sub in base.enum_keys().flatten() {
            // A bad entry must not abort the whole search.
            let key = match base.open_subkey(&sub) {
                Ok(k) => k,
                Err(_) => continue,
            };
            let display: String = match key.get_value("DisplayName") {
                Ok(n) => n,
                Err(_) => continue,
            };
            let cmd: Option<String> = key
                .get_value("QuietUninstallString")
                .or_else(|_| key.get_value("UninstallString"))
                .ok();
            let cmd = match cmd {
                Some(c) if !c.trim().is_empty() => c,
                _ => continue,
            };
            if display.to_lowercase() == want {
                return Some(cmd);
            }
            if !exact_only && contains_match.is_none() && display.to_lowercase().contains(&want) {
                contains_match = Some(cmd);
            }
        }
    }
    contains_match
}

fn run_cmd(line: &str) -> Result<(), String> {
    let line = msi_to_uninstall(line);
    let (program, args) = split_cmd(&line);
    if program.is_empty() {
        return Err("uninstall command failed".into());
    }
    // Batch files can't run directly, they need cmd.
    let lower = program.to_lowercase();
    let status = if lower.ends_with(".bat") || lower.ends_with(".cmd") {
        Command::new("cmd")
            .args(["/S", "/C", &format!("\"{line}\"")])
            .status()
    } else {
        Command::new(&program).args(&args).status()
    }
    .map_err(|e| clean_io(e))?;
    if status.success() {
        Ok(())
    } else {
        Err("uninstall command failed".into())
    }
}

// Split `"C:\a b\x.exe" /SILENT` into program + args.
fn split_cmd(line: &str) -> (String, Vec<String>) {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (rest[..end].to_string(), split_args(&rest[end + 1..]));
        }
    }
    let mut parts = split_args(line);
    if parts.is_empty() {
        return (String::new(), Vec::new());
    }
    let program = parts.remove(0);
    (program, parts)
}

fn split_args(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has = false;
    for c in s.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has {
                    args.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        args.push(cur);
    }
    args
}

// Registry often records `MsiExec.exe /I{GUID}` (repair mode).
// Rewrite to `/X{GUID}` so it actually uninstalls.
fn msi_to_uninstall(cmd: &str) -> String {
    if !cmd.to_lowercase().contains("msiexec") {
        return cmd.to_string();
    }
    let (start, end) = match (cmd.find('{'), cmd.rfind('}')) {
        (Some(s), Some(e)) if e > s => (s, e),
        _ => return cmd.to_string(),
    };
    let guid = &cmd[start..=end];
    let tail = cmd[end + 1..].trim();
    if tail.is_empty() {
        format!("MsiExec.exe /X{guid}")
    } else {
        format!("MsiExec.exe /X{guid} {tail}")
    }
}

// ---------- ports ----------

fn ports() -> Result<(), String> {
    let output = Command::new("netstat")
        .args(["-ano"])
        .output()
        .map_err(|_| "can't run netstat")?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Collect unique (port, proto, pid) for LISTENING entries.
    let mut seen = std::collections::HashSet::new();
    let mut entries: Vec<(u16, String, String)> = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        let is_tcp = line.starts_with("TCP");
        let is_udp = line.starts_with("UDP");
        if !is_tcp && !is_udp {
            continue;
        }
        // TCP lines must say LISTENING; UDP always counts.
        if is_tcp && !line.contains("LISTENING") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let proto = parts[0].to_uppercase();
        let addr = parts[1];
        let pid = parts[parts.len() - 1].to_string();
        let port = match addr.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) {
            Some(p) => p,
            None => continue,
        };
        if seen.insert((port, proto.clone(), pid.clone())) {
            entries.push((port, proto, pid));
        }
    }

    if entries.is_empty() {
        return Err("ports: nothing listening".into());
    }

    // Resolve PID → process name via tasklist once.
    let tasklist = Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let mut pid_name: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for line in tasklist.lines() {
        // "svchost.exe","1234","Services","0","1,234 K"  — split on ","
        let trimmed = line.trim();
        let fields: Vec<&str> = trimmed.split("\",\"").collect();
        if fields.len() >= 2 {
            let name = fields[0].trim_matches('"');
            let pid = fields[1].trim_matches('"');
            if !name.is_empty() && !pid.is_empty() {
                pid_name.insert(pid.to_string(), name.to_string());
            }
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let port_w = entries.iter().map(|(p, _, _)| p.to_string().len()).max().unwrap_or(5).max(5);
    println!("{:<port_w$}  PROTO  PID      PROCESS", "PORT", port_w = port_w);
    for (port, proto, pid) in &entries {
        let name = pid_name.get(pid.as_str()).map(|s| s.as_str()).unwrap_or("-");
        println!("{:<port_w$}  {:<6} {:<8} {}", port, proto, pid, name, port_w = port_w);
    }
    Ok(())
}

fn kill_port(port: u16) -> Result<(), String> {
    let output = Command::new("netstat")
        .args(["-ano"])
        .output()
        .map_err(|_| "can't run netstat")?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let pids: Vec<&str> = stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with("TCP") && !line.starts_with("UDP") {
                return None;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                return None;
            }
            let addr = parts[1];
            let pid = parts[parts.len() - 1];
            let p = addr.rsplit(':').next()?;
            if p.parse::<u16>().ok()? == port {
                Some(pid)
            } else {
                None
            }
        })
        .collect();

    if pids.is_empty() {
        return Err(format!("kill: nothing on port {port}"));
    }

    let mut killed = 0;
    for pid in &pids {
        let status = Command::new("taskkill")
            .args(["/F", "/PID", pid])
            .status();
        if let Ok(s) = status {
            if s.success() {
                killed += 1;
            }
        }
    }

    if killed == 0 {
        return Err(format!("kill: couldn't kill process on port {port}"));
    }
    Ok(())
}

// ---------- apps ----------

fn apps() -> Result<(), String> {
    let mut list = installed_apps();
    if list.is_empty() {
        return Err("apps: no apps found".into());
    }

    // Biggest first, unknown size last.
    list.sort_by(|a, b| match (a.1, b.1) {
        (Some(x), Some(y)) => y.cmp(&x).then(a.0.cmp(&b.0)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });

    let width = list
        .iter()
        .map(|(n, _)| n.chars().count().min(50))
        .max()
        .unwrap_or(4)
        .max(4);
    println!("{:<width$}  SIZE", "NAME", width = width);
    for (name, size) in list {
        let short: String = name.chars().take(50).collect();
        let size = match size {
            Some(kb) => human_size(kb * 1024),
            None => "-".into(),
        };
        println!("{:<width$}  {}", short, size, width = width);
    }
    Ok(())
}

fn installed_apps() -> Vec<(String, Option<u64>)> {
    use std::collections::HashSet;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let roots = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (hkey, path) in roots {
        let base = match RegKey::predef(hkey).open_subkey(path) {
            Ok(k) => k,
            Err(_) => continue,
        };
        for sub in base.enum_keys().flatten() {
            let key = match base.open_subkey(&sub) {
                Ok(k) => k,
                Err(_) => continue,
            };
            let name: String = match key.get_value::<String, _>("DisplayName") {
                Ok(n) if !n.trim().is_empty() => n,
                _ => continue,
            };
            // Skip system components / updates.
            let sys: u32 = key.get_value("SystemComponent").unwrap_or(0);
            if sys == 1 {
                continue;
            }
            if !seen.insert(name.to_lowercase()) {
                continue;
            }
            let size: Option<u64> = key.get_value::<u32, _>("EstimatedSize").ok().map(u64::from);
            out.push((name, size));
        }
    }
    out
}

fn human_size(bytes: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    const KB: u64 = 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

// ---------- small helper ----------

// ---------- update ----------

const REPO: &str = "Muhammad-Owais-Warsi/px";

fn update() -> Result<(), String> {
    let current = env!("CARGO_PKG_VERSION");
    println!("px {current}");

    let mut resp = ureq::get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("User-Agent", "px")
        .call()
        .map_err(|_| "update: can't reach github")?;
    let body = resp.body_mut();
    let json_str = body.read_to_string().map_err(|_| "update: bad response")?;
    let body: serde_json::Value =
        serde_json::from_str(&json_str).map_err(|_| "update: bad response")?;
    let tag = body["tag_name"]
        .as_str()
        .ok_or("update: no tag in response")?;
    let version = tag.trim_start_matches('v');

    if version == current {
        println!("already up to date");
        return Ok(());
    }

    println!("updating to {version}...");

    let asset = "px-windows-x86_64.zip";
    let url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
    let tmp = std::env::temp_dir().join(format!("px-update-{version}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|_| "update: can't create temp dir")?;
    let zip = tmp.join(asset);

    let mut resp = ureq::get(&url)
        .header("User-Agent", "px")
        .call()
        .map_err(|_| "update: download failed")?;
    let bytes = resp
        .body_mut()
        .read_to_vec()
        .map_err(|_| "update: download failed")?;
    std::fs::write(&zip, bytes).map_err(|_| "update: can't write zip")?;

    let zip_file = std::fs::File::open(&zip).map_err(|_| "update: can't open zip")?;
    let mut archive = zip::ZipArchive::new(zip_file).map_err(|_| "update: bad zip")?;
    archive
        .extract(tmp.join("bin"))
        .map_err(|_| "update: can't extract zip")?;

    let new_exe = tmp.join("bin").join("px.exe");
    if !new_exe.exists() {
        return Err("update: px.exe not found in archive".into());
    }

    let me = std::env::current_exe().map_err(|_| "update: can't find own path")?;
    let me_s = me.to_string_lossy();
    let new_s = new_exe.to_string_lossy();
    let cleanup = new_exe.parent().unwrap().to_string_lossy();

    let bat_content = format!(
        "@echo off\r\n\
         ping -n 4 127.0.0.1 >nul\r\n\
         copy /Y \"{new_s}\" \"{me_s}\" >nul 2>&1\r\n\
         if errorlevel 1 (\r\n\
           ping -n 4 127.0.0.1 >nul\r\n\
           copy /Y \"{new_s}\" \"{me_s}\" >nul 2>&1\r\n\
         )\r\n\
         start \"\" \"{me_s}\"\r\n\
         del /f /q \"{cleanup}\\px.exe\" >nul 2>&1\r\n\
         del /f /q \"%~f0\" >nul 2>&1\r\n",
    );
    let bat = std::env::temp_dir().join(format!("px_update_{}.bat", std::process::id()));
    std::fs::write(&bat, bat_content).map_err(|_| "update: can't write script")?;
    let bat_s = bat.to_string_lossy();

    Command::new("cmd")
        .args(["/C", "start", "", "/min", "cmd", "/C", &bat_s])
        .spawn()
        .map_err(|_| "update: can't launch updater")?;

    println!("updated to {version}");
    std::process::exit(0);
}

// ---------- destroy ----------

fn destroy() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|_| "can't find own path")?;
    let dir = exe.parent().ok_or("can't find own directory")?;

    // Remove from PATH if installed to %LOCALAPPDATA%\px\bin.
    let local_px = std::env::var("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .map(|p| p.join("px").join("bin"))
        .ok();

    if let Some(ref px_dir) = local_px {
        if dir == px_dir.as_path() {
            let user_path = std::env::var("Path").unwrap_or_default();
            let new_path: Vec<&str> = user_path
                .split(';')
                .filter(|p| {
                    let norm = p.trim_end_matches('\\').to_lowercase();
                    norm != px_dir.to_string_lossy().to_lowercase()
                })
                .collect();
            std::process::Command::new("reg")
                .args([
                    "add",
                    "HKCU\\Environment",
                    "/v",
                    "Path",
                    "/t",
                    "REG_EXPAND_SZ",
                    "/d",
                    &new_path.join(";"),
                    "/f",
                ])
                .status()
                .map_err(|_| "can't update PATH")?;
        }
    }
    // Can't delete a running exe. Write a temp script, run it detached, then exit.
    let path_str = exe.to_string_lossy().to_string();
    let bat = format!(
        "@echo off\r\nping -n 2 127.0.0.1 >nul\r\ndel /f /q \"{}\"\r\n",
        path_str
    );
    let bat_path = std::env::temp_dir().join(format!("px_cleanup_{}.bat", std::process::id()));
    std::fs::write(&bat_path, bat).map_err(|_| "can't write cleanup script")?;
    Command::new("cmd")
        .args(["/C", "start", "", "/min", "cmd", "/C", &bat_path.to_string_lossy()])
        .spawn()
        .map_err(|_| "can't schedule deletion")?;

    println!("px uninstalled");
    std::process::exit(0);
}

fn clean_io(e: std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound => "not found".into(),
        PermissionDenied => "permission denied".into(),
        AlreadyExists => "already exists".into(),
        _ => "failed".into(),
    }
}

//! Local engine health checks and privileged fix commands.

use crate::engines::{docker, incus};
use crate::models::{CheckId, CheckLevel, DiagCheck, DiagReport, HostId};
use crate::prefs::RemoteProfile;
use std::path::Path;

pub fn user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "ken".into())
}

fn in_session(name: &str) -> bool {
    groups_of(&["id", "-nG"]).iter().any(|g| g == name)
}

fn in_account(name: &str) -> bool {
    let user = user();
    groups_of(&["id", "-nG", &user]).iter().any(|g| g == name)
}

fn groups_of(args: &[&str]) -> Vec<String> {
    let Ok(out) = std::process::Command::new(args[0]).args(&args[1..]).output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout).split_whitespace().map(str::to_string).collect()
}

fn group_check(id: CheckId, group: &str) -> DiagCheck {
    if in_session(group) {
        return ok(id, format!("This session has the {group} group."));
    }
    if in_account(group) {
        return DiagCheck {
            id,
            level: CheckLevel::Fail,
            detail: format!(
                "The account is in {group}, but this login session is not. Log out of the desktop and back in. A new terminal is not enough."
            ),
            command: None,
        };
    }
    fail(id, format!("This user is not in the {group} group."))
}

fn on_path(bin: &str) -> bool {
    which(bin).is_some()
}

fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let p = dir.join(bin);
        p.is_file().then_some(p)
    })
}

pub fn command_for(id: CheckId) -> String {
    let user = user();
    match id {
        CheckId::DockerPkg => "pacman -S --needed docker docker-compose".into(),
        CheckId::DockerGroup => format!("usermod -aG docker {user}"),
        CheckId::DockerDaemon => "systemctl enable --now docker.socket docker.service".into(),
        CheckId::IncusPkg => "pacman -S --needed incus".into(),
        CheckId::IncusGroup => format!("usermod -aG incus-admin,incus {user}"),
        CheckId::IncusDaemon => "systemctl enable --now lxcfs.service incus.socket incus.service".into(),
        CheckId::IncusInit => "incus admin init --auto".into(),
        CheckId::Vte => "pacman -S --needed vte4".into(),
    }
}

pub async fn report(remote: &RemoteProfile) -> DiagReport {
    let mut checks = Vec::new();
    push(&mut checks, docker_pkg());
    push(&mut checks, docker_group());
    push(&mut checks, docker_daemon().await);
    push(&mut checks, incus_pkg());
    push(&mut checks, incus_group());
    push(&mut checks, incus_daemon().await);
    push(&mut checks, incus_init(remote).await);
    push(&mut checks, vte());
    DiagReport { checks }
}

fn push(out: &mut Vec<DiagCheck>, check: DiagCheck) {
    out.push(check);
}

fn docker_pkg() -> DiagCheck {
    if on_path("docker") {
        ok(CheckId::DockerPkg, "docker is on PATH.")
    } else {
        fail(CheckId::DockerPkg, "The docker package is not installed.")
    }
}

fn docker_group() -> DiagCheck {
    group_check(CheckId::DockerGroup, "docker")
}

async fn docker_daemon() -> DiagCheck {
    match docker::probe_local().await.0.state {
        crate::models::ConnState::Connected { detail } => ok(CheckId::DockerDaemon, detail),
        crate::models::ConnState::Permission { reason } => fail(CheckId::DockerDaemon, reason),
        crate::models::ConnState::Unavailable { reason } => fail(CheckId::DockerDaemon, reason),
        _ => fail(CheckId::DockerDaemon, "Docker is not available on this computer."),
    }
}

fn incus_pkg() -> DiagCheck {
    if on_path("incus") {
        ok(CheckId::IncusPkg, "incus is on PATH.")
    } else {
        fail(CheckId::IncusPkg, "The incus package is not installed.")
    }
}

fn incus_group() -> DiagCheck {
    group_check(CheckId::IncusGroup, "incus-admin")
}

async fn incus_daemon() -> DiagCheck {
    match incus::probe_local().await.0.state {
        crate::models::ConnState::Connected { detail } => ok(CheckId::IncusDaemon, detail),
        crate::models::ConnState::Permission { reason } => fail(CheckId::IncusDaemon, reason),
        crate::models::ConnState::Unavailable { reason } => fail(CheckId::IncusDaemon, reason),
        _ => fail(CheckId::IncusDaemon, "Incus is not available on this computer."),
    }
}

async fn incus_init(remote: &RemoteProfile) -> DiagCheck {
    if !in_session("incus-admin") {
        return DiagCheck {
            id: CheckId::IncusInit,
            level: CheckLevel::Fail,
            detail: "Storage cannot be checked until this session has the incus-admin group.".into(),
            command: None,
        };
    }
    if incus::storage_ready(HostId::Local, remote).await {
        ok(CheckId::IncusInit, "Incus has a storage pool.")
    } else {
        fail(CheckId::IncusInit, "Incus has no storage pool. Initialize it.")
    }
}

fn vte() -> DiagCheck {
    let pc = Path::new("/usr/lib/pkgconfig/vte-2.91-gtk4.pc");
    if pc.exists()
        || which("pkg-config").is_some_and(|_| {
            std::process::Command::new("pkg-config")
                .args(["--exists", "vte-2.91-gtk4"])
                .status()
                .ok()
                .is_some_and(|s| s.success())
        })
    {
        ok(CheckId::Vte, "vte-2.91-gtk4 is present.")
    } else {
        fail(CheckId::Vte, "The vte4 package is not installed.")
    }
}

fn ok(id: CheckId, detail: impl Into<String>) -> DiagCheck {
    DiagCheck { id, level: CheckLevel::Ok, detail: detail.into(), command: None }
}

fn fail(id: CheckId, detail: impl Into<String>) -> DiagCheck {
    DiagCheck { id, level: CheckLevel::Fail, detail: detail.into(), command: Some(command_for(id)) }
}

pub fn run_fix(id: CheckId) -> Result<String, String> {
    let cmd = command_for(id);
    let mut parts = cmd.split_whitespace();
    let bin = parts.next().ok_or_else(|| "empty command".to_string())?;
    let args: Vec<&str> = parts.collect();
    let pkexec = which("pkexec");
    let mut command = if let Some(pk) = pkexec {
        let mut c = std::process::Command::new(pk);
        c.arg(bin);
        c.args(&args);
        c
    } else {
        return Err(format!("pkexec is missing. Run as root:\n{cmd}"));
    };
    match command.status() {
        Ok(st) if st.success() => {
            let extra = matches!(id, CheckId::DockerGroup | CheckId::IncusGroup)
                .then_some(" Log out and back in for group membership.")
                .unwrap_or("");
            Ok(format!("Finished: {cmd}.{extra}"))
        }
        Ok(st) => Err(format!("Command did not succeed (status {st}). Copy and run:\n{cmd}")),
        Err(err) => Err(format!("{err}. Copy and run:\n{cmd}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_group_command_names_the_user() {
        let cmd = command_for(CheckId::DockerGroup);
        assert!(cmd.contains("usermod"));
        assert!(cmd.contains("docker"));
    }

    #[test]
    fn stale_session_does_not_offer_usermod() {
        let session = groups_of(&["id", "-nG"]);
        let account = groups_of(&["id", "-nG", &user()]);
        if account.iter().any(|g| g == "incus-admin") && !session.iter().any(|g| g == "incus-admin") {
            let check = group_check(CheckId::IncusGroup, "incus-admin");
            assert!(check.command.is_none());
            assert!(check.detail.contains("login session"));
        }
    }
}

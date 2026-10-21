//! Every page in the sidebar, in order.

use crate::widgets::Page;

pub mod connections;
pub mod diagnose;
pub mod images;
pub mod networks;
pub mod overview;
pub mod storage;
pub mod session;
pub mod settings;
pub mod stacks;
pub mod workloads;

#[derive(Clone)]
pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub group: &'static str,
    pub description: &'static str,
    pub build: fn(&Page),
    pub fill: bool,
    pub nav: bool,
    pub host_filter: bool,
}

pub fn all() -> Vec<Section> {
    let s = |id, title, icon, group, description, build, fill, host_filter| Section {
        id,
        title,
        icon,
        group,
        description,
        build,
        fill,
        nav: true,
        host_filter,
    };
    vec![
        s(
            "host-all",
            "All Hosts",
            "network-workgroup-symbolic",
            "Hosts",
            "Workloads on this computer and the remote server.",
            overview::build,
            false,
            true,
        ),
        s("host-local", "Local", "computer-symbolic", "Hosts", "This computer.", overview::build, false, true),
        s(
            "host-remote",
            "Remote",
            "network-server-symbolic",
            "Hosts",
            "The configured remote server.",
            overview::build,
            false,
            true,
        ),
        s(
            "overview",
            "Overview",
            "view-grid-symbolic",
            "Manage",
            "Connection status and workload counts.",
            overview::build,
            false,
            false,
        ),
        s(
            "containers",
            "Containers",
            "application-x-executable-symbolic",
            "Manage",
            "Docker and Incus containers.",
            workloads::build_containers,
            true,
            false,
        ),
        s("vms", "Virtual Machines", "computer-symbolic", "Manage", "Incus virtual machines.", workloads::build_vms, true, false),
        s("stacks", "Stacks", "view-list-symbolic", "Manage", "Docker Compose projects.", stacks::build, true, false),
        s("images", "Images", "media-optical-symbolic", "Manage", "Container and VM images.", images::build, true, false),
        s(
            "networks",
            "Networks",
            "network-wired-symbolic",
            "Manage",
            "Docker and Incus networks.",
            networks::build,
            true,
            false,
        ),
        s(
            "storage",
            "Storage",
            "drive-harddisk-symbolic",
            "Manage",
            "Volumes and storage pools.",
            storage::build,
            true,
            false,
        ),
        s(
            "connections",
            "Connections",
            "preferences-system-network-symbolic",
            "System",
            "Local engines and the remote server.",
            connections::build,
            false,
            false,
        ),
    ]
}

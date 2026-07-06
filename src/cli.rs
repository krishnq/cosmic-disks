// SPDX-License-Identifier: GPL-3.0

use crate::actions::disks::{scan_drives, BlockDevice, Drive};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "cosmic-disks",
    version,
    about = "COSMIC disk and partition manager"
)]
pub struct Cli {
    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Text, global = true)]
    pub format: Format,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// List all physical disks
    ListDisks,
    /// List all volumes (partitions)
    ListVolumes,
    /// Mount a volume
    ///
    /// NOTE: mounts via udisks2's managed path (typically /run/media/$USER/<label>).
    /// Path-based mounting (with fstab write) is only available through the GUI.
    Mount {
        /// Block device path (e.g. /dev/sda1)
        device: String,
        /// Comma-separated mount options (e.g. ro,noatime)
        #[arg(long, default_value = "")]
        options: String,
    },
    /// Unmount a volume
    Unmount {
        /// Block device path (e.g. /dev/sda1)
        device: String,
    },
    /// Format a volume (DESTROYS all data on the device)
    Format {
        /// Block device path (e.g. /dev/sda1)
        device: String,
        /// Filesystem type (e.g. ext4, vfat)
        #[arg(long, default_value = "ext4")]
        fs: String,
        /// Volume label
        #[arg(long, default_value = "")]
        label: String,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
    },
}

#[derive(ValueEnum, Clone)]
pub enum Format {
    Text,
    Json,
}

pub async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cli.command {
        Command::ListDisks => {
            let drives = scan_drives().await?;
            match cli.format {
                Format::Json => println!("{}", serde_json::to_string_pretty(&drives)?),
                Format::Text => print_drives(&drives),
            }
        }
        Command::ListVolumes => {
            let volumes: Vec<BlockDevice> = scan_drives()
                .await?
                .into_iter()
                .flat_map(|d| d.partitions)
                .collect();
            match cli.format {
                Format::Json => println!("{}", serde_json::to_string_pretty(&volumes)?),
                Format::Text => print_volumes(&volumes),
            }
        }
        Command::Mount { device, options } => {
            let msg = crate::actions::volumes::mount_default(&device, &options).await?;
            println!("{msg}");
        }
        Command::Unmount { device } => {
            let msg = crate::actions::volumes::unmount(&device).await?;
            println!("{msg}");
        }
        Command::Format {
            device,
            fs,
            label,
            yes,
        } => {
            if !yes && !confirm_format(&device, &fs)? {
                println!("Aborted.");
                return Ok(());
            }
            let msg = crate::actions::volumes::format(&device, &fs, &label).await?;
            println!("{msg}");
        }
    }
    Ok(())
}

/// Interactive y/N prompt before a destructive format.  Defaults to "no" —
/// including when stdin is not a terminal (e.g. piped input reaching EOF).
fn confirm_format(device: &str, fs: &str) -> std::io::Result<bool> {
    use std::io::Write;

    print!("This will DESTROY all data on {device} (formatting as {fs}). Continue? [y/N] ");
    std::io::stdout().flush()?;

    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "YES"))
}

fn print_drives(drives: &[Drive]) {
    for drive in drives {
        println!("{}", drive.display_name());
        if drive.partitions.is_empty() {
            println!("  (no partitions)");
        } else {
            for p in &drive.partitions {
                let mounts = if p.mount_points.is_empty() {
                    "-".to_string()
                } else {
                    p.mount_points.join(", ")
                };
                let fs = if p.fs_type.is_empty() {
                    "-"
                } else {
                    &p.fs_type
                };
                println!(
                    "  {:<20} {:>10}  {:<8}  {}",
                    p.device,
                    p.display_size(),
                    fs,
                    mounts
                );
            }
        }
        println!();
    }
}

fn print_volumes(volumes: &[BlockDevice]) {
    println!(
        "{:<20} {:<20} {:>10}  {:<10}  MOUNTPOINTS",
        "DEVICE", "LABEL", "SIZE", "TYPE"
    );
    println!("{}", "-".repeat(80));
    for v in volumes {
        let label = if v.label.is_empty() { "-" } else { &v.label };
        let fs = if v.fs_type.is_empty() {
            "-"
        } else {
            &v.fs_type
        };
        let mounts = if v.mount_points.is_empty() {
            "-".to_string()
        } else {
            v.mount_points.join(", ")
        };
        println!(
            "{:<20} {:<20} {:>10}  {:<10}  {}",
            v.device,
            label,
            v.display_size(),
            fs,
            mounts
        );
    }
}

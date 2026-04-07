use anyhow::{Context, Result};
use colored::Colorize;
use fpbx_core::bundle::{default_backup_dir, fmt_bytes, list_bundles as core_list_bundles, open_bundle};
use std::{env, fs, path::PathBuf};

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    match args.get(1) {
        Some(path) => show_bundle(PathBuf::from(path)),
        None => list_bundles(),
    }
}

fn list_bundles() -> Result<()> {
    let dir = default_backup_dir();
    if !dir.exists() {
        println!("{}", "No backup directory found.".yellow());
        return Ok(());
    }

    let entries = core_list_bundles(&dir).context("read backup dir")?;

    if entries.is_empty() {
        println!("{}", "No .fpbx bundles found.".yellow());
        return Ok(());
    }

    println!("\n{}/", dir.display().to_string().dimmed());
    println!();

    let name_width = entries
        .iter()
        .map(|(p, _)| p.file_name().and_then(|n| n.to_str()).unwrap_or("?").len())
        .max()
        .unwrap_or(0);

    for (path, m) in &entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        let date = m.created_at.format("%Y-%m-%d %H:%M").to_string();
        let size = fmt_bytes(m.files_tar_bytes + m.db_dump_bytes);
        println!(
            "   {:<width$}   {:<20}   {}   {}",
            name.cyan(),
            m.domain.domain_name.white().bold(),
            date.dimmed(),
            size.yellow(),
            width = name_width
        );
    }

    println!();
    println!("{}", format!("{} bundle(s)", entries.len()).dimmed());
    Ok(())
}

fn show_bundle(path: PathBuf) -> Result<()> {
    let staging = std::env::temp_dir().join("fpbx-info-staging");
    let manifest =
        open_bundle(&path, &staging).with_context(|| format!("failed to open {:?}", path))?;

    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");

    println!();
    println!("{:<14}{}", "Bundle:".dimmed(), name.cyan().bold());
    println!(
        "{:<14}{}",
        "Created:".dimmed(),
        manifest
            .created_at
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string()
            .white()
    );
    println!("{:<14}{}", "Source:".dimmed(), manifest.source_host.white());
    println!();
    println!(
        "{:<14}{} ({})",
        "Domain:".dimmed(),
        manifest.domain.domain_name.white().bold(),
        manifest
            .domain
            .domain_description
            .as_deref()
            .unwrap_or("no description")
            .dimmed(),
    );
    println!(
        "{:<14}{}",
        "UUID:".dimmed(),
        manifest.domain.domain_uuid.dimmed()
    );
    println!(
        "{:<14}{}",
        "Enabled:".dimmed(),
        if manifest.domain.domain_enabled {
            "yes".green().to_string()
        } else {
            "no".red().to_string()
        }
    );
    println!();

    // Checksum is verified inside open_bundle - if we got here it passed.
    println!("{:<14}{}", "Checksum:".dimmed(), "✓ valid".green().bold());
    println!();

    if manifest.table_counts.0.is_empty() {
        println!("{}", "Table counts: (none recorded)".yellow());
    } else {
        println!("{}", "Table counts:".dimmed());
        let max_len = manifest
            .table_counts
            .0
            .iter()
            .map(|(t, _)| t.len())
            .max()
            .unwrap_or(0);
        for (table, count) in &manifest.table_counts.0 {
            println!(
                "   {:<width$}  {}",
                table.dimmed(),
                count.to_string().white(),
                width = max_len
            );
        }
        println!();
        println!(
            "   {:<width$}  {}",
            "total rows".dimmed(),
            manifest
                .table_counts
                .total_rows()
                .to_string()
                .white()
                .bold(),
            width = max_len
        );
    }

    println!();
    if manifest.file_paths.is_empty() {
        println!("{}", "Files: (none)".yellow());
    } else {
        println!("{}", "Files:".dimmed());
        for p in &manifest.file_paths {
            println!("   {}", p.dimmed());
        }
    }

    println!();
    println!("{}", "Sizes:".dimmed());
    println!(
        "   {:<14}{}",
        "DB dump:".dimmed(),
        fmt_bytes(manifest.db_dump_bytes).yellow()
    );
    println!(
        "   {:<14}{}",
        "Files:".dimmed(),
        fmt_bytes(manifest.files_tar_bytes).yellow()
    );
    println!(
        "   {:<14}{}",
        "Total:".dimmed(),
        fmt_bytes(manifest.db_dump_bytes + manifest.files_tar_bytes)
            .yellow()
            .bold()
    );
    println!();

    let _ = fs::remove_dir_all(&staging);
    Ok(())
}

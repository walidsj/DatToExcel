#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod excel;
mod table;
mod value;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // Mode baris perintah: dat-to-excel <file...> [--out <folder>]
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(cli(&args));
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DAT to Excel")
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([900.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "DAT to Excel",
        options,
        Box::new(|cc| {
            app::apply_theme(&cc.egui_ctx);
            Ok(Box::new(app::App::default()))
        }),
    )
}

fn cli(args: &[String]) -> i32 {
    use std::path::PathBuf;

    let mut out: Option<PathBuf> = None;
    let mut inputs = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--out" {
            out = iter.next().map(PathBuf::from);
        } else {
            inputs.push(PathBuf::from(arg));
        }
    }

    let mut failed = 0;
    for input in inputs {
        let result = table::read_table(&input).and_then(|table| {
            let profiles = value::profile_table(&table);
            let dir = out.clone().unwrap_or_else(|| input.parent().map(Into::into).unwrap_or_default());
            let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("hasil");
            let path = dir.join(format!("{stem}.xlsx"));

            for (i, p) in profiles.iter().enumerate() {
                let resolved = excel::resolve(excel::ColumnType::Auto, p);
                println!("  kolom {}: {}", i + 1, resolved.label());
            }

            excel::convert(&table, &profiles, &[], &path, &mut |_| {}).map(|stats| (path, stats))
        });

        match result {
            Ok((path, stats)) => {
                println!(
                    "{} -> {} ({} baris, {} sheet, {} fallback teks)",
                    input.display(),
                    path.display(),
                    stats.rows,
                    stats.sheets,
                    stats.text_fallbacks
                );
                for example in stats.fallback_examples {
                    println!("    {example}");
                }
            }
            Err(error) => {
                failed += 1;
                eprintln!("{}: {error}", input.display());
            }
        }
    }

    i32::from(failed > 0)
}

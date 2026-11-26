//! Antarmuka aplikasi (egui).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Sense,
    Stroke, Ui, pos2, vec2,
};
use rfd::FileDialog;

use crate::excel::{self, CellValue, ColumnType, Resolved};
use crate::table::{Table, read_table};
use crate::value::{ColumnProfile, profile_table};

// ============================================================
// WARNA & TEMA
// ============================================================

mod palette {
    use eframe::egui::Color32;

    pub const BG: Color32 = Color32::from_rgb(0xF4, 0xF6, 0xFA);
    pub const CARD: Color32 = Color32::WHITE;
    pub const BORDER: Color32 = Color32::from_rgb(0xE3, 0xE7, 0xEF);
    pub const TEXT: Color32 = Color32::from_rgb(0x1E, 0x29, 0x3B);
    pub const MUTED: Color32 = Color32::from_rgb(0x64, 0x74, 0x8B);
    pub const ACCENT: Color32 = Color32::from_rgb(0x4F, 0x46, 0xE5);
    pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0xEE, 0xF2, 0xFF);
    pub const STRIPE: Color32 = Color32::from_rgb(0xF8, 0xFA, 0xFC);
    pub const SUCCESS: Color32 = Color32::from_rgb(0x05, 0x96, 0x69);
    pub const WARN: Color32 = Color32::from_rgb(0xB4, 0x53, 0x09);
    pub const WARN_SOFT: Color32 = Color32::from_rgb(0xFF, 0xF7, 0xED);
    pub const ERROR: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26);
}

pub fn apply_theme(ctx: &egui::Context) {
    use egui::{FontFamily, TextStyle};

    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::light();

    visuals.panel_fill = palette::BG;
    visuals.window_fill = palette::CARD;
    visuals.extreme_bg_color = palette::CARD;
    visuals.faint_bg_color = palette::STRIPE;
    visuals.override_text_color = Some(palette::TEXT);
    visuals.selection.bg_fill = palette::ACCENT_SOFT;
    visuals.selection.stroke = Stroke::new(1.0_f32, palette::ACCENT);
    visuals.hyperlink_color = palette::ACCENT;
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.window_stroke = Stroke::new(1.0_f32, palette::BORDER);

    let radius = CornerRadius::same(7);
    let w = &mut visuals.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, palette::BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, palette::TEXT);
    w.inactive.weak_bg_fill = Color32::from_rgb(0xF1, 0xF4, 0xF9);
    w.inactive.bg_fill = Color32::from_rgb(0xF1, 0xF4, 0xF9);
    w.inactive.bg_stroke = Stroke::new(1.0_f32, palette::BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, palette::TEXT);
    w.hovered.weak_bg_fill = Color32::from_rgb(0xE6, 0xEB, 0xF5);
    w.hovered.bg_fill = Color32::from_rgb(0xE6, 0xEB, 0xF5);
    w.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0xC7, 0xCF, 0xE0));
    w.hovered.fg_stroke = Stroke::new(1.0_f32, palette::TEXT);
    w.active.weak_bg_fill = Color32::from_rgb(0xDB, 0xE2, 0xF0);
    w.active.bg_fill = Color32::from_rgb(0xDB, 0xE2, 0xF0);
    w.active.fg_stroke = Stroke::new(1.0_f32, palette::TEXT);
    for v in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        v.corner_radius = radius;
        v.expansion = 0.0;
    }

    style.visuals = visuals;
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(12.0, 6.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.combo_width = 140.0;

    style.text_styles = [
        (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
    ]
    .into();

    ctx.set_style(style);
}

fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(palette::CARD)
        .stroke(Stroke::new(1.0_f32, palette::BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn section_title(ui: &mut Ui, title: &str, subtitle: Option<&str>) {
    ui.label(RichText::new(title).heading().strong());
    if let Some(subtitle) = subtitle {
        ui.label(RichText::new(subtitle).small().color(palette::MUTED));
    }
    ui.add_space(4.0);
}

fn badge(ui: &mut Ui, text: impl Into<String>, fg: Color32, bg: Color32) {
    Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).small().color(fg));
        });
}

fn primary_button(text: &str, width: f32) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(Color32::WHITE).strong())
        .fill(palette::ACCENT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(8))
        .min_size(vec2(width, 40.0))
}

fn format_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

fn format_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / (1 << 10) as f64),
        b => format!("{b} B"),
    }
}

// ============================================================
// MODEL
// ============================================================

pub struct DatFile {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub table: Table,
    pub profiles: Vec<ColumnProfile>,
    pub header: bool,
}

impl DatFile {
    fn load(path: PathBuf) -> Result<Self, String> {
        let table = read_table(&path)?;
        let profiles = profile_table(&table);
        let header = excel::has_header(&profiles);
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("tanpa-nama")
            .to_string();

        Ok(Self {
            path,
            name,
            size,
            table,
            profiles,
            header,
        })
    }

    fn data_rows(&self) -> usize {
        self.table.rows() - usize::from(self.header)
    }
}

/// Entri daftar file; `data` kosong selama file masih dibaca.
struct FileSlot {
    path: PathBuf,
    name: String,
    data: Option<Arc<DatFile>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Info,
    Success,
    Warn,
    Error,
}

struct LogEntry {
    time: String,
    level: Level,
    text: String,
}

enum Message {
    Loaded(Arc<DatFile>),
    LoadFailed { path: PathBuf, error: String },
    Log(Level, String),
    Progress(f32),
    Finished { ok: usize, failed: usize, seconds: f32 },
}

// ============================================================
// APLIKASI
// ============================================================

pub struct App {
    files: Vec<FileSlot>,
    selected: usize,
    column_types: Vec<ColumnType>,
    preview_rows: usize,
    output_dir: Option<PathBuf>,
    logs: Vec<LogEntry>,
    progress: f32,
    converting: bool,
    tx: Sender<Message>,
    rx: Receiver<Message>,
}

impl Default for App {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            files: Vec::new(),
            selected: 0,
            column_types: Vec::new(),
            preview_rows: 50,
            output_dir: None,
            logs: Vec::new(),
            progress: 0.0,
            converting: false,
            tx,
            rx,
        }
    }
}

impl App {
    fn loading(&self) -> usize {
        self.files.iter().filter(|slot| slot.data.is_none()).count()
    }

    fn busy(&self) -> bool {
        self.converting || self.loading() > 0
    }

    fn ready_files(&self) -> impl Iterator<Item = &Arc<DatFile>> {
        self.files.iter().filter_map(|slot| slot.data.as_ref())
    }

    fn log(&mut self, level: Level, text: impl Into<String>) {
        self.logs.push(LogEntry {
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            level,
            text: text.into(),
        });

        if self.logs.len() > 1000 {
            let excess = self.logs.len() - 1000;
            self.logs.drain(0..excess);
        }
    }

    #[cfg(test)]
    fn pick_files(&mut self) {}

    #[cfg(not(test))]
    fn pick_files(&mut self) {
        if let Some(paths) = FileDialog::new()
            .add_filter("Data teks", &["dat", "txt", "csv"])
            .add_filter("Semua file", &["*"])
            .set_title("Pilih file sumber")
            .pick_files()
        {
            self.load_files(paths);
        }
    }

    fn load_files(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            if path.is_dir() || self.files.iter().any(|slot| slot.path == path) {
                continue;
            }

            // Masuk ke daftar dulu; isi menyusul setelah selesai dibaca.
            self.files.push(FileSlot {
                name: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("tanpa-nama")
                    .to_string(),
                path: path.clone(),
                data: None,
            });

            let tx = self.tx.clone();

            thread::spawn(move || {
                let message = match DatFile::load(path.clone()) {
                    Ok(file) => Message::Loaded(Arc::new(file)),
                    Err(error) => Message::LoadFailed { path, error },
                };
                let _ = tx.send(message);
            });
        }
    }

    fn remove_file(&mut self, index: usize) {
        let removed = self.files.remove(index);

        if self.selected > index || self.selected >= self.files.len() {
            self.selected = self.selected.saturating_sub(1);
        }

        self.sync_column_types();
        let note = if removed.data.is_none() { " (pembacaan dibatalkan)" } else { "" };
        self.log(Level::Info, format!("File dihapus: {}{note}", removed.name));
    }

    fn sync_column_types(&mut self) {
        let columns = self.ready_files().map(|f| f.table.columns()).max().unwrap_or(0);
        self.column_types.resize(columns, ColumnType::Auto);
    }

    fn pick_output_dir(&mut self) {
        if let Some(path) = FileDialog::new().set_title("Pilih folder output").pick_folder() {
            self.log(Level::Info, format!("Folder output: {}", path.display()));
            self.output_dir = Some(path);
        }
    }

    fn start_conversion(&mut self) {
        if self.busy() || self.files.is_empty() {
            return;
        }

        let files: Vec<Arc<DatFile>> = self.ready_files().cloned().collect();
        let column_types = self.column_types.clone();
        let output_dir = self.output_dir.clone();
        let tx = self.tx.clone();

        self.converting = true;
        self.progress = 0.0;
        self.log(Level::Info, format!("Memulai konversi {} file…", files.len()));

        thread::spawn(move || {
            let started = Instant::now();
            let total_rows: usize = files.iter().map(|f| f.table.rows()).sum::<usize>().max(1);
            let mut done_rows = 0usize;
            let (mut ok, mut failed) = (0, 0);

            for file in &files {
                let dir = output_dir
                    .clone()
                    .or_else(|| file.path.parent().map(Path::to_path_buf))
                    .unwrap_or_else(|| PathBuf::from("."));

                if let Err(error) = fs::create_dir_all(&dir) {
                    let _ = tx.send(Message::Log(Level::Error, format!("Gagal membuat folder output: {error}")));
                    failed += 1;
                    done_rows += file.table.rows();
                    continue;
                }

                let output_path = unique_output_path(&dir, &file.path);
                let _ = tx.send(Message::Log(Level::Info, format!("Memproses {}…", file.name)));

                let base = done_rows;
                let mut on_progress = |rows: usize| {
                    let _ = tx.send(Message::Progress((base + rows) as f32 / total_rows as f32));
                };

                match excel::convert(&file.table, &file.profiles, &column_types, &output_path, &mut on_progress) {
                    Ok(stats) => {
                        ok += 1;
                        let sheets = if stats.sheets > 1 {
                            format!(", {} sheet", stats.sheets)
                        } else {
                            String::new()
                        };
                        let _ = tx.send(Message::Log(
                            Level::Success,
                            format!(
                                "Berhasil: {} ({} baris{sheets})",
                                output_path.display(),
                                format_count(stats.rows)
                            ),
                        ));

                        if stats.text_fallbacks > 0 {
                            let _ = tx.send(Message::Log(
                                Level::Warn,
                                format!(
                                    "{}: {} nilai tidak sesuai tipe kolom dan disimpan sebagai teks, mis. {}",
                                    file.name,
                                    format_count(stats.text_fallbacks),
                                    stats.fallback_examples.join("; ")
                                ),
                            ));
                        }
                    }
                    Err(error) => {
                        failed += 1;
                        let _ = tx.send(Message::Log(
                            Level::Error,
                            format!("Gagal mengonversi {}: {error}", file.name),
                        ));
                    }
                }

                done_rows += file.table.rows();
            }

            let _ = tx.send(Message::Finished {
                ok,
                failed,
                seconds: started.elapsed().as_secs_f32(),
            });
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Loaded(file) => {
                    // File dihapus sebelum selesai dibaca → abaikan.
                    let Some(slot) = self
                        .files
                        .iter_mut()
                        .find(|slot| slot.path == file.path && slot.data.is_none())
                    else {
                        continue;
                    };
                    slot.data = Some(Arc::clone(&file));
                    let header = if file.header { ", baris judul terdeteksi" } else { "" };
                    self.log(
                        Level::Success,
                        format!(
                            "File ditambahkan: {} ({} baris, {} kolom, delimiter '{}', {}{header})",
                            file.name,
                            format_count(file.data_rows()),
                            file.table.columns(),
                            match file.table.delimiter {
                                '\t' => "tab".to_string(),
                                c => c.to_string(),
                            },
                            file.table.encoding,
                        ),
                    );
                    self.sync_column_types();
                }
                Message::LoadFailed { path, error } => {
                    if let Some(index) = self
                        .files
                        .iter()
                        .position(|slot| slot.path == path && slot.data.is_none())
                    {
                        self.files.remove(index);
                        if self.selected > index || self.selected >= self.files.len() {
                            self.selected = self.selected.saturating_sub(1);
                        }
                    }
                    self.log(Level::Error, format!("Gagal membaca {}: {error}", path.display()));
                }
                Message::Log(level, text) => self.log(level, text),
                Message::Progress(progress) => self.progress = progress,
                Message::Finished { ok, failed, seconds } => {
                    self.converting = false;
                    self.progress = 1.0;
                    let level = if failed == 0 { Level::Success } else { Level::Warn };
                    self.log(
                        level,
                        format!("Selesai dalam {seconds:.1} detik: {ok} berhasil, {failed} gagal."),
                    );
                }
            }
        }

        if self.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    }
}

fn unique_output_path(output_dir: &Path, source_path: &Path) -> PathBuf {
    let stem = source_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("hasil");

    let mut candidate = output_dir.join(format!("{stem}.xlsx"));
    let mut suffix = 1;

    while candidate.exists() {
        candidate = output_dir.join(format!("{stem}_{suffix}.xlsx"));
        suffix += 1;
    }

    candidate
}

// ============================================================
// TAMPILAN
// ============================================================

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.handle_drop(ctx);

        egui::TopBottomPanel::top("header")
            .frame(
                Frame::new()
                    .fill(palette::CARD)
                    .inner_margin(Margin::symmetric(20, 14))
                    .stroke(Stroke::new(1.0_f32, palette::BORDER)),
            )
            .show(ctx, |ui| self.header(ui));

        egui::TopBottomPanel::bottom("footer")
            .frame(Frame::new().fill(palette::BG).inner_margin(Margin::symmetric(20, 6)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("© 2026 PDE Bapenda").small().color(palette::MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(concat!("DAT to Excel v", env!("CARGO_PKG_VERSION")))
                                .small()
                                .color(palette::MUTED),
                        );
                    });
                });
            });

        egui::SidePanel::left("sidebar")
            .exact_width(320.0)
            .resizable(false)
            .frame(Frame::new().fill(palette::BG).inner_margin(Margin {
                left: 16,
                right: 8,
                top: 16,
                bottom: 16,
            }))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.files_card(ui);
                    ui.add_space(12.0);
                    self.convert_card(ui);
                });
            });

        egui::CentralPanel::default()
            .frame(Frame::new().fill(palette::BG).inner_margin(Margin {
                left: 8,
                right: 16,
                top: 16,
                bottom: 16,
            }))
            .show(ctx, |ui| {
                let log_height = 170.0;
                let preview_height = (ui.available_height() - log_height - 12.0).max(220.0);

                ui.allocate_ui(vec2(ui.available_width(), preview_height), |ui| {
                    card(ui, |ui| {
                        ui.set_min_height(preview_height - 34.0);
                        self.preview(ui);
                    });
                });
                ui.add_space(12.0);
                card(ui, |ui| self.log_view(ui));
            });

        self.drop_overlay(ctx);
    }
}

impl App {
    fn handle_drop(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());

        if !dropped.is_empty() && !self.converting {
            self.load_files(dropped);
        }
    }

    fn drop_overlay(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }

        let rect = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
        painter.rect_filled(rect, 0.0_f32, Color32::from_rgba_unmultiplied(79, 70, 229, 40));
        painter.rect_stroke(
            rect.shrink(12.0),
            CornerRadius::same(16),
            Stroke::new(2.0_f32, palette::ACCENT),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Lepaskan file untuk menambahkan",
            FontId::proportional(20.0),
            palette::ACCENT,
        );
    }

    fn header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(9), palette::ACCENT);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "XL",
                FontId::proportional(14.0),
                Color32::WHITE,
            );

            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("DAT to Excel").size(18.0).strong());
                ui.label(
                    RichText::new("Konversi file DAT/TXT/CSV ke Microsoft Excel dengan deteksi tipe otomatis")
                        .small()
                        .color(palette::MUTED),
                );
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let loading = self.loading();
                if loading > 0 {
                    ui.spinner();
                    ui.label(RichText::new(format!("Membaca {loading} file…")).color(palette::MUTED));
                } else if !self.files.is_empty() {
                    let rows: usize = self.ready_files().map(|f| f.data_rows()).sum();
                    badge(
                        ui,
                        format!("{} file · {} baris", self.files.len(), format_count(rows)),
                        palette::ACCENT,
                        palette::ACCENT_SOFT,
                    );
                }
            });
        });
    }

    fn files_card(&mut self, ui: &mut Ui) {
        card(ui, |ui| {
            section_title(ui, "File sumber", Some("Pilih atau seret file .dat, .txt, .csv"));

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.converting, egui::Button::new("+  Tambah file"))
                    .clicked()
                {
                    self.pick_files();
                }
                if ui
                    .add_enabled(!self.converting && !self.files.is_empty(), egui::Button::new("Kosongkan"))
                    .clicked()
                {
                    self.files.clear();
                    self.selected = 0;
                    self.sync_column_types();
                    self.log(Level::Info, "Daftar file dikosongkan.");
                }
            });

            ui.add_space(6.0);

            if self.files.is_empty() {
                Frame::new()
                    .stroke(Stroke::new(1.0_f32, palette::BORDER))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(12, 22))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("Belum ada file").color(palette::MUTED));
                            ui.label(RichText::new("Seret file ke jendela ini").small().color(palette::MUTED));
                        });
                    });
                return;
            }

            let mut remove = None;

            for (index, slot) in self.files.iter().enumerate() {
                let selected = index == self.selected;
                let fill = if selected { palette::ACCENT_SOFT } else { palette::CARD };
                let stroke = if selected { palette::ACCENT } else { palette::BORDER };

                // Area klik kartu dibuat lebih dulu (scope ber-sense) sehingga
                // tombol × di dalamnya tetap menerima klik sendiri.
                let response = ui
                    .scope_builder(
                        egui::UiBuilder::new().id_salt(("file", index)).sense(Sense::click()),
                        |ui| {
                            Frame::new()
                                .fill(fill)
                                .stroke(Stroke::new(1.0_f32, stroke))
                                .corner_radius(CornerRadius::same(8))
                                .inner_margin(Margin::symmetric(10, 8))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        // Sisakan ruang untuk tombol di kanan.
                                        let text_width = (ui.available_width() - 56.0).max(40.0);
                                        ui.vertical(|ui| {
                                            ui.set_width(text_width);
                                            ui.spacing_mut().item_spacing.y = 2.0;
                                            ui.add(
                                                egui::Label::new(RichText::new(&slot.name).strong())
                                                    .truncate()
                                                    .selectable(false),
                                            );
                                            let detail = match &slot.data {
                                                Some(file) => format!(
                                                    "{} baris · {} kolom · {}",
                                                    format_count(file.data_rows()),
                                                    file.table.columns(),
                                                    format_size(file.size)
                                                ),
                                                None => "Membaca file…".to_string(),
                                            };
                                            ui.add(
                                                egui::Label::new(RichText::new(detail).small().color(palette::MUTED))
                                                    .selectable(false),
                                            );
                                        });

                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            let button = egui::Button::new(
                                                RichText::new("×").size(18.0).color(palette::MUTED),
                                            )
                                            .frame(false)
                                            .min_size(vec2(26.0, 26.0));
                                            if ui
                                                .add_enabled(!self.converting, button)
                                                .on_hover_text("Hapus dari daftar")
                                                .clicked()
                                            {
                                                remove = Some(index);
                                            }
                                            if slot.data.is_none() {
                                                ui.spinner();
                                            }
                                        });
                                    });
                                });
                        },
                    )
                    .response
                    .on_hover_cursor(egui::CursorIcon::PointingHand);

                if response.clicked() && remove.is_none() {
                    self.selected = index;
                }
                ui.add_space(2.0);
            }

            if let Some(index) = remove {
                self.remove_file(index);
            }
        });
    }

    fn convert_card(&mut self, ui: &mut Ui) {
        card(ui, |ui| {
            section_title(ui, "Konversi", None);

            ui.label(RichText::new("Folder output").small().color(palette::MUTED));
            let output = self
                .output_dir
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "Sama dengan folder file sumber".to_string());
            ui.add(egui::Label::new(output).truncate());

            ui.horizontal(|ui| {
                if ui.add_enabled(!self.converting, egui::Button::new("Pilih folder")).clicked() {
                    self.pick_output_dir();
                }
                if self.output_dir.is_some()
                    && ui.add_enabled(!self.converting, egui::Button::new("Reset")).clicked()
                {
                    self.output_dir = None;
                }
            });

            ui.add_space(8.0);

            let label = if self.converting { "Mengonversi…" } else { "Konversi ke Excel" };
            let width = ui.available_width();
            if ui
                .add_enabled(!self.busy() && !self.files.is_empty(), primary_button(label, width))
                .clicked()
            {
                self.start_conversion();
            }

            if self.converting || self.progress > 0.0 {
                ui.add_space(4.0);
                ui.add(
                    egui::ProgressBar::new(self.progress)
                        .desired_height(8.0)
                        .corner_radius(CornerRadius::same(4))
                        .fill(if self.converting { palette::ACCENT } else { palette::SUCCESS })
                        .animate(self.converting),
                );
                let status = if self.converting {
                    format!("{:.0}%", self.progress * 100.0)
                } else {
                    "Selesai".to_string()
                };
                ui.label(RichText::new(status).small().color(palette::MUTED));
            }
        });
    }

    fn preview(&mut self, ui: &mut Ui) {
        if let Some(slot) = self.files.get(self.selected).filter(|slot| slot.data.is_none()) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Preview").heading().strong());
                ui.label(RichText::new(&slot.name).color(palette::MUTED));
            });
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.label(RichText::new("Membaca dan menganalisis file…").color(palette::MUTED));
            });
            return;
        }

        let Some(file) = self.files.get(self.selected).and_then(|slot| slot.data.clone()) else {
            section_title(ui, "Preview", None);
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Tambahkan file untuk melihat preview dan mengatur tipe kolom.").color(palette::MUTED));
            });
            return;
        };

        ui.horizontal(|ui| {
            ui.label(RichText::new("Preview").heading().strong());
            ui.label(RichText::new(&file.name).color(palette::MUTED));

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(!self.converting, egui::Button::new("Reset tipe"))
                    .on_hover_text("Kembalikan semua kolom ke Otomatis")
                    .clicked()
                {
                    self.column_types.fill(ColumnType::Auto);
                }

                egui::ComboBox::from_id_salt("preview_rows")
                    .width(90.0)
                    .selected_text(format!("{} baris", self.preview_rows))
                    .show_ui(ui, |ui| {
                        for n in [20, 50, 100, 200] {
                            ui.selectable_value(&mut self.preview_rows, n, format!("{n} baris"));
                        }
                    });
            });
        });
        ui.label(
            RichText::new(
                "Tipe kolom berlaku untuk semua file. Sel berwarna oranye tidak cocok dengan tipenya dan akan disimpan sebagai teks.",
            )
            .small()
            .color(palette::MUTED),
        );
        ui.add_space(4.0);

        let columns = file.table.columns();
        let header = file.header;
        let first_data = usize::from(header);
        let last = (first_data + self.preview_rows).min(file.table.rows());

        let resolved: Vec<Resolved> = (0..columns)
            .map(|c| excel::resolve(self.column_types.get(c).copied().unwrap_or_default(), &file.profiles[c]))
            .collect();

        // Isi preview sudah diformat seperti hasil Excel.
        let cells: Vec<Vec<(String, bool, bool)>> = (first_data..last)
            .map(|row| {
                (0..columns)
                    .map(|c| {
                        let raw = file.table.cell(row, c);
                        let value = excel::cell_value(raw, resolved[c], &file.profiles[c]);
                        let mismatch = matches!(value, CellValue::Text(_)) && resolved[c] != Resolved::Text;
                        let numeric = matches!(value, CellValue::Number(_));
                        (excel::display(&value, resolved[c]), numeric, mismatch)
                    })
                    .collect()
            })
            .collect();

        let font = FontId::proportional(13.5);
        let widths: Vec<f32> = (0..columns)
            .map(|c| {
                let longest = cells.iter().map(|r| r[c].0.chars().count()).max().unwrap_or(0);
                let title = if header { file.table.cell(0, c).chars().count() } else { 0 };
                (longest.max(title) as f32 * 7.6 + 24.0).clamp(150.0, 320.0)
            })
            .collect();
        let total_width: f32 = widths.iter().sum::<f32>() + 44.0;
        let row_height = 26.0;

        egui::ScrollArea::horizontal()
            .id_salt("preview_h")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.set_min_width(total_width);
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);

                // Kepala kolom: judul, pilihan tipe, hasil deteksi.
                let head_height = 80.0;
                let (head_rect, _) = ui.allocate_exact_size(vec2(total_width, head_height), Sense::hover());
                ui.painter().rect_filled(head_rect, CornerRadius::same(6), palette::STRIPE);

                let mut x = head_rect.left() + 44.0;
                for c in 0..columns {
                    let rect = egui::Rect::from_min_size(pos2(x, head_rect.top()), vec2(widths[c], head_height))
                        .shrink2(vec2(6.0, 6.0));
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
                    child.spacing_mut().item_spacing = vec2(4.0, 4.0);

                    let title = if header && !file.table.cell(0, c).is_empty() {
                        file.table.cell(0, c).to_string()
                    } else {
                        format!("Kolom {}", c + 1)
                    };
                    child.add(egui::Label::new(RichText::new(title).strong()).truncate());

                    if let Some(column_type) = self.column_types.get_mut(c) {
                        child.add_enabled_ui(!self.converting, |ui| {
                            egui::ComboBox::from_id_salt(("column_type", c))
                                .width(widths[c] - 14.0)
                                .selected_text(column_type.label())
                                .show_ui(ui, |ui| {
                                    for option in ColumnType::ALL {
                                        ui.selectable_value(column_type, option, option.label());
                                    }
                                });
                        });
                    }

                    let auto = self.column_types.get(c).copied().unwrap_or_default() == ColumnType::Auto;
                    let note = if auto {
                        format!("Terdeteksi: {}", resolved[c].label())
                    } else {
                        format!("Format: {}", resolved[c].label())
                    };
                    child.label(RichText::new(note).small().color(if auto { palette::ACCENT } else { palette::MUTED }));

                    x += widths[c];
                }

                // Baris data (dengan scroll vertikal, kepala tetap terlihat).
                egui::ScrollArea::vertical()
                    .id_salt("preview_v")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (i, row) in cells.iter().enumerate() {
                            let (rect, _) = ui.allocate_exact_size(vec2(total_width, row_height), Sense::hover());
                            let painter = ui.painter();

                            if i % 2 == 1 {
                                painter.rect_filled(rect, 0.0_f32, palette::STRIPE);
                            }

                            painter.text(
                                pos2(rect.left() + 34.0, rect.center().y),
                                Align2::RIGHT_CENTER,
                                (first_data + i + 1).to_string(),
                                FontId::proportional(12.0),
                                palette::MUTED,
                            );

                            let mut x = rect.left() + 44.0;
                            for (c, (text, numeric, mismatch)) in row.iter().enumerate() {
                                let cell = egui::Rect::from_min_size(pos2(x, rect.top()), vec2(widths[c], row_height));
                                let inner = cell.shrink2(vec2(8.0, 0.0));

                                if *mismatch {
                                    painter.rect_filled(cell.shrink(2.0), CornerRadius::same(4), palette::WARN_SOFT);
                                }

                                let (pos, align) = if *numeric {
                                    (pos2(inner.right(), inner.center().y), Align2::RIGHT_CENTER)
                                } else {
                                    (pos2(inner.left(), inner.center().y), Align2::LEFT_CENTER)
                                };
                                let color = if *mismatch { palette::WARN } else { palette::TEXT };
                                painter.with_clip_rect(inner).text(pos, align, text, font.clone(), color);

                                if *mismatch {
                                    ui.interact(cell, ui.id().with(("cell", i, c)), Sense::hover())
                                        .on_hover_text(format!("Tidak cocok dengan tipe {}; disimpan sebagai teks.", resolved[c].label()));
                                }

                                x += widths[c];
                            }
                        }

                        if file.data_rows() > cells.len() {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(format!(
                                    "Menampilkan {} dari {} baris",
                                    cells.len(),
                                    format_count(file.data_rows())
                                ))
                                .small()
                                .color(palette::MUTED),
                            );
                        }
                    });
            });
    }

    fn log_view(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Log").heading().strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(!self.logs.is_empty(), egui::Button::new("Hapus log").small()).clicked() {
                    self.logs.clear();
                }
            });
        });

        egui::ScrollArea::vertical()
            .id_salt("log")
            .max_height(100.0)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                if self.logs.is_empty() {
                    ui.label(RichText::new("Belum ada aktivitas.").color(palette::MUTED));
                }
                for entry in &self.logs {
                    let (dot, color) = match entry.level {
                        Level::Info => ("•", palette::MUTED),
                        Level::Success => ("✔", palette::SUCCESS),
                        Level::Warn => ("!", palette::WARN),
                        Level::Error => ("×", palette::ERROR),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(RichText::new(&entry.time).monospace().color(palette::MUTED));
                        ui.label(RichText::new(dot).color(color));
                        let text = RichText::new(&entry.text);
                        ui.label(if entry.level == Level::Error { text.color(palette::ERROR) } else { text });
                    });
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulasikan klik mouse pada daftar file dengan egui headless.
    fn click(ctx: &egui::Context, app: &mut App, pos: egui::Pos2) {
        let frame = |ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 600.0))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.files_card(ui));
            });
        };
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(ctx, app, vec![egui::Event::PointerMoved(pos)]);
        frame(ctx, app, vec![button(true)]);
        frame(ctx, app, vec![button(false)]);
        frame(ctx, app, vec![]);
    }

    fn app_with_slots(n: usize) -> App {
        let mut app = App::default();
        for i in 0..n {
            app.files.push(FileSlot {
                path: PathBuf::from(format!("/tmp/f{i}.dat")),
                name: format!("f{i}.dat"),
                data: None,
            });
        }
        app
    }

    #[test]
    fn remove_button_and_select_card() {
        // Uji black-box: klik di kolom tombol × dan di area teks, per posisi y.
        let run = |pos: egui::Pos2| {
            let ctx = egui::Context::default();
            apply_theme(&ctx);
            let mut app = app_with_slots(3);
            click(&ctx, &mut app, pos2(-50.0, -50.0));
            click(&ctx, &mut app, pos);
            (app.files.len(), app.selected)
        };

        let mut removed = 0;
        let mut selected_other = 0;
        for y in (0..420).step_by(4) {
            let y = y as f32;
            // Kolom tombol × (sisi kanan kartu).
            let (len, _) = (330..=370)
                .step_by(8)
                .map(|x| run(pos2(x as f32, y)))
                .min_by_key(|r| r.0)
                .unwrap();
            if len == 2 {
                removed += 1;
            }
            // Area nama file: memilih kartu, tidak menghapus.
            let (len, selected) = run(pos2(120.0, y));
            if len == 0 {
                continue; // tombol "Kosongkan"
            }
            assert_eq!(len, 3, "klik di nama file tidak boleh menghapus (y={y})");
            if selected != 0 {
                selected_other += 1;
            }
        }

        assert!(removed > 0, "tombol × tidak pernah menghapus file");
        assert!(selected_other > 0, "klik kartu tidak pernah memilih file");
    }
}

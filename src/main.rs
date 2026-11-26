use chrono::NaiveDate;
use eframe::egui;
use rfd::FileDialog;
use rust_xlsxwriter::{Format, Workbook};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::Instant,
};

const PREVIEW_ROWS: usize = 20;
const MAX_EXCEL_ROWS: usize = 1_048_576;
const MAX_EXCEL_COLUMNS: usize = 16_384;
const LOG_EVERY_ROWS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DataType {
    Auto,
    Text,
    Date,
    Number,
    Currency,
    Percentage,
    Boolean,
}

impl DataType {
    const ALL: [Self; 7] = [
        Self::Auto,
        Self::Text,
        Self::Date,
        Self::Number,
        Self::Currency,
        Self::Percentage,
        Self::Boolean,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Auto => "Otomatis",
            Self::Text => "Teks",
            Self::Date => "Tanggal",
            Self::Number => "Angka",
            Self::Currency => "Mata Uang",
            Self::Percentage => "Persentase",
            Self::Boolean => "Boolean",
        }
    }
}

struct PreviewData {
    rows: Vec<Vec<String>>,
    total_rows: usize,
    column_count: usize,
}

struct PreviewFile {
    path: PathBuf,
    data: PreviewData,
}

struct DatToExcelApp {
    selected_files: Vec<PathBuf>,
    output_dir: Option<PathBuf>,

    previews: Vec<PreviewFile>,
    preview_index: usize,

    column_types: Vec<DataType>,
    column_count: usize,

    logs: Vec<String>,
    busy: bool,
    receiver: Option<Receiver<String>>,
}

impl Default for DatToExcelApp {
    fn default() -> Self {
        Self {
            selected_files: Vec::new(),
            output_dir: None,
            previews: Vec::new(),
            preview_index: 0,
            column_types: Vec::new(),
            column_count: 0,
            logs: Vec::new(),
            busy: false,
            receiver: None,
        }
    }
}

impl DatToExcelApp {
    fn select_files(&mut self, files: Vec<PathBuf>) {
        self.selected_files = files;
        self.previews.clear();
        self.preview_index = 0;
        self.column_count = 0;
        self.column_types.clear();

        let mut max_columns = 0;

        for path in &self.selected_files {
            match load_preview(path) {
                Ok(data) => {
                    max_columns = max_columns.max(data.column_count);

                    self.logs.push(format!(
                        "[PREVIEW] {} | {} baris | {} kolom",
                        path.display(),
                        data.total_rows,
                        data.column_count
                    ));

                    self.previews.push(PreviewFile {
                        path: path.clone(),
                        data,
                    });
                }
                Err(error) => {
                    self.logs
                        .push(format!("[GAGAL BACA] {}: {}", path.display(), error));
                }
            }
        }

        self.column_count = max_columns;
        self.column_types = vec![DataType::Auto; self.column_count];

        if self.previews.is_empty() && !self.selected_files.is_empty() {
            self.logs
                .push("[PERINGATAN] Tidak ada file yang berhasil dibaca.".to_string());
        }
    }

    fn poll_conversion(&mut self) {
        let mut messages = Vec::new();
        let mut disconnected = false;

        let Some(receiver) = &self.receiver else {
            return;
        };

        loop {
            match receiver.try_recv() {
                Ok(message) => messages.push(message),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        self.logs.extend(messages);

        if disconnected {
            self.logs
                .push("[SELESAI] Seluruh proses konversi berakhir.".to_string());

            self.busy = false;
            self.receiver = None;
        }
    }

    fn start_conversion(&mut self) {
        if self.busy {
            return;
        }

        if self.selected_files.is_empty() {
            self.logs
                .push("[PERINGATAN] Pilih file DAT terlebih dahulu.".to_string());
            return;
        }

        let Some(output_dir) = self.output_dir.clone() else {
            self.logs
                .push("[PERINGATAN] Pilih folder output terlebih dahulu.".to_string());
            return;
        };

        let files = self.selected_files.clone();
        let column_types = self.column_types.clone();
        let total_files = files.len();

        let (sender, receiver) = mpsc::channel::<String>();

        self.busy = true;
        self.logs.clear();

        self.logs
            .push(format!("[MULAI] Konversi {} file", total_files));
        self.logs
            .push(format!("[INFO] Folder output: {}", output_dir.display()));

        thread::spawn(move || {
            let batch_started = Instant::now();
            let mut success_count = 0usize;
            let mut failure_count = 0usize;

            for (file_index, input) in files.iter().enumerate() {
                let file_name = input
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("File DAT");

                let _ = sender.send(format!(
                    "\n[FILE {}/{}] Memulai: {}",
                    file_index + 1,
                    total_files,
                    file_name
                ));

                let result = convert_file(input, &output_dir, &column_types, |message| {
                    let _ = sender.send(message);
                });

                match result {
                    Ok(output) => {
                        success_count += 1;

                        let _ = sender.send(format!(
                            "[BERHASIL] {} -> {}",
                            file_name,
                            output.display()
                        ));
                    }
                    Err(error) => {
                        failure_count += 1;

                        let _ = sender.send(format!("[GAGAL] {}: {}", file_name, error));
                    }
                }
            }

            let _ = sender.send(format!(
                "\n[RINGKASAN] Berhasil: {} | Gagal: {} | Total: {}",
                success_count, failure_count, total_files
            ));

            let _ = sender.send(format!(
                "[DURASI TOTAL] {:.2} detik",
                batch_started.elapsed().as_secs_f64()
            ));
        });

        self.receiver = Some(receiver);
    }
}

impl eframe::App for DatToExcelApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_conversion();

        let mut visuals = egui::Visuals::light();
        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(22, 101, 52);
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(34, 139, 78);
        visuals.selection.bg_fill = egui::Color32::from_rgb(22, 163, 74);
        ctx.set_visuals(visuals);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading(
                    egui::RichText::new("DAT to Excel")
                        .size(23.0)
                        .strong()
                        .color(egui::Color32::from_rgb(22, 101, 52)),
                );

                ui.label("Konversi file DAT menjadi Excel (.xlsx)");
            });

            ui.add_space(10.0);

            // File sumber.
            ui.group(|ui| {
                ui.heading("File Sumber");

                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Pilih File DAT"))
                        .clicked()
                    {
                        if let Some(files) = FileDialog::new()
                            .add_filter("Data DAT", &["dat"])
                            .pick_files()
                        {
                            self.select_files(files);
                        }
                    }

                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Hapus Pilihan"))
                        .clicked()
                    {
                        self.select_files(Vec::new());
                    }
                });

                ui.label(format!("{} file dipilih", self.selected_files.len()));
            });

            ui.add_space(8.0);

            // Preview data.
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.heading("Preview Data");

                    ui.label(
                        egui::RichText::new(format!("{} file", self.previews.len()))
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                });

                if self.previews.is_empty() {
                    ui.add_space(8.0);
                    ui.label("Pilih file DAT untuk melihat contoh data.");
                } else {
                    let selected_name = self
                        .previews
                        .get(self.preview_index)
                        .and_then(|file| file.path.file_name())
                        .and_then(|name| name.to_str())
                        .unwrap_or("File DAT")
                        .to_string();

                    ui.horizontal(|ui| {
                        ui.label("File preview:");

                        egui::ComboBox::from_id_salt("preview_file_selector")
                            .selected_text(selected_name)
                            .width(280.0)
                            .show_ui(ui, |ui| {
                                for (index, file) in self.previews.iter().enumerate() {
                                    let name = file
                                        .path
                                        .file_name()
                                        .and_then(|name| name.to_str())
                                        .unwrap_or("File DAT");

                                    ui.selectable_value(&mut self.preview_index, index, name);
                                }
                            });
                    });

                    let preview = &self.previews[self.preview_index].data;

                    ui.label(
                        egui::RichText::new(format!(
                            "{} baris · {} kolom · maksimal {} baris preview",
                            preview.total_rows, preview.column_count, PREVIEW_ROWS
                        ))
                        .small()
                        .color(egui::Color32::GRAY),
                    );

                    ui.add_space(6.0);

                    egui::ScrollArea::both()
                        .id_salt("dat_preview_scroll")
                        .max_height(320.0)
                        .show(ui, |ui| {
                            egui::Grid::new("dat_preview_grid")
                                .striped(true)
                                .min_col_width(125.0)
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new("#").strong());

                                    for index in 0..preview.column_count {
                                        egui::ComboBox::from_id_salt(format!(
                                            "column_type_{index}"
                                        ))
                                        .selected_text(self.column_types[index].label())
                                        .width(115.0)
                                        .show_ui(
                                            ui,
                                            |ui| {
                                                for data_type in DataType::ALL {
                                                    ui.selectable_value(
                                                        &mut self.column_types[index],
                                                        data_type,
                                                        data_type.label(),
                                                    );
                                                }
                                            },
                                        );
                                    }

                                    ui.end_row();

                                    for (row_index, row) in preview.rows.iter().enumerate() {
                                        ui.label(
                                            egui::RichText::new((row_index + 1).to_string())
                                                .small()
                                                .color(egui::Color32::GRAY),
                                        );

                                        for column_index in 0..preview.column_count {
                                            let value = row
                                                .get(column_index)
                                                .map(String::as_str)
                                                .unwrap_or("");

                                            ui.add_sized([125.0, 22.0], egui::Label::new(value));
                                        }

                                        ui.end_row();
                                    }
                                });
                        });

                    ui.label(
                        egui::RichText::new(
                            "Tipe data berlaku berdasarkan posisi kolom untuk semua file.",
                        )
                        .small()
                        .color(egui::Color32::GRAY),
                    );
                }
            });

            ui.add_space(8.0);

            // Folder output.
            ui.group(|ui| {
                ui.heading("Folder Output");

                if ui
                    .add_enabled(!self.busy, egui::Button::new("Pilih Folder"))
                    .clicked()
                {
                    if let Some(folder) = FileDialog::new().pick_folder() {
                        self.output_dir = Some(folder);
                    }
                }

                ui.label(
                    self.output_dir
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "Belum dipilih".to_string()),
                );
            });

            ui.add_space(10.0);

            ui.vertical_centered(|ui| {
                let button = egui::Button::new(
                    egui::RichText::new(if self.busy {
                        "Sedang Mengonversi..."
                    } else {
                        "Konversi ke Excel"
                    })
                    .strong()
                    .color(egui::Color32::WHITE),
                )
                .fill(egui::Color32::from_rgb(22, 101, 52))
                .min_size(egui::vec2(220.0, 38.0));

                if ui.add_enabled(!self.busy, button).clicked() {
                    self.start_conversion();
                }
            });

            ui.add_space(8.0);

            // Log proses.
            egui::CollapsingHeader::new("Log Proses")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} pesan", self.logs.len()));

                        if ui
                            .add_enabled(!self.busy, egui::Button::new("Bersihkan Log"))
                            .clicked()
                        {
                            self.logs.clear();
                        }
                    });

                    egui::ScrollArea::vertical()
                        .id_salt("conversion_logs")
                        .max_height(125.0)
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            if self.logs.is_empty() {
                                ui.label("Belum ada proses.");
                            }

                            for message in &self.logs {
                                ui.label(message);
                            }
                        });
                });

            ui.add_space(8.0);

            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("Copyright © 2026 PDE Bapenda")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            });
        });

        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

fn decode_file(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }

    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        return char::decode_utf16(units)
            .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
    }

    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect();

        return char::decode_utf16(units)
            .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
    }

    String::from_utf8_lossy(bytes).into_owned()
}

fn parse_records(content: &str) -> Vec<Vec<String>> {
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let line = line.trim_end_matches('\r');

            // Menghilangkan delimiter terakhir jika file menggunakan
            // tanda | tambahan di akhir setiap baris.
            let line = line.strip_suffix('|').unwrap_or(line);

            line.split('|')
                .map(|value| value.trim().to_string())
                .collect()
        })
        .collect()
}

fn load_preview(path: &Path) -> Result<PreviewData, String> {
    let bytes = fs::read(path).map_err(|error| format!("Gagal membaca file: {error}"))?;

    let content = decode_file(&bytes);
    let records = parse_records(&content);

    let column_count = records.iter().map(Vec::len).max().unwrap_or(0);
    let total_rows = records.len();

    Ok(PreviewData {
        rows: records.into_iter().take(PREVIEW_ROWS).collect(),
        total_rows,
        column_count,
    })
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    let value = value.trim();

    [
        "%d-%m-%Y", "%d/%m/%Y", "%d.%m.%Y", "%Y-%m-%d", "%Y/%m/%d", "%d-%m-%y", "%d/%m/%y",
    ]
    .iter()
    .find_map(|format| NaiveDate::parse_from_str(value, format).ok())
}

fn parse_number(value: &str) -> Option<f64> {
    let value = value.trim();

    let cleaned = value
        .strip_prefix("Rp")
        .or_else(|| value.strip_prefix("rp"))
        .unwrap_or(value)
        .trim()
        .replace(' ', "");

    if cleaned.is_empty() {
        return None;
    }

    let unsigned = cleaned
        .strip_prefix('-')
        .or_else(|| cleaned.strip_prefix('+'))
        .unwrap_or(&cleaned);

    let integer_part = unsigned.split('.').next().unwrap_or("");

    // Hindari mengubah kode seperti 00123 menjadi angka.
    if integer_part.len() > 1
        && integer_part.starts_with('0')
        && integer_part.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }

    cleaned.parse::<f64>().ok()
}

fn parse_boolean(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "true" | "1" | "ya" | "yes" | "y" => Some(true),
        "false" | "0" | "tidak" | "no" | "n" => Some(false),
        _ => None,
    }
}

fn parse_percentage(value: &str) -> Option<f64> {
    let value = value.trim();
    let number = value.strip_suffix('%').unwrap_or(value);
    parse_number(number).map(|number| number / 100.0)
}

fn date_to_excel_serial(date: NaiveDate) -> f64 {
    let epoch = NaiveDate::from_ymd_opt(1899, 12, 30).unwrap();
    (date - epoch).num_days() as f64
}

fn convert_file<F>(
    input: &Path,
    output_dir: &Path,
    column_types: &[DataType],
    mut progress: F,
) -> Result<PathBuf, String>
where
    F: FnMut(String),
{
    let started_at = Instant::now();

    let file_name = input
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("File DAT");

    progress(format!("[BACA] Membaca file: {file_name}"));

    let bytes = fs::read(input).map_err(|error| format!("Tidak dapat membaca file: {error}"))?;

    progress(format!("[BACA] Ukuran file: {} byte", bytes.len()));

    let content = decode_file(&bytes);
    let records = parse_records(&content);

    if records.is_empty() {
        return Err("File tidak berisi data.".to_string());
    }

    let total_rows = records.len();
    let column_count = records.iter().map(Vec::len).max().unwrap_or(0);

    if total_rows > MAX_EXCEL_ROWS {
        return Err(format!(
            "Jumlah baris {} melebihi batas Excel {}.",
            total_rows, MAX_EXCEL_ROWS
        ));
    }

    if column_count > MAX_EXCEL_COLUMNS {
        return Err(format!(
            "Jumlah kolom {} melebihi batas Excel {}.",
            column_count, MAX_EXCEL_COLUMNS
        ));
    }

    progress(format!(
        "[INFO] Data terbaca: {} baris x {} kolom",
        total_rows, column_count
    ));

    fs::create_dir_all(output_dir)
        .map_err(|error| format!("Gagal membuat folder output: {error}"))?;

    let stem = input
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("hasil");

    let mut output = output_dir.join(format!("{stem}.xlsx"));

    if output.exists() {
        let mut suffix = 1usize;

        loop {
            let candidate = output_dir.join(format!("{stem}_{suffix}.xlsx"));

            if !candidate.exists() {
                output = candidate;
                break;
            }

            suffix += 1;
        }
    }

    progress(format!("[INFO] File keluaran: {}", output.display()));

    let date_format = Format::new().set_num_format("dd-mm-yyyy");
    let number_format = Format::new().set_num_format("#,##0.############");
    let currency_format = Format::new().set_num_format("\"Rp\" #,##0.00");
    let percentage_format = Format::new().set_num_format("0.00%");

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();

    worksheet
        .set_name("Data")
        .map_err(|error| error.to_string())?;

    let mut written_cells = 0usize;

    for (row_index, row) in records.iter().enumerate() {
        let current_row = row_index + 1;

        if current_row == 1 || current_row % LOG_EVERY_ROWS == 0 || current_row == total_rows {
            let percentage = current_row as f64 / total_rows as f64 * 100.0;

            progress(format!(
                "[PROGRES] Baris {}/{} ({:.2}%) | Sel ditulis: {} | Waktu: {:.2} detik",
                current_row,
                total_rows,
                percentage,
                written_cells,
                started_at.elapsed().as_secs_f64()
            ));
        }

        for column_index in 0..column_count {
            let value = row.get(column_index).map(String::as_str).unwrap_or("");

            if value.is_empty() {
                continue;
            }

            let data_type = column_types
                .get(column_index)
                .copied()
                .unwrap_or(DataType::Auto);

            let r = row_index as u32;
            let c = column_index as u16;

            let result: Result<(), String> = match data_type {
                DataType::Text => worksheet
                    .write_string(r, c, value)
                    .map(|_| ())
                    .map_err(|error| error.to_string()),

                DataType::Date => {
                    let date = parse_date(value).ok_or_else(|| {
                        format!(
                            "Baris {}, kolom {} bukan tanggal valid: {}",
                            current_row,
                            column_index + 1,
                            value
                        )
                    })?;

                    worksheet
                        .write_number_with_format(r, c, date_to_excel_serial(date), &date_format)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }

                DataType::Number => {
                    let number = parse_number(value).ok_or_else(|| {
                        format!(
                            "Baris {}, kolom {} bukan angka valid: {}",
                            current_row,
                            column_index + 1,
                            value
                        )
                    })?;

                    worksheet
                        .write_number_with_format(r, c, number, &number_format)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }

                DataType::Currency => {
                    let number = parse_number(value).ok_or_else(|| {
                        format!(
                            "Baris {}, kolom {} bukan mata uang valid: {}",
                            current_row,
                            column_index + 1,
                            value
                        )
                    })?;

                    worksheet
                        .write_number_with_format(r, c, number, &currency_format)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }

                DataType::Percentage => {
                    let number = parse_percentage(value).ok_or_else(|| {
                        format!(
                            "Baris {}, kolom {} bukan persentase valid: {}",
                            current_row,
                            column_index + 1,
                            value
                        )
                    })?;

                    worksheet
                        .write_number_with_format(r, c, number, &percentage_format)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }

                DataType::Boolean => {
                    let boolean = parse_boolean(value).ok_or_else(|| {
                        format!(
                            "Baris {}, kolom {} bukan Boolean valid: {}",
                            current_row,
                            column_index + 1,
                            value
                        )
                    })?;

                    worksheet
                        .write_boolean(r, c, boolean)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }

                DataType::Auto => {
                    if let Some(date) = parse_date(value) {
                        worksheet
                            .write_number_with_format(
                                r,
                                c,
                                date_to_excel_serial(date),
                                &date_format,
                            )
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    } else if let Some(number) = parse_number(value) {
                        worksheet
                            .write_number_with_format(r, c, number, &number_format)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    } else {
                        worksheet
                            .write_string(r, c, value)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    }
                }
            };

            result.map_err(|error| {
                format!(
                    "Gagal menulis baris {}, kolom {} (nilai {:?}): {}",
                    current_row,
                    column_index + 1,
                    value,
                    error
                )
            })?;

            written_cells += 1;
        }
    }

    progress(format!(
        "[INFO] Penulisan data selesai. Total sel ditulis: {}",
        written_cells
    ));

    progress("[SIMPAN] Menyimpan workbook Excel...".to_string());

    workbook
        .save(&output)
        .map_err(|error| format!("Gagal menyimpan Excel: {error}"))?;

    progress(format!(
        "[SIMPAN] File berhasil disimpan dalam {:.2} detik",
        started_at.elapsed().as_secs_f64()
    ));

    Ok(output)
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([680.0, 760.0])
            .with_min_inner_size([540.0, 580.0])
            .with_title("DAT to Excel - PDE Bapenda"),
        ..Default::default()
    };

    eframe::run_native(
        "DAT to Excel",
        options,
        Box::new(|_cc| Ok(Box::new(DatToExcelApp::default()))),
    )
}

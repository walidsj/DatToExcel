//! Penentuan format kolom dan penulisan workbook Excel.

use std::path::Path;

use chrono::{Datelike, Timelike};
use rust_xlsxwriter::{Color, ExcelDateTime, Format, FormatBorder, Workbook, Worksheet};

use crate::table::Table;
use crate::value::{
    ColumnProfile, DateValue, Kind, parse_date, parse_number, parse_percentage,
};

/// Batas baris per worksheet Excel.
const EXCEL_MAX_ROWS: usize = 1_048_576;

// ============================================================
// TIPE KOLOM PILIHAN PENGGUNA
// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColumnType {
    #[default]
    Auto,
    Text,
    /// Angka dengan deteksi otomatis bulat/desimal.
    Number,
    Integer,
    Decimal,
    Currency,
    Percentage,
    Date,
    DateTime,
}

impl ColumnType {
    pub const ALL: [Self; 9] = [
        Self::Auto,
        Self::Text,
        Self::Number,
        Self::Integer,
        Self::Decimal,
        Self::Currency,
        Self::Percentage,
        Self::Date,
        Self::DateTime,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Otomatis",
            Self::Text => "Teks",
            Self::Number => "Angka",
            Self::Integer => "Bilangan bulat",
            Self::Decimal => "Desimal",
            Self::Currency => "Mata uang (Rp)",
            Self::Percentage => "Persentase",
            Self::Date => "Tanggal",
            Self::DateTime => "Tanggal & waktu",
        }
    }
}

/// Format akhir sebuah kolom setelah pilihan pengguna digabung dengan profil.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolved {
    Text,
    Integer,
    Decimal(u8),
    Currency(u8),
    Percentage(u8),
    Date,
    DateTime { seconds: bool, millis: bool },
}

impl Resolved {
    pub fn label(self) -> String {
        match self {
            Resolved::Text => Kind::Text.label().to_string(),
            Resolved::Integer => "Bilangan bulat".to_string(),
            Resolved::Decimal(d) => format!("Desimal ({d} digit)"),
            Resolved::Currency(0) => "Rupiah".to_string(),
            Resolved::Currency(d) => format!("Rupiah ({d} digit)"),
            Resolved::Percentage(d) => format!("Persentase ({d} digit)"),
            Resolved::Date => "Tanggal".to_string(),
            Resolved::DateTime { .. } => "Tanggal & waktu".to_string(),
        }
    }

    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            Resolved::Integer | Resolved::Decimal(_) | Resolved::Currency(_) | Resolved::Percentage(_)
        )
    }

    fn num_format(self) -> Option<String> {
        let decimals = |d: u8| {
            if d == 0 {
                String::new()
            } else {
                format!(".{}", "0".repeat(d as usize))
            }
        };

        Some(match self {
            Resolved::Text => return None,
            Resolved::Integer => "#,##0".to_string(),
            Resolved::Decimal(d) => format!("#,##0{}", decimals(d)),
            Resolved::Currency(d) => {
                let n = format!("#,##0{}", decimals(d));
                format!("\"Rp\" {n};-\"Rp\" {n}")
            }
            Resolved::Percentage(d) => format!("0{}%", decimals(d)),
            Resolved::Date => "dd/mm/yyyy".to_string(),
            Resolved::DateTime { seconds, millis } => match (seconds, millis) {
                (_, true) => "dd/mm/yyyy hh:mm:ss.000".to_string(),
                (true, false) => "dd/mm/yyyy hh:mm:ss".to_string(),
                _ => "dd/mm/yyyy hh:mm".to_string(),
            },
        })
    }
}

fn decimal_places(profile: &ColumnProfile) -> u8 {
    profile.max_decimals.clamp(2, 6)
}

pub fn resolve(column_type: ColumnType, profile: &ColumnProfile) -> Resolved {
    let currency_decimals = if !profile.has_fraction {
        0
    } else {
        decimal_places(profile)
    };
    let percentage = Resolved::Percentage(profile.pct_decimals.max(profile.max_decimals).min(4));
    let date_time = Resolved::DateTime {
        seconds: profile.has_seconds,
        millis: profile.has_fraction_seconds,
    };

    match column_type {
        ColumnType::Auto => match profile.kind {
            Kind::Text => Resolved::Text,
            Kind::Integer => Resolved::Integer,
            Kind::Decimal => Resolved::Decimal(decimal_places(profile)),
            Kind::Currency => Resolved::Currency(currency_decimals),
            Kind::Percentage => percentage,
            Kind::Date => Resolved::Date,
            Kind::DateTime => date_time,
        },
        ColumnType::Text => Resolved::Text,
        ColumnType::Number => {
            if !profile.has_fraction {
                Resolved::Integer
            } else {
                Resolved::Decimal(decimal_places(profile))
            }
        }
        ColumnType::Integer => Resolved::Integer,
        ColumnType::Decimal => Resolved::Decimal(decimal_places(profile)),
        ColumnType::Currency => Resolved::Currency(currency_decimals),
        ColumnType::Percentage => percentage,
        ColumnType::Date => Resolved::Date,
        ColumnType::DateTime => date_time,
    }
}

// ============================================================
// NILAI SEL
// ============================================================

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CellValue<'a> {
    Empty,
    Text(&'a str),
    Number(f64),
    Date(DateValue),
}

/// Ubah teks sel menjadi nilai sesuai format kolom. Nilai yang gagal
/// di-parse dikembalikan sebagai teks apa adanya.
pub fn cell_value<'a>(raw: &'a str, resolved: Resolved, profile: &ColumnProfile) -> CellValue<'a> {
    if raw.trim().is_empty() {
        return CellValue::Empty;
    }

    let sep = profile.decimal_sep;
    let parsed = match resolved {
        Resolved::Text => None,
        Resolved::Integer | Resolved::Decimal(_) | Resolved::Currency(_) => {
            parse_number(raw, sep).map(|n| CellValue::Number(n.value))
        }
        Resolved::Percentage(_) => parse_percentage(raw, sep, false).map(|n| CellValue::Number(n.value)),
        Resolved::Date | Resolved::DateTime { .. } => {
            parse_date(raw, profile.date_order, true).map(CellValue::Date)
        }
    };

    parsed.unwrap_or(CellValue::Text(raw))
}

/// Teks tampilan (gaya Indonesia) untuk preview dan estimasi lebar kolom.
pub fn display(value: &CellValue, resolved: Resolved) -> String {
    match *value {
        CellValue::Empty => String::new(),
        CellValue::Text(text) => text.to_string(),
        CellValue::Number(n) => match resolved {
            Resolved::Integer => group_number(n, 0),
            Resolved::Decimal(d) => group_number(n, d),
            Resolved::Currency(d) => {
                let body = group_number(n.abs(), d);
                if n < 0.0 { format!("-Rp {body}") } else { format!("Rp {body}") }
            }
            Resolved::Percentage(d) => format!("{}%", group_number(n * 100.0, d)),
            _ => group_number(n, 2),
        },
        CellValue::Date(date) => {
            let d = date.date.format("%d/%m/%Y");
            match (resolved, date.time) {
                (Resolved::DateTime { seconds, millis }, time) => {
                    let time = time.unwrap_or_default();
                    let mut text = format!("{d} {:02}:{:02}", time.hour(), time.minute());
                    if seconds || millis {
                        text.push_str(&format!(":{:02}", time.second()));
                    }
                    if millis {
                        text.push_str(&format!(".{:03}", time.nanosecond() / 1_000_000));
                    }
                    text
                }
                (_, Some(time)) => format!("{d} {}", time.format("%H:%M:%S")),
                _ => d.to_string(),
            }
        }
    }
}

/// 1234567.891 → "1.234.567,89" (pemisah ribuan titik, desimal koma).
fn group_number(value: f64, decimals: u8) -> String {
    let text = format!("{:.*}", decimals as usize, value.abs());
    let (int, frac) = text.split_once('.').unwrap_or((&text, ""));

    let mut grouped = String::with_capacity(text.len() + text.len() / 3 + 1);
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(c);
    }

    let is_zero = text.bytes().all(|b| b == b'0' || b == b'.');
    let mut out = if value < 0.0 && !is_zero { "-".to_string() } else { String::new() };
    out.push_str(&grouped);
    if !frac.is_empty() {
        out.push(',');
        out.push_str(frac);
    }
    out
}

fn excel_datetime(value: &DateValue) -> Option<ExcelDateTime> {
    let date = ExcelDateTime::from_ymd(
        u16::try_from(value.date.year()).ok()?,
        value.date.month() as u8,
        value.date.day() as u8,
    )
    .ok()?;

    match value.time {
        None => Some(date),
        Some(time) => date
            .and_hms_milli(
                time.hour() as u16,
                time.minute() as u8,
                time.second() as u8,
                (time.nanosecond() / 1_000_000).min(999) as u16,
            )
            .ok(),
    }
}

// ============================================================
// KONVERSI
// ============================================================

pub struct ConvertStats {
    pub rows: usize,
    pub sheets: usize,
    /// Nilai di kolom bertipe angka/tanggal yang gagal di-parse dan ditulis sebagai teks.
    pub text_fallbacks: usize,
    pub fallback_examples: Vec<String>,
}

/// Tahap konversi yang dilaporkan ke pemanggil.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Menulis baris ke worksheet: `rows` dari `total` selesai.
    Writing { rows: usize, total: usize },
    /// Membangun dan mengompresi file xlsx lalu menyimpannya ke disk.
    Saving,
}

pub fn has_header(profiles: &[ColumnProfile]) -> bool {
    profiles.iter().any(|p| p.header_mismatch)
}

pub fn convert(
    table: &Table,
    profiles: &[ColumnProfile],
    column_types: &[ColumnType],
    output_path: &Path,
    progress: &mut dyn FnMut(Stage),
) -> Result<ConvertStats, String> {
    let columns = table.columns();
    if columns > 16_384 {
        return Err("Jumlah kolom melebihi batas Excel (16.384).".into());
    }

    let header = has_header(profiles);
    let resolved: Vec<Resolved> = (0..columns)
        .map(|col| resolve(column_types.get(col).copied().unwrap_or_default(), &profiles[col]))
        .collect();

    let formats: Vec<Option<Format>> = resolved
        .iter()
        .map(|r| {
            r.num_format().map(|f| {
                let format = Format::new().set_num_format(f);
                if r.is_numeric() { format } else { format.set_align(rust_xlsxwriter::FormatAlign::Left) }
            })
        })
        .collect();

    let header_format = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xE8EEFC))
        .set_border_bottom(FormatBorder::Thin)
        .set_border_bottom_color(Color::RGB(0x9AA8C7));

    let widths = column_widths(table, profiles, &resolved, header);

    let data_start = usize::from(header);
    let data_rows = table.rows() - data_start;
    let per_sheet = EXCEL_MAX_ROWS - data_start;
    let sheet_count = data_rows.div_ceil(per_sheet).max(1);

    let mut workbook = Workbook::new();
    let mut stats = ConvertStats {
        rows: data_rows,
        sheets: sheet_count,
        text_fallbacks: 0,
        fallback_examples: Vec::new(),
    };

    let xl = |e: rust_xlsxwriter::XlsxError| e.to_string();

    for sheet in 0..sheet_count {
        let worksheet = workbook.add_worksheet_with_low_memory();
        let name = if sheet == 0 { "Data".to_string() } else { format!("Data {}", sheet + 1) };
        worksheet.set_name(&name).map_err(xl)?;

        for (col, width) in widths.iter().enumerate() {
            worksheet.set_column_width(col as u16, *width).map_err(xl)?;
        }

        let mut excel_row: u32 = 0;

        if header {
            for col in 0..columns {
                worksheet
                    .write_string_with_format(0, col as u16, table.cell(0, col), &header_format)
                    .map_err(xl)?;
            }
            worksheet.set_freeze_panes(1, 0).map_err(xl)?;
            excel_row = 1;
        }

        let first = data_start + sheet * per_sheet;
        let last = (first + per_sheet).min(table.rows());

        for row in first..last {
            for col in 0..columns {
                write_cell(
                    worksheet,
                    excel_row,
                    col as u16,
                    table.cell(row, col),
                    resolved[col],
                    &profiles[col],
                    formats[col].as_ref(),
                    &mut stats,
                )
                .map_err(xl)?;
            }

            excel_row += 1;

            if (row + 1) % 2_000 == 0 {
                progress(Stage::Writing {
                    rows: row + 1,
                    total: table.rows(),
                });
            }
        }

        if header && excel_row > 1 && columns > 0 {
            worksheet
                .autofilter(0, 0, excel_row - 1, columns as u16 - 1)
                .map_err(xl)?;
        }
    }

    progress(Stage::Writing {
        rows: table.rows(),
        total: table.rows(),
    });
    progress(Stage::Saving);

    workbook.save(output_path).map_err(xl)?;
    Ok(stats)
}

#[allow(clippy::too_many_arguments)]
fn write_cell(
    worksheet: &mut Worksheet,
    row: u32,
    col: u16,
    raw: &str,
    resolved: Resolved,
    profile: &ColumnProfile,
    format: Option<&Format>,
    stats: &mut ConvertStats,
) -> Result<(), rust_xlsxwriter::XlsxError> {
    let value = cell_value(raw, resolved, profile);

    match (value, format) {
        (CellValue::Empty, _) => {}
        (CellValue::Number(n), Some(format)) => {
            worksheet.write_number_with_format(row, col, n, format)?;
        }
        (CellValue::Date(date), Some(format)) => match excel_datetime(&date) {
            Some(datetime) => {
                worksheet.write_datetime_with_format(row, col, &datetime, format)?;
            }
            None => {
                worksheet.write_string(row, col, raw)?;
            }
        },
        (CellValue::Text(text), _) => {
            if resolved != Resolved::Text {
                stats.text_fallbacks += 1;
                if stats.fallback_examples.len() < 5 {
                    stats.fallback_examples.push(format!("baris {}, kolom {}: \"{text}\"", row + 1, col + 1));
                }
            }
            worksheet.write_string(row, col, text)?;
        }
        (_, None) => {
            worksheet.write_string(row, col, raw)?;
        }
    }

    Ok(())
}

fn column_widths(table: &Table, profiles: &[ColumnProfile], resolved: &[Resolved], header: bool) -> Vec<f64> {
    const SAMPLE: usize = 3_000;

    (0..table.columns())
        .map(|col| {
            let mut width = if header { table.cell(0, col).chars().count() + 3 } else { 0 };

            for row in usize::from(header)..table.rows().min(SAMPLE) {
                let value = cell_value(table.cell(row, col), resolved[col], &profiles[col]);
                width = width.max(display(&value, resolved[col]).chars().count());
            }

            (width as f64 + 2.0).clamp(8.0, 60.0)
        })
        .collect()
}

// ============================================================
// TEST
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::parse_bytes;
    use crate::value::profile_table;

    #[test]
    fn grouping() {
        assert_eq!(group_number(1234567.891, 2), "1.234.567,89");
        assert_eq!(group_number(-40300.0, 0), "-40.300");
        assert_eq!(group_number(999.0, 0), "999");
        assert_eq!(group_number(-0.001, 2), "0,00");
    }

    #[test]
    fn resolve_formats() {
        let table = parse_bytes(b"1|2.5|05-12-2025 10:00:05|abc\n2|3.125|06-12-2025 11:00:00|def\n").unwrap();
        let p = profile_table(&table);

        assert_eq!(resolve(ColumnType::Auto, &p[0]), Resolved::Integer);
        assert_eq!(resolve(ColumnType::Auto, &p[1]), Resolved::Decimal(3));
        assert_eq!(resolve(ColumnType::Auto, &p[2]), Resolved::DateTime { seconds: true, millis: false });
        assert_eq!(resolve(ColumnType::Auto, &p[3]), Resolved::Text);
        assert_eq!(resolve(ColumnType::Number, &p[0]), Resolved::Integer);
        assert_eq!(resolve(ColumnType::Currency, &p[0]), Resolved::Currency(0));
        assert_eq!(Resolved::Integer.num_format().unwrap(), "#,##0");
        assert_eq!(Resolved::Decimal(2).num_format().unwrap(), "#,##0.00");

        let v = cell_value(table.cell(0, 2), resolve(ColumnType::Auto, &p[2]), &p[2]);
        assert_eq!(display(&v, resolve(ColumnType::Auto, &p[2])), "05/12/2025 10:00:05");
    }

    #[test]
    fn writes_workbook() {
        let table = parse_bytes(
            b"tanggal;nama;jumlah;nik\n05-12-2025;Budi;1.500,25;6471012345678901\n06-12-2025 08:30;Ani;-200;0012\n",
        )
        .unwrap();
        let profiles = profile_table(&table);
        assert!(has_header(&profiles));

        let dir = std::env::temp_dir().join(format!("dat-to-excel-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.xlsx");

        let stats = convert(&table, &profiles, &[], &path, &mut |_| {}).unwrap();
        assert_eq!(stats.rows, 2);
        assert_eq!(stats.text_fallbacks, 0);
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fallback_counts() {
        let p = crate::value::ColumnProfile {
            kind: Kind::Integer,
            decimal_sep: crate::value::DecimalSep::Dot,
            date_order: crate::value::DateOrder::Dmy,
            max_decimals: 0,
            has_fraction: false,
            pct_decimals: 0,
            has_seconds: false,
            has_fraction_seconds: false,
            header_mismatch: false,
            non_empty: 1,
        };
        assert_eq!(cell_value("x", Resolved::Integer, &p), CellValue::Text("x"));
        assert_eq!(cell_value("1,234", Resolved::Integer, &p), CellValue::Number(1234.0));
    }
}

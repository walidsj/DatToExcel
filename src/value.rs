//! Parsing nilai sel (angka, persentase, mata uang, tanggal, tanggal-waktu)
//! dan profil tipe per kolom.
//!
//! Deteksi dilakukan per kolom, bukan per sel: pemisah desimal (titik/koma),
//! urutan tanggal (DMY/MDY), dan jumlah digit desimal ditentukan dari seluruh
//! isi kolom sehingga nilai ambigu seperti `1.500` dibaca konsisten.

use chrono::{NaiveDate, NaiveTime, Timelike};

use crate::table::Table;

// ============================================================
// ANGKA
// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DecimalSep {
    /// Tidak ada bukti: `1.234` / `1,234` dianggap pemisah ribuan.
    #[default]
    Unknown,
    Dot,
    Comma,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Number {
    pub value: f64,
    /// Jumlah digit desimal bermakna (nol di belakang dibuang): `12.50` → 1.
    pub decimals: u8,
    /// Sumber ditulis dengan bagian desimal, walau nol (`40300.0`).
    pub has_fraction: bool,
    pub currency: bool,
}

/// Bagian angka setelah tanda, kurung, dan simbol mata uang dibuang.
struct NumberParts<'a> {
    negative: bool,
    currency: bool,
    digits: std::borrow::Cow<'a, str>,
}

fn split_number(raw: &str) -> Option<NumberParts<'_>> {
    let mut s = raw.trim();
    let mut negative = false;
    let mut currency = false;

    if s.len() >= 2 && s.starts_with('(') && s.ends_with(')') {
        negative = true;
        s = s[1..s.len() - 1].trim();
    }

    for _ in 0..2 {
        if let Some(rest) = s.strip_prefix('-') {
            if negative {
                return None;
            }
            negative = true;
            s = rest.trim_start();
        } else if let Some(rest) = s.strip_prefix('+') {
            s = rest.trim_start();
        }

        if let Some(rest) = strip_currency_prefix(s) {
            currency = true;
            s = rest;
        }
    }

    if let Some(rest) = s.strip_suffix('-') {
        // Format akuntansi: "1.000-"
        if negative {
            return None;
        }
        negative = true;
        s = rest.trim_end();
    }

    if let Some(rest) = strip_currency_suffix(s) {
        currency = true;
        s = rest;
    }

    let digits: std::borrow::Cow<str> = if s.chars().any(char::is_whitespace) {
        // Spasi (termasuk NBSP) di tengah sebagai pemisah ribuan: "1 234,56".
        std::borrow::Cow::Owned(s.chars().filter(|c| !c.is_whitespace()).collect())
    } else {
        std::borrow::Cow::Borrowed(s)
    };

    if digits.is_empty()
        || !digits.bytes().any(|b| b.is_ascii_digit())
        || !digits.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b',')
        || digits.ends_with(['.', ','])
    {
        return None;
    }

    Some(NumberParts {
        negative,
        currency,
        digits,
    })
}

fn strip_currency_prefix(s: &str) -> Option<&str> {
    for symbol in ["$", "€", "£", "¥", "₹", "₩"] {
        if let Some(rest) = s.strip_prefix(symbol) {
            return Some(rest.trim_start());
        }
    }

    for code in ["rp", "idr", "usd"] {
        if s.len() >= code.len() && s.is_char_boundary(code.len()) && s[..code.len()].eq_ignore_ascii_case(code) {
            let rest = &s[code.len()..];
            // Hindari "Rp" menempel pada huruf lain.
            if rest.starts_with(|c: char| c.is_alphabetic()) {
                return None;
            }
            let rest = rest.strip_prefix('.').unwrap_or(rest);
            return Some(rest.trim_start());
        }
    }

    None
}

fn strip_currency_suffix(s: &str) -> Option<&str> {
    for code in ["rp", "idr", "usd", "$", "€"] {
        if s.len() > code.len() && s.is_char_boundary(s.len() - code.len()) {
            let (head, tail) = s.split_at(s.len() - code.len());
            if tail.eq_ignore_ascii_case(code) && head.ends_with(|c: char| c.is_whitespace() || c.is_ascii_digit()) {
                return Some(head.trim_end());
            }
        }
    }
    None
}

/// Pengelompokan ribuan valid: kelompok pertama 1–3 digit, berikutnya 3 digit.
fn valid_grouping(digits: &str, sep: char) -> bool {
    let mut parts = digits.split(sep);
    let first = parts.next().unwrap_or("");

    !first.is_empty() && first.len() <= 3 && parts.all(|part| part.len() == 3)
}

/// Apa yang tersirat sebuah nilai tentang pemisah desimal kolomnya.
pub fn decimal_evidence(raw: &str) -> Option<DecimalSep> {
    let parts = split_number(raw)?;
    let d = parts.digits.as_ref();
    let dots = d.matches('.').count();
    let commas = d.matches(',').count();

    match (dots, commas) {
        (0, 0) => None,
        (_, 0) | (0, _) => {
            let (sep, as_sep) = if commas == 0 {
                ('.', DecimalSep::Dot)
            } else {
                (',', DecimalSep::Comma)
            };
            let other = if sep == '.' {
                DecimalSep::Comma
            } else {
                DecimalSep::Dot
            };

            if dots + commas > 1 {
                // Muncul berkali-kali → pasti pemisah ribuan.
                valid_grouping(d, sep).then_some(other)
            } else {
                let (int, frac) = d.split_once(sep)?;
                let ambiguous = frac.len() == 3 && !int.is_empty() && int.len() <= 3 && int != "0";
                (!ambiguous).then_some(as_sep)
            }
        }
        _ => {
            let last_dot = d.rfind('.');
            let last_comma = d.rfind(',');
            Some(if last_dot > last_comma {
                DecimalSep::Dot
            } else {
                DecimalSep::Comma
            })
        }
    }
}

pub fn parse_number(raw: &str, sep: DecimalSep) -> Option<Number> {
    let parts = split_number(raw)?;
    let d = parts.digits.as_ref();
    let dots = d.matches('.').count();
    let commas = d.matches(',').count();

    let (int_part, frac_part): (String, &str) = match (dots, commas) {
        (0, 0) => (d.to_string(), ""),

        (_, 0) | (0, _) => {
            let (sep_char, is_decimal_for_column) = if commas == 0 {
                ('.', sep == DecimalSep::Dot)
            } else {
                (',', sep == DecimalSep::Comma)
            };
            let is_thousands_for_column = match sep {
                DecimalSep::Unknown => false,
                _ => !is_decimal_for_column,
            };
            let count = dots + commas;

            if count > 1 || is_thousands_for_column {
                if is_decimal_for_column || !valid_grouping(d, sep_char) {
                    return None;
                }
                (d.replace(sep_char, ""), "")
            } else if is_decimal_for_column {
                let (int, frac) = d.split_once(sep_char)?;
                (int.to_string(), frac)
            } else {
                // Kolom tanpa bukti: 3 digit di belakang pemisah = ribuan.
                let (int, frac) = d.split_once(sep_char)?;
                if frac.len() == 3 && !int.is_empty() && int.len() <= 3 && int != "0" {
                    (format!("{int}{frac}"), "")
                } else {
                    (int.to_string(), frac)
                }
            }
        }

        _ => {
            let dot_is_decimal = d.rfind('.') > d.rfind(',');
            let (decimal, thousands) = if dot_is_decimal { ('.', ',') } else { (',', '.') };

            if d.matches(decimal).count() != 1 {
                return None;
            }
            match (sep, dot_is_decimal) {
                (DecimalSep::Dot, false) | (DecimalSep::Comma, true) => return None,
                _ => {}
            }

            let (int, frac) = d.split_once(decimal)?;
            if !valid_grouping(int, thousands) {
                return None;
            }
            (int.replace(thousands, ""), frac)
        }
    };

    let int_part = if int_part.is_empty() { "0".to_string() } else { int_part };
    let text = if frac_part.is_empty() {
        int_part
    } else {
        format!("{int_part}.{frac_part}")
    };

    let mut value: f64 = text.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    if parts.negative {
        value = -value;
    }

    let decimals = frac_part.trim_end_matches('0').len().min(u8::MAX as usize) as u8;

    Some(Number {
        value,
        decimals,
        has_fraction: !frac_part.is_empty(),
        currency: parts.currency,
    })
}

/// "12,5%" → 0.125 dengan `decimals` = digit desimal persentase yang ditampilkan.
pub fn parse_percentage(raw: &str, sep: DecimalSep, require_sign: bool) -> Option<Number> {
    let trimmed = raw.trim();
    let (body, had_sign) = match trimmed.strip_suffix('%') {
        Some(body) => (body, true),
        None => (trimmed, false),
    };

    if require_sign && !had_sign {
        return None;
    }

    let number = parse_number(body, sep)?;
    if number.currency {
        return None;
    }

    Some(Number {
        value: number.value / 100.0,
        decimals: number.decimals,
        has_fraction: number.has_fraction,
        currency: false,
    })
}

/// Nilai yang sebaiknya tetap teks pada mode Otomatis: kode dengan nol di
/// depan ("007"), nomor panjang (NIK/NPWP > 15 digit, melebihi presisi Excel),
/// atau nomor telepon berawalan "+".
pub fn is_code_like(raw: &str) -> bool {
    let s = raw.trim();

    if let Some(rest) = s.strip_prefix('+') {
        return rest.len() >= 6 && rest.bytes().all(|b| b.is_ascii_digit() || b == b' ' || b == b'-');
    }

    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }

    (s.len() > 1 && s.starts_with('0')) || s.len() > 15
}

// ============================================================
// TANGGAL & WAKTU
// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DateOrder {
    /// Hari-bulan-tahun (standar Indonesia).
    #[default]
    Dmy,
    Mdy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateValue {
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
}

pub fn date_order_evidence(raw: &str) -> Option<DateOrder> {
    let (date, _) = split_date_time(raw.trim());
    let (a, b, c) = date_parts(date)?;

    if a.len() == 4 {
        return None;
    }

    if !(1..=2).contains(&a.len()) || !(1..=2).contains(&b.len()) || !matches!(c.len(), 2 | 4) {
        return None;
    }

    let first: u32 = a.parse().ok()?;
    let second: u32 = b.parse().ok()?;

    let order = match (first > 12, second > 12) {
        (true, false) => DateOrder::Dmy,
        (false, true) => DateOrder::Mdy,
        _ => return None,
    };

    // Hanya nilai yang benar-benar tanggal valid pada urutan itu.
    parse_date_part(date, order, false).map(|_| order)
}

fn split_date_time(s: &str) -> (&str, Option<&str>) {
    if let Some(index) = s.find(['T', 't']) {
        let (date, time) = s.split_at(index);
        if date.len() >= 6 && date.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'-' | b'/' | b'.')) {
            return (date, Some(&time[1..]));
        }
    }

    match s.split_once(char::is_whitespace) {
        Some((date, time)) => (date, Some(time.trim())),
        None => (s, None),
    }
}

fn date_parts(s: &str) -> Option<(&str, &str, &str)> {
    let sep = s.chars().find(|c| matches!(c, '-' | '/' | '.'))?;
    let mut it = s.split(sep);
    let parts = (it.next()?, it.next()?, it.next()?);

    if it.next().is_some() {
        return None;
    }

    let all_digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    (all_digits(parts.0) && all_digits(parts.1) && all_digits(parts.2)).then_some(parts)
}

fn parse_date_part(s: &str, order: DateOrder, allow_compact: bool) -> Option<NaiveDate> {
    let (year, month, day) = if let Some((a, b, c)) = date_parts(s) {
        let n = |p: &str| p.parse::<u32>().ok();

        if a.len() == 4 && (1..=2).contains(&b.len()) && (1..=2).contains(&c.len()) {
            (n(a)? as i32, n(b)?, n(c)?)
        } else if (1..=2).contains(&a.len()) && (1..=2).contains(&b.len()) && matches!(c.len(), 2 | 4) {
            let year = n(c)? as i32;
            let year = if c.len() == 2 {
                // Aturan Excel: 00–29 → 2000-an, 30–99 → 1900-an.
                if year < 30 { 2000 + year } else { 1900 + year }
            } else {
                year
            };
            match order {
                DateOrder::Dmy => (year, n(b)?, n(a)?),
                DateOrder::Mdy => (year, n(a)?, n(b)?),
            }
        } else {
            return None;
        }
    } else if allow_compact && s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit()) {
        (s[..4].parse().ok()?, s[4..6].parse().ok()?, s[6..].parse().ok()?)
    } else {
        return None;
    };

    // Rentang yang didukung Excel.
    if !(1900..=9999).contains(&year) {
        return None;
    }

    NaiveDate::from_ymd_opt(year, month, day)
}

fn parse_time_part(s: &str) -> Option<NaiveTime> {
    let mut s = s.trim();

    // Zona waktu sederhana di akhir: "Z" atau "+07:00"/"+0700".
    if let Some(rest) = s.strip_suffix(['Z', 'z']) {
        s = rest.trim_end();
    } else if let Some(index) = s.rfind(['+', '-']) {
        let tz = &s[index + 1..];
        if index > 0 && matches!(tz.len(), 4 | 5) && tz.bytes().all(|b| b.is_ascii_digit() || b == b':') {
            s = s[..index].trim_end();
        }
    }

    let upper = s.to_ascii_uppercase();
    let (s, meridiem) = if let Some(rest) = upper.strip_suffix("AM") {
        (rest.trim_end().to_string(), Some(false))
    } else if let Some(rest) = upper.strip_suffix("PM") {
        (rest.trim_end().to_string(), Some(true))
    } else {
        (upper.clone(), None)
    };

    let sep = if s.contains(':') { ':' } else { '.' };
    let parts: Vec<&str> = s.split(sep).collect();

    if !(2..=3).contains(&parts.len()) {
        return None;
    }

    let digits = |p: &str| (1..=2).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit());
    if !digits(parts[0]) || parts[1].len() != 2 || !digits(parts[1]) {
        return None;
    }

    let mut hour: u32 = parts[0].parse().ok()?;
    let minute: u32 = parts[1].parse().ok()?;
    let (second, nano) = match parts.get(2) {
        None => (0, 0),
        Some(sec) => {
            let (whole, frac) = match sec.split_once(['.', ',']) {
                Some((whole, frac)) => (whole, frac),
                None => (*sec, ""),
            };
            if whole.len() != 2 || !digits(whole) || !frac.bytes().all(|b| b.is_ascii_digit()) || frac.len() > 9 {
                return None;
            }
            let nano = if frac.is_empty() {
                0
            } else {
                format!("{frac:0<9}").parse::<u32>().ok()?
            };
            (whole.parse().ok()?, nano)
        }
    };

    if let Some(pm) = meridiem {
        if !(1..=12).contains(&hour) {
            return None;
        }
        hour = match (pm, hour) {
            (false, 12) => 0,
            (false, h) => h,
            (true, 12) => 12,
            (true, h) => h + 12,
        };
    }

    NaiveTime::from_hms_nano_opt(hour, minute, second, nano)
}

pub fn parse_date(raw: &str, order: DateOrder, allow_compact: bool) -> Option<DateValue> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }

    let (date, time) = split_date_time(s);
    let date = parse_date_part(date, order, allow_compact)?;
    let time = match time {
        Some(t) if !t.is_empty() => Some(parse_time_part(t)?),
        Some(_) => return None,
        None => None,
    };

    Some(DateValue { date, time })
}

// ============================================================
// PROFIL KOLOM
// ============================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Integer,
    Decimal,
    Currency,
    Percentage,
    Date,
    DateTime,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Text => "Teks",
            Kind::Integer => "Bilangan bulat",
            Kind::Decimal => "Desimal",
            Kind::Currency => "Mata uang",
            Kind::Percentage => "Persentase",
            Kind::Date => "Tanggal",
            Kind::DateTime => "Tanggal & waktu",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnProfile {
    pub kind: Kind,
    pub decimal_sep: DecimalSep,
    pub date_order: DateOrder,
    /// Digit desimal bermakna terbanyak di antara nilai angka kolom ini.
    pub max_decimals: u8,
    /// Ada nilai angka yang ditulis dengan bagian desimal di sumber.
    pub has_fraction: bool,
    /// Digit desimal terbanyak di antara nilai persentase.
    pub pct_decimals: u8,
    pub has_seconds: bool,
    pub has_fraction_seconds: bool,
    /// Baris pertama tidak cocok dengan tipe kolom (kemungkinan judul).
    pub header_mismatch: bool,
    pub non_empty: usize,
}

#[derive(Default)]
struct Counts {
    non_empty: usize,
    code: usize,
    integer: usize,
    decimal: usize,
    currency: usize,
    percentage: usize,
    date: usize,
    datetime: usize,
}

impl Counts {
    fn kind(&self) -> Kind {
        let n = self.non_empty;
        let numeric = self.integer + self.decimal + self.currency;

        if n == 0 || self.code > 0 {
            Kind::Text
        } else if self.date + self.datetime == n {
            if self.datetime > 0 { Kind::DateTime } else { Kind::Date }
        } else if self.percentage == n {
            Kind::Percentage
        } else if numeric == n {
            if self.currency > 0 {
                Kind::Currency
            } else if self.decimal > 0 {
                Kind::Decimal
            } else {
                Kind::Integer
            }
        } else {
            Kind::Text
        }
    }
}

/// Profil semua kolom; kolom dianalisis paralel di beberapa thread.
pub fn profile_table(table: &Table) -> Vec<ColumnProfile> {
    let columns = table.columns();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(columns.max(1));

    if threads <= 1 || table.rows() < 2_000 {
        return (0..columns).map(|col| profile_column(table, col)).collect();
    }

    let mut profiles: Vec<Option<ColumnProfile>> = vec![None; columns];
    std::thread::scope(|scope| {
        let chunk = columns.div_ceil(threads);
        for (index, slots) in profiles.chunks_mut(chunk).enumerate() {
            scope.spawn(move || {
                for (offset, slot) in slots.iter_mut().enumerate() {
                    *slot = Some(profile_column(table, index * chunk + offset));
                }
            });
        }
    });

    profiles.into_iter().map(|p| p.expect("profil kolom")).collect()
}

/// Pemeriksaan murah sebelum parsing tanggal: diawali digit dan memuat
/// pemisah tanggal di 5 karakter pertama (format ringkas tidak dideteksi otomatis).
fn may_be_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 6
        && bytes[0].is_ascii_digit()
        && bytes[..bytes.len().min(5)].iter().any(|b| matches!(b, b'-' | b'/' | b'.'))
}

fn profile_column(table: &Table, col: usize) -> ColumnProfile {
    let rows = table.rows();

    // Tahap 1: kumpulkan bukti pemisah desimal dan urutan tanggal.
    let (mut dot, mut comma, mut dmy, mut mdy) = (0usize, 0usize, 0usize, 0usize);

    for row in 0..rows {
        let value = table.cell(row, col);
        if value.is_empty() {
            continue;
        }
        match decimal_evidence(value) {
            Some(DecimalSep::Dot) => dot += 1,
            Some(DecimalSep::Comma) => comma += 1,
            _ => {}
        }
        if !may_be_date(value) {
            continue;
        }
        match date_order_evidence(value) {
            Some(DateOrder::Dmy) => dmy += 1,
            Some(DateOrder::Mdy) => mdy += 1,
            None => {}
        }
    }

    let decimal_sep = match (dot, comma) {
        (0, 0) => DecimalSep::Unknown,
        (d, c) if d >= c => DecimalSep::Dot,
        _ => DecimalSep::Comma,
    };
    let date_order = if mdy > dmy { DateOrder::Mdy } else { DateOrder::Dmy };

    // Tahap 2: klasifikasi setiap nilai (baris pertama dihitung terpisah).
    let mut first = Counts::default();
    let mut rest = Counts::default();
    let mut max_decimals = 0u8;
    let mut has_fraction = false;
    let mut pct_decimals = 0u8;
    let mut has_seconds = false;
    let mut has_fraction_seconds = false;

    for row in 0..rows {
        let value = table.cell(row, col);
        if value.is_empty() {
            continue;
        }

        let counts = if row == 0 { &mut first } else { &mut rest };
        counts.non_empty += 1;

        let number = parse_number(value, decimal_sep);
        if let Some(number) = number {
            max_decimals = max_decimals.max(number.decimals);
            has_fraction |= number.has_fraction;
        }

        if is_code_like(value) {
            counts.code += 1;
        } else if let Some(date) = may_be_date(value)
            .then(|| parse_date(value, date_order, false))
            .flatten()
        {
            match date.time {
                Some(time) => {
                    counts.datetime += 1;
                    has_seconds |= time.second() != 0;
                    has_fraction_seconds |= time.nanosecond() != 0;
                }
                None => counts.date += 1,
            }
        } else if let Some(pct) = value
            .ends_with('%')
            .then(|| parse_percentage(value, decimal_sep, true))
            .flatten()
        {
            counts.percentage += 1;
            pct_decimals = pct_decimals.max(pct.decimals);
        } else if let Some(number) = number {
            if number.currency {
                counts.currency += 1;
            } else if number.has_fraction {
                // Sumber sudah desimal (termasuk "40300.0") → tetap desimal.
                counts.decimal += 1;
            } else {
                counts.integer += 1;
            }
        }
    }

    has_seconds |= has_fraction_seconds;

    let rest_kind = rest.kind();
    let (kind, header_mismatch) = if rest.non_empty == 0 {
        (first.kind(), false)
    } else if rest_kind == Kind::Text || first.non_empty == 0 {
        (rest_kind, false)
    } else {
        let all = Counts {
            non_empty: first.non_empty + rest.non_empty,
            code: first.code + rest.code,
            integer: first.integer + rest.integer,
            decimal: first.decimal + rest.decimal,
            currency: first.currency + rest.currency,
            percentage: first.percentage + rest.percentage,
            date: first.date + rest.date,
            datetime: first.datetime + rest.datetime,
        };
        let all_kind = all.kind();

        if all_kind == Kind::Text {
            // Hanya baris pertama yang tidak cocok → anggap judul kolom.
            (rest_kind, true)
        } else {
            (all_kind, false)
        }
    };

    ColumnProfile {
        kind,
        decimal_sep,
        date_order,
        max_decimals,
        has_fraction,
        pct_decimals,
        has_seconds,
        has_fraction_seconds,
        header_mismatch,
        non_empty: first.non_empty + rest.non_empty,
    }
}

// ============================================================
// TEST
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::parse_bytes;

    fn num(raw: &str, sep: DecimalSep) -> Option<f64> {
        parse_number(raw, sep).map(|n| n.value)
    }

    #[test]
    fn numbers_unknown_separator() {
        let u = DecimalSep::Unknown;
        assert_eq!(num("40300.0", u), Some(40300.0));
        assert_eq!(num("1.234", u), Some(1234.0));
        assert_eq!(num("1.234.567", u), Some(1_234_567.0));
        assert_eq!(num("1,234,567", u), Some(1_234_567.0));
        assert_eq!(num("1.234,56", u), Some(1234.56));
        assert_eq!(num("1,234.56", u), Some(1234.56));
        assert_eq!(num("0.123", u), Some(0.123));
        assert_eq!(num("50,00", u), Some(50.0));
        assert_eq!(num("-12", u), Some(-12.0));
        assert_eq!(num("(1.234,50)", u), Some(-1234.5));
        assert_eq!(num("1.000-", u), Some(-1000.0));
        assert_eq!(num("Rp 1.500.000", u), Some(1_500_000.0));
        assert_eq!(num("Rp. 2.500,75", u), Some(2500.75));
        assert_eq!(num("-Rp 5.000", u), Some(-5000.0));
        assert_eq!(num("1 234 567,5", u), Some(1_234_567.5));
        assert_eq!(num(".5", u), Some(0.5));
        assert_eq!(num("1.2.3", u), None);
        assert_eq!(num("12.34.5", u), None);
        assert_eq!(num("1,23,456", u), None);
        assert_eq!(num("5.", u), None);
        assert_eq!(num("abc", u), None);
        assert_eq!(num("KTP- 3263", u), None);
        assert_eq!(num("Rupiah", u), None);
        assert_eq!(num("--5", u), None);
        assert_eq!(num("1e5", u), None);
    }

    #[test]
    fn numbers_with_column_separator() {
        assert_eq!(num("1.500", DecimalSep::Dot), Some(1.5));
        assert_eq!(num("1,500", DecimalSep::Dot), Some(1500.0));
        assert_eq!(num("1,5", DecimalSep::Dot), None);
        assert_eq!(num("1.500", DecimalSep::Comma), Some(1500.0));
        assert_eq!(num("1,500", DecimalSep::Comma), Some(1.5));
        assert_eq!(num("1.234,5", DecimalSep::Dot), None);
        assert_eq!(num("1,234.5", DecimalSep::Comma), None);
        assert_eq!(num("12.5", DecimalSep::Comma), None);
    }

    #[test]
    fn number_decimals() {
        let d = |raw| parse_number(raw, DecimalSep::Dot).unwrap().decimals;
        assert_eq!(d("40300.0"), 0);
        assert_eq!(d("12.50"), 1);
        assert_eq!(d("0.125"), 3);
        assert_eq!(d("7"), 0);
    }

    #[test]
    fn evidence() {
        assert_eq!(decimal_evidence("40300.0"), Some(DecimalSep::Dot));
        assert_eq!(decimal_evidence("1.234"), None);
        assert_eq!(decimal_evidence("1.234.567"), Some(DecimalSep::Comma));
        assert_eq!(decimal_evidence("12,5"), Some(DecimalSep::Comma));
        assert_eq!(decimal_evidence("0.500"), Some(DecimalSep::Dot));
        assert_eq!(decimal_evidence("1234.567"), Some(DecimalSep::Dot));
    }

    #[test]
    fn codes() {
        assert!(is_code_like("007"));
        assert!(is_code_like("6471012345678901"));
        assert!(is_code_like("+62 812-3456-7890"));
        assert!(!is_code_like("0"));
        assert!(!is_code_like("0.0"));
        assert!(!is_code_like("123456"));
    }

    #[test]
    fn percentages() {
        let p = parse_percentage("12,5%", DecimalSep::Comma, true).unwrap();
        assert!((p.value - 0.125).abs() < 1e-12);
        assert_eq!(p.decimals, 1);
        assert!(parse_percentage("12", DecimalSep::Unknown, true).is_none());
        assert_eq!(parse_percentage("50", DecimalSep::Unknown, false).unwrap().value, 0.5);
    }

    fn date(raw: &str, order: DateOrder) -> Option<(String, Option<String>)> {
        parse_date(raw, order, false).map(|d| {
            (
                d.date.format("%Y-%m-%d").to_string(),
                d.time.map(|t| t.format("%H:%M:%S%.3f").to_string()),
            )
        })
    }

    #[test]
    fn dates() {
        let dmy = DateOrder::Dmy;
        let ymd = |s: &str| Some((s.to_string(), None));
        assert_eq!(date("05-12-2025", dmy), ymd("2025-12-05"));
        assert_eq!(date("5/1/2025", dmy), ymd("2025-01-05"));
        assert_eq!(date("05.12.2025", dmy), ymd("2025-12-05"));
        assert_eq!(date("2025-12-05", dmy), ymd("2025-12-05"));
        assert_eq!(date("2025/1/5", dmy), ymd("2025-01-05"));
        assert_eq!(date("05-12-25", dmy), ymd("2025-12-05"));
        assert_eq!(date("05-12-95", dmy), ymd("1995-12-05"));
        assert_eq!(date("12/31/2025", DateOrder::Mdy), ymd("2025-12-31"));
        assert_eq!(date("31-02-2025", dmy), None);
        assert_eq!(date("13-13-2025", dmy), None);
        assert_eq!(date("05-12-025", dmy), None);
        assert_eq!(date("2025-12", dmy), None);
        assert_eq!(date("1.234.567", dmy), None);
        assert_eq!(date("40300.0", dmy), None);
        assert_eq!(date("20251205", dmy), None);
        assert_eq!(
            parse_date("20251205", dmy, true).map(|d| d.date),
            NaiveDate::from_ymd_opt(2025, 12, 5)
        );
        assert_eq!(date("01-01-1899", dmy), None);
    }

    #[test]
    fn datetimes() {
        let dmy = DateOrder::Dmy;
        let dt = |d: &str, t: &str| Some((d.to_string(), Some(t.to_string())));
        assert_eq!(date("05-12-2025 13:45", dmy), dt("2025-12-05", "13:45:00.000"));
        assert_eq!(date("05-12-2025 13:45:07", dmy), dt("2025-12-05", "13:45:07.000"));
        assert_eq!(date("2025-12-05T13:45:07.250Z", dmy), dt("2025-12-05", "13:45:07.250"));
        assert_eq!(date("2025-12-05T13:45:07+07:00", dmy), dt("2025-12-05", "13:45:07.000"));
        assert_eq!(date("2025-12-05 13:45:07.123456", dmy), dt("2025-12-05", "13:45:07.123"));
        assert_eq!(date("05/12/2025 01:05 PM", dmy), dt("2025-12-05", "13:05:00.000"));
        assert_eq!(date("05/12/2025 12:00 AM", dmy), dt("2025-12-05", "00:00:00.000"));
        assert_eq!(date("05-12-2025 13.45", dmy), dt("2025-12-05", "13:45:00.000"));
        assert_eq!(date("05-12-2025 24:00", dmy), None);
        assert_eq!(date("05-12-2025 13:60", dmy), None);
        assert_eq!(date("05-12-2025 abc", dmy), None);
        assert_eq!(date("05-12-2025 13:4", dmy), None);
    }

    fn kinds(data: &str) -> Vec<Kind> {
        profile_table(&parse_bytes(data.as_bytes()).unwrap())
            .iter()
            .map(|p| p.kind)
            .collect()
    }

    #[test]
    fn profile_sample_dat() {
        let data = "05-12-2025|KTP- 3263|24|40300.0|40300.0|0.0|\n\
                    05-12-2025|KTP- 3264|1|35000.0|0.0|0.0|\n\
                    14-02-2025|KTBBI4339|1|86592.0|0.0|12.5|\n";
        assert_eq!(
            kinds(data),
            vec![Kind::Date, Kind::Text, Kind::Integer, Kind::Decimal, Kind::Decimal, Kind::Decimal]
        );

        let table = parse_bytes(data.as_bytes()).unwrap();
        let profiles = profile_table(&table);
        assert_eq!(profiles[3].decimal_sep, DecimalSep::Dot);
        assert_eq!(profiles[5].max_decimals, 1);
    }

    #[test]
    fn profile_mixed() {
        assert_eq!(kinds("001\n002\n010\n"), vec![Kind::Text]);
        assert_eq!(kinds("1.500\n2.000\n12,5\n"), vec![Kind::Decimal]);
        assert_eq!(kinds("05-12-2025\n05-12-2025 10:00\n"), vec![Kind::DateTime]);
        assert_eq!(kinds("12%\n7,5%\n"), vec![Kind::Percentage]);
        assert_eq!(kinds("Rp 1.000\n2.000\n"), vec![Kind::Currency]);
        assert_eq!(kinds("1\nabc\n3\n"), vec![Kind::Text]);
        assert_eq!(kinds("100.0\n25.0\n"), vec![Kind::Decimal]);
        assert_eq!(kinds("100\n25\n"), vec![Kind::Integer]);
        assert_eq!(kinds("1.000\n2.500.000\n"), vec![Kind::Integer]);
        assert_eq!(kinds("12/31/2025\n01/02/2025\n"), vec![Kind::Date]);
    }

    #[test]
    fn profile_header_row() {
        let table = parse_bytes(b"tanggal;jumlah\n05-12-2025;1.000\n06-12-2025;2.500\n").unwrap();
        let profiles = profile_table(&table);
        assert_eq!(profiles[0].kind, Kind::Date);
        assert_eq!(profiles[1].kind, Kind::Integer);
        assert!(profiles[0].header_mismatch && profiles[1].header_mismatch);

        let table = parse_bytes(b"12/31/2025\n01/02/2025\n").unwrap();
        assert_eq!(profile_table(&table)[0].date_order, DateOrder::Mdy);
    }
}

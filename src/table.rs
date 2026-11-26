//! Pembacaan file DAT/TXT/CSV menjadi tabel sel teks.
//!
//! Seluruh isi sel disimpan dalam satu buffer `String` dengan offset per sel,
//! sehingga file ratusan ribu baris tetap hemat memori.

use std::fs;
use std::path::Path;

pub struct Table {
    text: String,
    /// Offset akhir setiap sel di `text` (awal sel = akhir sel sebelumnya).
    cell_ends: Vec<u32>,
    /// Indeks sel pertama untuk setiap baris; panjang = jumlah baris + 1.
    row_starts: Vec<u32>,
    columns: usize,
    pub delimiter: char,
    pub encoding: &'static str,
}

impl Table {
    fn new(delimiter: char, encoding: &'static str) -> Self {
        Self {
            text: String::new(),
            cell_ends: Vec::new(),
            row_starts: vec![0],
            columns: 0,
            delimiter,
            encoding,
        }
    }

    pub fn rows(&self) -> usize {
        self.row_starts.len() - 1
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn cell(&self, row: usize, col: usize) -> &str {
        let first = self.row_starts[row] as usize;
        let len = self.row_starts[row + 1] as usize - first;

        if col >= len {
            return "";
        }

        let index = first + col;
        let start = if index == 0 {
            0
        } else {
            self.cell_ends[index - 1] as usize
        };

        &self.text[start..self.cell_ends[index] as usize]
    }

    fn push_cell(&mut self, value: &str) -> Result<(), String> {
        self.text.push_str(value);
        let end = u32::try_from(self.text.len()).map_err(|_| "File terlalu besar (> 4 GB).")?;
        self.cell_ends.push(end);
        Ok(())
    }

    fn end_row(&mut self) -> Result<(), String> {
        let count = u32::try_from(self.cell_ends.len()).map_err(|_| "Jumlah sel terlalu banyak.")?;
        let len = (count - self.row_starts.last().copied().unwrap_or(0)) as usize;

        self.columns = self.columns.max(len);
        self.row_starts.push(count);
        Ok(())
    }
}

// ============================================================
// PEMBACAAN FILE
// ============================================================

pub fn read_table(path: &Path) -> Result<Table, String> {
    let bytes = fs::read(path).map_err(|error| format!("Tidak dapat membaca file: {error}"))?;
    parse_bytes(&bytes)
}

pub fn parse_bytes(bytes: &[u8]) -> Result<Table, String> {
    let (content, encoding) = decode_text(bytes)?;
    let (delimiter, has_terminator) = sniff_delimiter(&content);

    let mut table = Table::new(delimiter, encoding);
    // File berpemisah pipa (DAT) tidak memakai kutip; tanda " diperlakukan biasa.
    let quotes = delimiter != '|';
    parse_content(&content, delimiter, has_terminator, quotes, &mut table)?;

    if table.rows() == 0 {
        return Err("File tidak memiliki data.".to_string());
    }

    Ok(table)
}

/// Parser berbasis aliran karakter: mendukung field ber-kutip ("a|b", "a""b")
/// termasuk baris baru di dalam kutip, CRLF/LF/CR, dan delimiter penutup baris.
fn parse_content(
    content: &str,
    delimiter: char,
    has_terminator: bool,
    quotes: bool,
    table: &mut Table,
) -> Result<(), String> {
    let mut fields: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut line_has_content = false;

    let mut chars = content.chars().peekable();

    let finish_field = |fields: &mut Vec<String>, field: &mut String, quoted: &mut bool| {
        let value = if *quoted {
            std::mem::take(field)
        } else {
            let trimmed = field.trim().to_string();
            field.clear();
            trimmed
        };
        fields.push(value);
        *quoted = false;
    };

    let finish_row = |fields: &mut Vec<String>,
                      line_has_content: &mut bool,
                      table: &mut Table|
     -> Result<(), String> {
        if *line_has_content {
            // Buang satu field kosong terakhir bila file memakai delimiter penutup.
            if has_terminator
                && fields.len() > 1
                && fields.last().is_some_and(String::is_empty)
            {
                fields.pop();
            }

            for value in fields.iter() {
                table.push_cell(value)?;
            }
            table.end_row()?;
        }

        fields.clear();
        *line_has_content = false;
        Ok(())
    };

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }

        match c {
            '"' if quotes && !quoted && field.trim().is_empty() => {
                field.clear();
                quoted = true;
                in_quotes = true;
                line_has_content = true;
            }
            c if c == delimiter => {
                finish_field(&mut fields, &mut field, &mut quoted);
                line_has_content = true;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                finish_field(&mut fields, &mut field, &mut quoted);
                finish_row(&mut fields, &mut line_has_content, table)?;
            }
            c => {
                // Teks sesudah kutip penutup tetap diterima (toleran).
                if !c.is_whitespace() {
                    line_has_content = true;
                }
                field.push(c);
            }
        }
    }

    finish_field(&mut fields, &mut field, &mut quoted);
    finish_row(&mut fields, &mut line_has_content, table)
}

/// Tentukan delimiter dari sampel baris. Prioritas: `|`, tab, `;`, `,`;
/// kandidat pertama yang muncul di >= 80% record dipilih.
/// Juga mendeteksi apakah setiap baris diakhiri delimiter (`a|b|`).
fn sniff_delimiter(content: &str) -> (char, bool) {
    let mut end = content.len().min(256 * 1024);
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    let head = &content[..end];

    let mut chosen = '|';

    for candidate in ['|', '\t', ';', ','] {
        let records = sample_records(head, candidate != '|');
        let with = records
            .iter()
            .filter(|record| count_outside_quotes(record, candidate, candidate != '|') > 0)
            .count();

        if !records.is_empty() && with * 10 >= records.len() * 8 {
            chosen = candidate;
            break;
        }
    }

    let records = sample_records(head, chosen != '|');
    let terminated = records.iter().filter(|r| r.ends_with(chosen)).count();
    let has_terminator = !records.is_empty() && terminated * 10 >= records.len() * 9;

    (chosen, has_terminator)
}

/// Hingga 200 record tidak kosong; dengan `quotes`, baris baru di dalam
/// kutip tidak memutus record.
fn sample_records(text: &str, quotes: bool) -> Vec<&str> {
    let mut records = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;

    for (i, c) in text.char_indices() {
        match c {
            '"' if quotes => in_quotes = !in_quotes,
            '\n' | '\r' if !in_quotes => {
                let record = text[start..i].trim();
                if !record.is_empty() {
                    records.push(record);
                    if records.len() >= 200 {
                        return records;
                    }
                }
                start = i + 1;
            }
            _ => {}
        }
    }

    let record = text[start..].trim();
    if !record.is_empty() && records.len() < 200 {
        records.push(record);
    }
    records
}

fn count_outside_quotes(line: &str, delimiter: char, quotes: bool) -> usize {
    let mut in_quotes = false;
    let mut count = 0;

    for c in line.chars() {
        if c == '"' && quotes {
            in_quotes = !in_quotes;
        } else if c == delimiter && !in_quotes {
            count += 1;
        }
    }

    count
}

// ============================================================
// ENCODING
// ============================================================

fn decode_text(bytes: &[u8]) -> Result<(String, &'static str), String> {
    if let Some(content) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(content.to_vec())
            .map(|text| (text, "UTF-8"))
            .map_err(|error| format!("Encoding UTF-8 tidak valid: {error}"));
    }

    if let Some(content) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(content, u16::from_le_bytes).map(|text| (text, "UTF-16 LE"));
    }

    if let Some(content) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(content, u16::from_be_bytes).map(|text| (text, "UTF-16 BE"));
    }

    // UTF-16 tanpa BOM: banyak byte nol di posisi ganjil/genap.
    if bytes.len() >= 4 {
        let sample = &bytes[..bytes.len().min(4096) & !1];
        let zeros_odd = sample.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
        let zeros_even = sample.iter().step_by(2).filter(|&&b| b == 0).count();
        let half = sample.len() / 2;

        if zeros_odd * 10 >= half * 9 {
            return decode_utf16(bytes, u16::from_le_bytes).map(|text| (text, "UTF-16 LE"));
        }
        if zeros_even * 10 >= half * 9 {
            return decode_utf16(bytes, u16::from_be_bytes).map(|text| (text, "UTF-16 BE"));
        }
    }

    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => Ok((text, "UTF-8")),
        Err(_) => Ok((bytes.iter().map(|&b| cp1252(b)).collect(), "Windows-1252")),
    }
}

fn decode_utf16(bytes: &[u8], to_unit: fn([u8; 2]) -> u16) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("File UTF-16 memiliki panjang byte tidak valid.".into());
    }

    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| to_unit([chunk[0], chunk[1]]))
        .collect();

    String::from_utf16(&units).map_err(|error| format!("Encoding UTF-16 tidak valid: {error}"))
}

fn cp1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];

    match byte {
        0x80..=0x9F => HIGH[(byte - 0x80) as usize],
        _ => byte as char,
    }
}

// ============================================================
// TEST
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(table: &Table) -> Vec<Vec<String>> {
        (0..table.rows())
            .map(|r| {
                (0..table.columns())
                    .map(|c| table.cell(r, c).to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn pipe_with_terminator() {
        let table = parse_bytes(b"05-12-2025|KTP- 3263|24|40300.0||\n05-12-2025|X|1|0.0|5|\n").unwrap();
        assert_eq!(table.delimiter, '|');
        assert_eq!(table.columns(), 5);
        assert_eq!(
            rows(&table),
            vec![
                vec!["05-12-2025", "KTP- 3263", "24", "40300.0", ""],
                vec!["05-12-2025", "X", "1", "0.0", "5"],
            ]
        );
    }

    #[test]
    fn pipe_without_terminator_keeps_trailing_empty() {
        let table = parse_bytes(b"a|b|\r\nc|d|e\r\nf|g|h\r\n").unwrap();
        assert_eq!(table.columns(), 3);
        assert_eq!(table.cell(0, 2), "");
        assert_eq!(table.cell(1, 2), "e");
    }

    #[test]
    fn csv_quotes_and_blank_lines() {
        let table = parse_bytes(b"nama,nilai\n\"Budi, S.E.\",\"1.234,5\"\n\n\"a \"\"b\"\"\nc\",2\n").unwrap();
        assert_eq!(table.delimiter, ',');
        assert_eq!(
            rows(&table),
            vec![
                vec!["nama", "nilai"],
                vec!["Budi, S.E.", "1.234,5"],
                vec!["a \"b\"\nc", "2"],
            ]
        );
    }

    #[test]
    fn pipe_ignores_quotes() {
        let table = parse_bytes(b"\"ukuran 5|2\n").unwrap();
        assert_eq!(table.cell(0, 0), "\"ukuran 5");
        assert_eq!(table.cell(0, 1), "2");
    }

    #[test]
    fn semicolon_preferred_over_comma() {
        let table = parse_bytes(b"a;1,5\nb;2,25\n").unwrap();
        assert_eq!(table.delimiter, ';');
        assert_eq!(table.cell(1, 1), "2,25");
    }

    #[test]
    fn utf16_and_cp1252() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "a|é\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let table = parse_bytes(&bytes).unwrap();
        assert_eq!(table.cell(0, 1), "é");

        let table = parse_bytes(&[b'a', b'|', 0xE9, 0x80]).unwrap();
        assert_eq!(table.encoding, "Windows-1252");
        assert_eq!(table.cell(0, 1), "é€");
    }
}

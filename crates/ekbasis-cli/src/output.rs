//! Small rendering helpers for CLI output.
//!
//! Two rules keep the output readable in any terminal:
//!
//! * Tables are pure ASCII, so column widths stay predictable.
//! * Line-oriented output may use markers such as `[ok]` / `[!!]` / `[--]`.

/// Title line plus a rule, shown at the start of every command.
pub fn banner(title: &str) -> String {
    let rule = "-".repeat(title.chars().count().max(12));
    format!("AION · {title}\n{rule}")
}

/// `key  value` line with the key padded to a fixed column.
pub fn kv(key: &str, value: impl std::fmt::Display) -> String {
    format!("{:<14}{}", key, value)
}

fn pad(text: &str, width: usize, right_align: bool) -> String {
    let length = text.chars().count();
    if length >= width {
        return text.to_string();
    }
    let filler = " ".repeat(width - length);
    if right_align {
        format!("{filler}{text}")
    } else {
        format!("{text}{filler}")
    }
}

/// Builds an ASCII table; the first column is left aligned, all others right aligned.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    if headers.is_empty() {
        return String::new();
    }
    let mut widths: Vec<usize> = headers.iter().map(|header| header.chars().count()).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(cell.chars().count());
            }
        }
    }

    let mut out = String::new();
    let header_cells: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(index, header)| pad(header, widths[index], index > 0))
        .collect();
    out.push_str(&header_cells.join("  "));
    out.push('\n');
    out.push_str(&"-".repeat(widths.iter().sum::<usize>() + 2 * (widths.len() - 1)));
    out.push('\n');
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(index, cell)| pad(cell, widths.get(index).copied().unwrap_or(0), index > 0))
            .collect();
        out.push_str(&cells.join("  "));
        out.push('\n');
    }
    out
}

/// Indents every line of a block by `spaces`.
pub fn indent(text: &str, spaces: usize) -> String {
    let padding = " ".repeat(spaces);
    text.lines()
        .map(|line| format!("{padding}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

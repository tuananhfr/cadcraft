//! Names of the SHX fonts a drawing program ships with: a drawing that asks for one of them is
//! not worth a "font missing" notice, the built-in stroke font stands in silently.

pub const STANDARD_SHX: &[&str] = &[
    "txt", "simplex", "romans", "romand", "romanc", "romant", "complex", "italic", "italicc", "italict", "scripts", "scriptc", "greeks", "greekc",
    "gothice", "gothicg", "gothici", "monotxt", "syastro", "symap", "symath", "symeteo", "symusic", "isocp", "isocp2", "isocp3", "isoct", "isoct2",
    "isoct3", "isocpeur", "isoproxy",
];

/// Whether `name` (a font file name, with or without `.shx` and a folder) is a standard one: its
/// stem is in [`STANDARD_SHX`], case-insensitively, or starts with `iso` (the ISO family).
pub fn is_standard_shx(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or("").trim().to_ascii_lowercase();
    let stem = file.strip_suffix(".shx").unwrap_or(&file);
    STANDARD_SHX.contains(&stem) || stem.starts_with("iso")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_names_ignore_case_extension_and_folder() {
        for n in ["romans.shx", "ROMANS.SHX", "Romans", "C:\\Fonts\\txt.shx", "/x/Simplex.shx", "isocp.shx", "ISOCP7.shx", "isoNew", "monotxt"] {
            assert!(is_standard_shx(n), "{n}");
        }
        for n in ["vhtime.shx", "romans2.shx", "xromans.shx", "", ".shx", "arial.ttf", "txt.ttf", "my iso.shx"] {
            assert!(!is_standard_shx(n), "{n}");
        }
    }

    #[test]
    fn the_list_is_lower_case_and_has_no_duplicates() {
        let mut seen = std::collections::BTreeSet::new();
        for n in STANDARD_SHX {
            assert_eq!(*n, n.to_ascii_lowercase());
            assert!(seen.insert(*n), "{n}");
        }
        assert_eq!(STANDARD_SHX.len(), 31);
    }
}

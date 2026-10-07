use std::error::Error;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Parse bytes or binary K/KiB, M/MiB, G/GiB suffixes, checking overflow.
pub fn parse_size(value: &str) -> Result<u64> {
    let value = value.trim().to_ascii_uppercase();
    let end = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    let number: u64 = value[..end].parse()?;
    let multiplier = match &value[end..] {
        "" | "B" => 1,
        "K" | "KB" | "KIB" => 1024,
        "M" | "MB" | "MIB" => 1024 * 1024,
        "G" | "GB" | "GIB" => 1024 * 1024 * 1024,
        suffix => return Err(format!("unknown size suffix: {suffix}").into()),
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| "size overflow".into())
}

/// Reject duplicate entries to avoid accidentally running a case twice.
pub fn parse_unique<T: PartialEq>(
    value: &str,
    parse: impl Fn(&str) -> Result<T>,
) -> Result<Vec<T>> {
    let mut result = Vec::new();
    for token in value.split(',') {
        let item = parse(token.trim())?;
        if result.contains(&item) {
            return Err("duplicate list entry".into());
        }
        result.push(item);
    }
    Ok(result)
}

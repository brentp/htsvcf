/// Parse a region string like `chr`, `chr:100`, or `chr:100-200`.
///
/// Input coordinates are 1-based inclusive, output is 0-based (start) and optional 0-based (end).
pub fn parse_region_1based(region: &str) -> Option<(String, u64, Option<u64>)> {
  let (chrom, rest) = region.split_once(':').unwrap_or((region, ""));
  if chrom.is_empty() {
    return None;
  }
  if rest.is_empty() {
    return Some((chrom.to_string(), 0, None));
  }

  let (start_str, end_str) = rest.split_once('-').unwrap_or((rest, ""));
  let start_1 = start_str.replace(',', "").parse::<u64>().ok()?;
  let start_0 = start_1.saturating_sub(1);

  if end_str.is_empty() {
    return Some((chrom.to_string(), start_0, None));
  }

  let end_1 = end_str.replace(',', "").parse::<u64>().ok()?;
  let end_0 = end_1.saturating_sub(1);
  Some((chrom.to_string(), start_0, Some(end_0)))
}

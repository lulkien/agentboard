pub(super) fn format_age(now_ms: i64, timestamp_ms: i64) -> String {
    let elapsed_ms = now_ms.saturating_sub(timestamp_ms).max(0);
    if elapsed_ms < 60_000 {
        "now".to_owned()
    } else if elapsed_ms < 3_600_000 {
        format!("{} min", elapsed_ms / 60_000)
    } else if elapsed_ms < 86_400_000 {
        format!("{} h", elapsed_ms / 3_600_000)
    } else {
        format!("{} d", elapsed_ms / 86_400_000)
    }
}

pub(super) fn format_timestamp(timestamp_ms: i64) -> String {
    let days = timestamp_ms.div_euclid(86_400_000);
    let millis = timestamp_ms.rem_euclid(86_400_000);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = millis / 3_600_000;
    let minute = millis / 60_000 % 60;
    let second = millis / 1_000 % 60;
    let subsecond = millis % 1_000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{subsecond:03}Z")
}

#[cfg(test)]
mod tests {
    use super::format_age;

    #[test]
    fn formats_relative_age_at_each_unit_boundary() {
        assert_eq!(format_age(0, 1), "now");
        assert_eq!(format_age(59_999, 0), "now");
        assert_eq!(format_age(60_000, 0), "1 min");
        assert_eq!(format_age(3_599_999, 0), "59 min");
        assert_eq!(format_age(3_600_000, 0), "1 h");
        assert_eq!(format_age(86_399_999, 0), "23 h");
        assert_eq!(format_age(86_400_000, 0), "1 d");
        assert_eq!(format_age(172_800_000, 0), "2 d");
    }
}

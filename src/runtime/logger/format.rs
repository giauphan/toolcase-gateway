//! Date/time formatting and log message sanitization.

/// Convert seconds since UNIX epoch to an ISO-8601 UTC date string (YYYY-MM-DD).
pub fn format_date(secs: u64) -> String {
    let ts = format_timestamp(secs);
    ts[..10].to_string()
}

/// Convert seconds since UNIX epoch to an ISO-8601 UTC timestamp string.
pub fn format_timestamp(secs: u64) -> String {
    let second = secs % 60;
    let minute = (secs / 60) % 60;
    let hour = (secs / 3600) % 24;
    let mut days = (secs / 86400) as i64;

    days += 719468;
    let era = (if days >= 0 { days } else { days - 146096 }) / 146097;
    let doe = (days - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hour, minute, second
    )
}

pub fn extract_date_from_filename(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    for i in 0..=bytes.len() - 10 {
        let sub = &bytes[i..i + 10];
        if sub[4] == b'-'
            && sub[7] == b'-'
            && sub[0..4].iter().all(u8::is_ascii_digit)
            && sub[5..7].iter().all(u8::is_ascii_digit)
            && sub[8..10].iter().all(u8::is_ascii_digit)
        {
            return Some(&name[i..i + 10]);
        }
    }
    None
}

pub fn parse_date_to_epoch_days(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if s.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (
        s[0..4].parse().ok()?,
        s[5..7].parse().ok()?,
        s[8..10].parse().ok()?,
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1970 {
        return None;
    }
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = (if y_adj >= 0 { y_adj } else { y_adj - 399 }) / 400;
    let yoe = (y_adj - era * 400) as u64;
    let m_adj = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * m_adj + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy as u64;
    let days = era * 146097 + doe as i64 - 719468;
    (days >= 0).then_some(days as u64)
}

/// Sanitize potential secrets (tokens, session cookies, api keys) from log messages.
pub fn sanitize_log_message(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut rest = msg;

    let patterns = [
        ("Bearer ", 7),
        ("sk-", 0),
        ("hatch_sess=", 11),
        ("datr=", 5),
    ];

    while !rest.is_empty() {
        let mut earliest: Option<(usize, &'static str, usize)> = None;
        for (pat, keep_len) in patterns {
            if let Some(pos) = rest.find(pat) {
                match earliest {
                    Some((min_pos, _, _)) if pos < min_pos => {
                        earliest = Some((pos, pat, keep_len));
                    }
                    None => {
                        earliest = Some((pos, pat, keep_len));
                    }
                    _ => {}
                }
            }
        }

        if let Some((pos, pat, keep_len)) = earliest {
            out.push_str(&rest[..pos + keep_len]);
            let after = &rest[pos + keep_len..];
            let val_len = match pat {
                "Bearer " | "sk-" => after
                    .find(|c: char| {
                        c.is_whitespace() || c == '"' || c == ',' || c == ';' || c == '&'
                    })
                    .unwrap_or(after.len()),
                _ => after
                    .find(|c: char| c.is_whitespace() || c == ';' || c == '"')
                    .unwrap_or(after.len()),
            };

            if (pat == "Bearer " || pat == "sk-") && val_len > 8 {
                let keep_prefix = if pat == "Bearer " { 4 } else { 7 };
                let prefix_len = keep_prefix.min(val_len);
                out.push_str(&after[..prefix_len]);
                out.push_str("...[MASKED]");
            } else {
                out.push_str("[MASKED]");
            }
            rest = &after[val_len..];
        } else {
            out.push_str(rest);
            break;
        }
    }
    out
}

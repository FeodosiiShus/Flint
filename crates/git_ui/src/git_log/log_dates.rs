use gpui::SharedString;
use time::{OffsetDateTime, UtcOffset};
use time_format::TimestampFormat;

const TODAY: &str = "Today";
const YESTERDAY: &str = "Yesterday";

const NANOSECONDS_PER_MILLISECOND: i128 = 1_000_000;
const MILLISECONDS_PER_SECOND: i128 = 1_000;
const MILLISECONDS_PER_MINUTE: i128 = 60 * MILLISECONDS_PER_SECOND;
const MILLISECONDS_PER_HOUR: i128 = 60 * MILLISECONDS_PER_MINUTE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrettyDateTime {
    MinutesAgo(i32),
    Today,
    Yesterday,
    DateTime,
}

pub(crate) fn format_pretty_date_time(unix_seconds: i64) -> SharedString {
    let Some((timestamp, now)) = local_timestamp_and_now(unix_seconds) else {
        return SharedString::default();
    };
    pretty_date_time_text(
        classify_pretty_date_time(timestamp, now, now.offset()),
        || time_format::format_time(timestamp),
        || system_date_time(timestamp, now),
    )
}

pub(crate) fn format_date(unix_seconds: i64) -> SharedString {
    local_timestamp_and_now(unix_seconds)
        .map(|(timestamp, now)| time_format::format_date(timestamp, now, false))
        .map(SharedString::from)
        .unwrap_or_default()
}

pub(crate) fn format_time(unix_seconds: i64) -> SharedString {
    local_timestamp_and_now(unix_seconds)
        .map(|(timestamp, _)| time_format::format_time(timestamp))
        .map(SharedString::from)
        .unwrap_or_default()
}

fn local_timestamp_and_now(unix_seconds: i64) -> Option<(OffsetDateTime, OffsetDateTime)> {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let timestamp = OffsetDateTime::from_unix_timestamp(unix_seconds)
        .ok()?
        .checked_to_offset(offset)?;
    let now = OffsetDateTime::now_utc().checked_to_offset(offset)?;
    Some((timestamp, now))
}

fn system_date_time(timestamp: OffsetDateTime, now: OffsetDateTime) -> String {
    time_format::format_local_timestamp(timestamp, now, TimestampFormat::Absolute)
}

fn epoch_milliseconds(date_time: OffsetDateTime) -> i128 {
    date_time
        .unix_timestamp_nanos()
        .div_euclid(NANOSECONDS_PER_MILLISECOND)
}

fn classify_pretty_date_time(
    timestamp: OffsetDateTime,
    now: OffsetDateTime,
    offset: UtcOffset,
) -> PrettyDateTime {
    let delta = epoch_milliseconds(now) - epoch_milliseconds(timestamp);
    if (0..=MILLISECONDS_PER_HOUR + MILLISECONDS_PER_MINUTE).contains(&delta) {
        let minutes = (delta as f64 / MILLISECONDS_PER_MINUTE as f64).round_ties_even() as i32;
        return PrettyDateTime::MinutesAgo(minutes);
    }

    let (Some(current), Some(local)) = (
        now.checked_to_offset(offset),
        timestamp.checked_to_offset(offset),
    ) else {
        return PrettyDateTime::DateTime;
    };
    let current_year = current.year();
    let current_day_of_year = current.ordinal();
    let year = local.year();
    let day_of_year = local.ordinal();

    if current_year == year && current_day_of_year == day_of_year {
        return PrettyDateTime::Today;
    }

    let is_yesterday_on_previous_year = current_year == year + 1
        && current_day_of_year == 1
        && day_of_year == time::util::days_in_year(year);
    let is_yesterday = is_yesterday_on_previous_year
        || (current_year == year && current_day_of_year == day_of_year + 1);
    if is_yesterday {
        return PrettyDateTime::Yesterday;
    }

    PrettyDateTime::DateTime
}

fn pretty_date_time_text(
    pretty_date_time: PrettyDateTime,
    time_text: impl FnOnce() -> String,
    date_time_text: impl FnOnce() -> String,
) -> SharedString {
    match pretty_date_time {
        PrettyDateTime::MinutesAgo(minutes) => minutes_ago_text(minutes),
        PrettyDateTime::Today => format!("{TODAY} {}", time_text()).into(),
        PrettyDateTime::Yesterday => format!("{YESTERDAY} {}", time_text()).into(),
        PrettyDateTime::DateTime => date_time_text().into(),
    }
}

fn minutes_ago_text(minutes: i32) -> SharedString {
    match minutes {
        ..=0 => "Moments ago".into(),
        1 => "A minute ago".into(),
        2..=59 => format!("{minutes} minutes ago").into(),
        _ => "1 hour ago".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Duration;
    use time::macros::{datetime, offset};

    fn classify_in_utc(timestamp: OffsetDateTime, now: OffsetDateTime) -> PrettyDateTime {
        classify_pretty_date_time(timestamp, now, UtcOffset::UTC)
    }

    #[test]
    fn minutes_ago_rounds_half_to_even_up_to_one_hour_and_one_minute() {
        let now = datetime!(2024-03-10 12:00:00 UTC);
        let cases = [
            (0, 0),
            (1, 0),
            (30, 0),
            (60, 1),
            (90, 2),
            (150, 2),
            (210, 4),
            (300, 5),
            (3600, 60),
            (3660, 61),
        ];
        for (seconds_ago, minutes) in cases {
            assert_eq!(
                classify_in_utc(now - Duration::seconds(seconds_ago), now),
                PrettyDateTime::MinutesAgo(minutes),
                "{seconds_ago} seconds ago"
            );
        }
        assert_eq!(
            classify_in_utc(now - Duration::seconds(3661), now),
            PrettyDateTime::Today
        );
    }

    #[test]
    fn minutes_ago_uses_millisecond_precision() {
        assert_eq!(
            classify_in_utc(
                datetime!(2024-03-10 11:59:30 UTC),
                datetime!(2024-03-10 12:00:00.500 UTC)
            ),
            PrettyDateTime::MinutesAgo(1)
        );
        assert_eq!(
            classify_in_utc(
                datetime!(2024-03-10 11:00:00 UTC),
                datetime!(2024-03-10 12:01:00.000 UTC)
            ),
            PrettyDateTime::MinutesAgo(61)
        );
        assert_eq!(
            classify_in_utc(
                datetime!(2024-03-10 11:00:00 UTC),
                datetime!(2024-03-10 12:01:00.001 UTC)
            ),
            PrettyDateTime::Today
        );
    }

    #[test]
    fn minutes_ago_text_follows_choice_format_limits() {
        assert_eq!(minutes_ago_text(0), "Moments ago");
        assert_eq!(minutes_ago_text(1), "A minute ago");
        assert_eq!(minutes_ago_text(2), "2 minutes ago");
        assert_eq!(minutes_ago_text(5), "5 minutes ago");
        assert_eq!(minutes_ago_text(59), "59 minutes ago");
        assert_eq!(minutes_ago_text(60), "1 hour ago");
        assert_eq!(minutes_ago_text(61), "1 hour ago");
    }

    #[test]
    fn future_timestamps_are_never_minutes_ago() {
        let now = datetime!(2024-03-10 12:00:00 UTC);
        assert_eq!(
            classify_in_utc(datetime!(2024-03-10 12:00:00.001 UTC), now),
            PrettyDateTime::Today
        );
        assert_eq!(
            classify_in_utc(now + Duration::seconds(1), now),
            PrettyDateTime::Today
        );
        assert_eq!(
            classify_in_utc(now + Duration::minutes(65), now),
            PrettyDateTime::Today
        );
        assert_eq!(
            classify_in_utc(now + Duration::days(1), now),
            PrettyDateTime::DateTime
        );
    }

    #[test]
    fn today_and_yesterday_use_the_local_calendar_day() {
        let now = datetime!(2024-03-10 20:00 UTC);
        let timestamp = datetime!(2024-03-10 18:00 UTC);
        assert_eq!(classify_in_utc(timestamp, now), PrettyDateTime::Today);
        assert_eq!(
            classify_pretty_date_time(timestamp, now, offset!(+5:00)),
            PrettyDateTime::Yesterday
        );

        let now = datetime!(2024-03-10 03:00 UTC);
        let timestamp = datetime!(2024-03-09 06:00 UTC);
        assert_eq!(classify_in_utc(timestamp, now), PrettyDateTime::Yesterday);
        assert_eq!(
            classify_pretty_date_time(timestamp, now, offset!(-5:00)),
            PrettyDateTime::Today
        );
    }

    #[test]
    fn yesterday_crosses_month_and_year_boundaries() {
        assert_eq!(
            classify_in_utc(
                datetime!(2024-02-29 08:00 UTC),
                datetime!(2024-03-01 10:00 UTC)
            ),
            PrettyDateTime::Yesterday
        );
        assert_eq!(
            classify_in_utc(
                datetime!(2024-12-31 08:00 UTC),
                datetime!(2025-01-01 10:00 UTC)
            ),
            PrettyDateTime::Yesterday
        );
        assert_eq!(
            classify_in_utc(
                datetime!(2023-12-31 08:00 UTC),
                datetime!(2024-01-01 10:00 UTC)
            ),
            PrettyDateTime::Yesterday
        );
        assert_eq!(
            classify_in_utc(
                datetime!(2024-12-30 08:00 UTC),
                datetime!(2025-01-01 10:00 UTC)
            ),
            PrettyDateTime::DateTime
        );
        assert_eq!(
            classify_in_utc(
                datetime!(2024-12-31 08:00 UTC),
                datetime!(2025-01-02 10:00 UTC)
            ),
            PrettyDateTime::DateTime
        );

        let now = datetime!(2024-12-31 20:00 UTC);
        let timestamp = datetime!(2024-12-31 10:00 UTC);
        assert_eq!(classify_in_utc(timestamp, now), PrettyDateTime::Today);
        assert_eq!(
            classify_pretty_date_time(timestamp, now, offset!(+5:00)),
            PrettyDateTime::Yesterday
        );
    }

    #[test]
    fn older_timestamps_fall_back_to_date_time() {
        let now = datetime!(2024-03-10 12:00 UTC);
        assert_eq!(
            classify_in_utc(now - Duration::days(2), now),
            PrettyDateTime::DateTime
        );
        assert_eq!(
            classify_in_utc(datetime!(2023-03-11 12:00 UTC), now),
            PrettyDateTime::DateTime
        );
        assert_eq!(
            classify_in_utc(datetime!(2023-03-10 12:00 UTC), now),
            PrettyDateTime::DateTime
        );
        assert_eq!(
            classify_in_utc(datetime!(2003-12-10 17:00:00 UTC), now),
            PrettyDateTime::DateTime
        );
        assert_eq!(
            classify_in_utc(datetime!(2004-12-08 23:59:59 UTC), now),
            PrettyDateTime::DateTime
        );
    }

    #[test]
    fn pretty_date_time_text_matches_intellij_cases() {
        let now = datetime!(2024-03-10 12:00 UTC);
        let time_text = || "TIME".to_string();
        let date_time_text = || "DATE TIME".to_string();
        let cases = [
            (now - Duration::seconds(1), "Moments ago"),
            (now - Duration::minutes(1), "A minute ago"),
            (now - Duration::minutes(5), "5 minutes ago"),
            (now - Duration::hours(1), "1 hour ago"),
            (now + Duration::minutes(65), "Today TIME"),
            (now - Duration::minutes(65), "Today TIME"),
            (now - Duration::days(1), "Yesterday TIME"),
            (now - Duration::days(2), "DATE TIME"),
        ];
        for (timestamp, expected) in cases {
            assert_eq!(
                pretty_date_time_text(classify_in_utc(timestamp, now), time_text, date_time_text),
                expected,
                "{timestamp}"
            );
        }
    }
}

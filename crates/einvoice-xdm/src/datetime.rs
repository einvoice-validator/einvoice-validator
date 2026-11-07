//! `xs:date` and `xs:dateTime`: parsing, string conversion and comparison, as
//! Saxon-HE 12.10 performs them under its default XSD 1.1 rules (year zero allowed,
//! years up to `2^31 - 1`). Values without a timezone are compared using an implicit
//! timezone supplied by the caller. Durations and date arithmetic are not provided.

use std::cmp::Ordering;
use std::fmt;

use crate::error::{ErrorCode, Result, XdmError};

/// An `xs:date`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
    /// Offset from UTC in minutes.
    timezone: Option<i16>,
}

/// An `xs:dateTime`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DateTime {
    date: Date,
    hour: u8,
    minute: u8,
    second: u8,
    nanosecond: u32,
}

const DAYS_PER_MONTH: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && !(year % 100 == 0 && year % 400 != 0)
}

fn is_valid_date(year: i32, month: u8, day: u8) -> bool {
    let Some(&days) = DAYS_PER_MONTH.get(usize::from(month).wrapping_sub(1)) else {
        return false;
    };
    (day > 0 && day <= days) || (month == 2 && day == 29 && is_leap_year(year))
}

/// Days from 1970-01-01 in the proleptic Gregorian calendar with a year zero.
fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Saxon's `java.util.StringTokenizer` with the delimiters returned as tokens.
fn tokenize<'a>(text: &'a str, delimiters: &str) -> Vec<&'a str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    for (index, c) in text.char_indices() {
        if delimiters.contains(c) {
            if start < index {
                tokens.push(&text[start..index]);
            }
            tokens.push(&text[index..index + c.len_utf8()]);
            start = index + c.len_utf8();
        }
    }
    if start < text.len() {
        tokens.push(&text[start..]);
    }
    tokens
}

/// Saxon's `DurationValue.simpleInteger`: ASCII digits only; `None` when not
/// numeric, `Some(Err(()))` beyond `i32::MAX`.
fn simple_integer(part: &str) -> std::result::Result<i32, SimpleIntegerError> {
    if part.is_empty() {
        return Err(SimpleIntegerError::NotNumeric);
    }
    let mut result: i64 = 0;
    for c in part.chars() {
        let digit = c.to_digit(10).filter(|_| c.is_ascii_digit());
        let Some(digit) = digit else {
            return Err(SimpleIntegerError::NotNumeric);
        };
        result = result * 10 + i64::from(digit);
        if result > i64::from(i32::MAX) {
            return Err(SimpleIntegerError::TooLarge);
        }
    }
    i32::try_from(result).map_err(|_| SimpleIntegerError::TooLarge)
}

#[derive(Debug, Clone, Copy)]
enum SimpleIntegerError {
    NotNumeric,
    TooLarge,
}

fn bad(kind: &str, text: &str, reason: &str) -> XdmError {
    XdmError::new(
        ErrorCode::Forg0001,
        format!("Invalid {kind} \"{text}\" ({reason})"),
    )
}

/// Two-character numeric component.
fn two_digits(part: Option<&&str>, kind: &str, text: &str, what: &str) -> Result<u8> {
    let part = part.ok_or_else(|| bad(kind, text, "Too short"))?;
    if part.chars().count() != 2 {
        return Err(bad(kind, text, &format!("{what} must be two digits")));
    }
    simple_integer(part)
        .ok()
        .and_then(|v| u8::try_from(v).ok())
        .ok_or_else(|| bad(kind, text, &format!("Non-numeric {what} component")))
}

impl Date {
    /// A date from its components.
    ///
    /// # Errors
    ///
    /// `FORG0001` for a day that does not exist or a timezone beyond ±14:00.
    pub fn new(year: i32, month: u8, day: u8, timezone: Option<i16>) -> Result<Self> {
        if !is_valid_date(year, month, day) {
            return Err(XdmError::new(ErrorCode::Forg0001, "Non-existent date"));
        }
        if timezone.is_some_and(|tz| tz.abs() > 14 * 60) {
            return Err(XdmError::new(ErrorCode::Forg0001, "Timezone out of range"));
        }
        Ok(Self {
            year,
            month,
            day,
            timezone,
        })
    }

    /// The year (0 is the year before 1).
    #[must_use]
    pub const fn year(&self) -> i32 {
        self.year
    }

    /// The month, 1 to 12.
    #[must_use]
    pub const fn month(&self) -> u8 {
        self.month
    }

    /// The day of the month.
    #[must_use]
    pub const fn day(&self) -> u8 {
        self.day
    }

    /// The timezone offset in minutes, if any.
    #[must_use]
    pub const fn timezone(&self) -> Option<i16> {
        self.timezone
    }

    /// Parses the lexical form of `xs:date` the way Saxon does
    /// (`GDateValue.setLexicalValue`).
    ///
    /// # Errors
    ///
    /// `FORG0001` for an invalid date, `FODT0001` for a year beyond `2^31 - 1`.
    #[expect(
        clippy::too_many_lines,
        reason = "a literal transcription of the reference parser, step by step"
    )]
    pub fn parse(text: &str) -> Result<Self> {
        const KIND: &str = "date";
        let trimmed = crate::double::trim_xml_whitespace(text);
        let tokens = tokenize(trimmed, "-:+TZ");
        let mut tokens = tokens.iter();
        let mut part = *tokens.next().ok_or_else(|| bad(KIND, text, "Too short"))?;
        let mut era = 1;
        if part == "+" {
            return Err(bad(KIND, text, "Date must not start with '+' sign"));
        }
        if part == "-" {
            era = -1;
            part = tokens
                .next()
                .ok_or_else(|| bad(KIND, text, "No year after '-'"))?;
        }
        let year_length = part.chars().count();
        if year_length < 4 {
            return Err(bad(KIND, text, "Year is less than four digits"));
        }
        if year_length > 4 && part.starts_with('0') {
            return Err(bad(
                KIND,
                text,
                "When year exceeds 4 digits, leading zeroes are not allowed",
            ));
        }
        let year = match simple_integer(part) {
            Ok(v) => v * era,
            Err(SimpleIntegerError::NotNumeric) => {
                return Err(bad(KIND, text, "Non-numeric year component"));
            }
            Err(SimpleIntegerError::TooLarge) => {
                return Err(XdmError::new(
                    ErrorCode::Fodt0001,
                    format!(
                        "Invalid date \"{text}\" (Year is outside the range that Saxon can handle)"
                    ),
                ));
            }
        };
        expect_delimiter(tokens.next(), "-", KIND, text, "Wrong delimiter after year")?;
        let month = two_digits(tokens.next(), KIND, text, "Month")?;
        if !(1..=12).contains(&month) {
            return Err(bad(KIND, text, "Month is out of range"));
        }
        expect_delimiter(
            tokens.next(),
            "-",
            KIND,
            text,
            "Wrong delimiter after month",
        )?;
        let day = two_digits(tokens.next(), KIND, text, "Day")?;
        if !(1..=31).contains(&day) {
            return Err(bad(KIND, text, "Day is out of range"));
        }
        let mut timezone = None;
        if let Some(&delimiter) = tokens.next() {
            match delimiter {
                "T" => return Err(bad(KIND, text, "Value includes time")),
                "Z" => {
                    if tokens.next().is_some() {
                        return Err(bad(KIND, text, "Continues after 'Z'"));
                    }
                    timezone = Some(0);
                }
                "+" | "-" => {
                    let hour_part = *tokens
                        .next()
                        .ok_or_else(|| bad(KIND, text, "Missing timezone"))?;
                    let hour = simple_integer(hour_part)
                        .map_err(|_| bad(KIND, text, "Non-numeric timezone hour component"))?;
                    if hour_part.chars().count() != 2 {
                        return Err(bad(KIND, text, "Timezone hour must be two digits"));
                    }
                    if hour > 14 {
                        return Err(bad(KIND, text, "Timezone hour is out of range"));
                    }
                    let colon = tokens
                        .next()
                        .ok_or_else(|| bad(KIND, text, "No minutes in timezone"))?;
                    if *colon != ":" {
                        return Err(bad(KIND, text, "Wrong delimiter after timezone hour"));
                    }
                    let minute_part = *tokens
                        .next()
                        .ok_or_else(|| bad(KIND, text, "No minutes in timezone"))?;
                    let minute = simple_integer(minute_part)
                        .map_err(|_| bad(KIND, text, "Non-numeric timezone minute component"))?;
                    if minute_part.chars().count() != 2 {
                        return Err(bad(KIND, text, "Timezone minute must be two digits"));
                    }
                    if minute > 59 {
                        return Err(bad(KIND, text, "Timezone minute is out of range"));
                    }
                    if tokens.next().is_some() {
                        return Err(bad(KIND, text, "Continues after timezone"));
                    }
                    let offset = i16::try_from(hour * 60 + minute).unwrap_or(0);
                    timezone = Some(if delimiter == "-" { -offset } else { offset });
                }
                _ => return Err(bad(KIND, text, "Timezone format is incorrect")),
            }
        }
        if !is_valid_date(year, month, day) {
            return Err(bad(KIND, text, "Non-existent date"));
        }
        Ok(Self {
            year,
            month,
            day,
            timezone,
        })
    }

    /// The date at midnight (`xs:dateTime` cast).
    #[must_use]
    pub const fn to_date_time(self) -> DateTime {
        DateTime {
            date: self,
            hour: 0,
            minute: 0,
            second: 0,
            nanosecond: 0,
        }
    }

    /// Compares two dates as instants (their starting midnight), applying the implicit
    /// timezone (minutes) to a value without timezone.
    #[must_use]
    pub fn compare(&self, other: &Self, implicit_timezone: i16) -> Ordering {
        self.to_date_time()
            .compare(&other.to_date_time(), implicit_timezone)
    }

    fn fmt_date(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.year < 0 {
            f.write_str("-")?;
        }
        let year = self.year.unsigned_abs();
        write!(f, "{year:04}-{:02}-{:02}", self.month, self.day)
    }
}

fn expect_delimiter(
    token: Option<&&str>,
    delimiter: &str,
    kind: &str,
    text: &str,
    reason: &str,
) -> Result<()> {
    match token {
        None => Err(bad(kind, text, "Too short")),
        Some(t) if *t == delimiter => Ok(()),
        Some(_) => Err(bad(kind, text, reason)),
    }
}

fn fmt_timezone(timezone: Option<i16>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match timezone {
        None => Ok(()),
        Some(0) => f.write_str("Z"),
        Some(tz) => {
            let sign = if tz > 0 { '+' } else { '-' };
            let tz = tz.unsigned_abs();
            write!(f, "{sign}{:02}:{:02}", tz / 60, tz % 60)
        }
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_date(f)?;
        fmt_timezone(self.timezone, f)
    }
}

impl DateTime {
    /// A date-time from a date (whose timezone it takes) and a time of day.
    ///
    /// # Errors
    ///
    /// `FORG0001` for components out of range.
    pub fn new(date: Date, hour: u8, minute: u8, second: u8, nanosecond: u32) -> Result<Self> {
        if hour > 23 || minute > 59 || second > 59 || nanosecond > 999_999_999 {
            return Err(XdmError::new(ErrorCode::Forg0001, "Time out of range"));
        }
        Ok(Self {
            date,
            hour,
            minute,
            second,
            nanosecond,
        })
    }

    /// The date part, with the timezone (`xs:date` cast).
    #[must_use]
    pub const fn date(&self) -> Date {
        self.date
    }

    /// The timezone offset in minutes, if any.
    #[must_use]
    pub const fn timezone(&self) -> Option<i16> {
        self.date.timezone
    }

    /// Parses the lexical form of `xs:dateTime` the way Saxon does
    /// (`DateTimeValue.makeDateTimeValue`), including `24:00:00` as midnight of the
    /// next day and fractional seconds beyond nine digits truncated.
    ///
    /// # Errors
    ///
    /// `FORG0001` for an invalid value, `FODT0001` for a year beyond `2^31 - 1`.
    #[expect(
        clippy::too_many_lines,
        reason = "a literal transcription of the reference parser, step by step"
    )]
    pub fn parse(text: &str) -> Result<Self> {
        const KIND: &str = "dateTime";
        let trimmed = crate::double::trim_xml_whitespace(text);
        let tokens = tokenize(trimmed, "-:.+TZ");
        let mut tokens = tokens.iter();
        let mut part = *tokens.next().ok_or_else(|| bad(KIND, text, "too short"))?;
        let mut era = 1;
        if part == "+" {
            return Err(bad(KIND, text, "Date must not start with '+' sign"));
        }
        if part == "-" {
            era = -1;
            part = tokens
                .next()
                .ok_or_else(|| bad(KIND, text, "No year after '-'"))?;
        }
        let year = match simple_integer(part) {
            Ok(v) => v * era,
            Err(SimpleIntegerError::NotNumeric) => {
                return Err(bad(KIND, text, "Non-numeric year component"));
            }
            Err(SimpleIntegerError::TooLarge) => {
                return Err(XdmError::new(
                    ErrorCode::Fodt0001,
                    format!(
                        "Invalid dateTime \"{text}\" (Year is outside the range that Saxon can handle)"
                    ),
                ));
            }
        };
        let year_length = part.chars().count();
        if year_length < 4 {
            return Err(bad(KIND, text, "Year is less than four digits"));
        }
        if year_length > 4 && part.starts_with('0') {
            return Err(bad(
                KIND,
                text,
                "When year exceeds 4 digits, leading zeroes are not allowed",
            ));
        }
        expect_delimiter(tokens.next(), "-", KIND, text, "Wrong delimiter after year")?;
        let month = two_digits(tokens.next(), KIND, text, "Month")?;
        if !(1..=12).contains(&month) {
            return Err(bad(KIND, text, "Month is out of range"));
        }
        expect_delimiter(
            tokens.next(),
            "-",
            KIND,
            text,
            "Wrong delimiter after month",
        )?;
        let day = two_digits(tokens.next(), KIND, text, "Day")?;
        if !(1..=31).contains(&day) {
            return Err(bad(KIND, text, "Day is out of range"));
        }
        expect_delimiter(tokens.next(), "T", KIND, text, "Wrong delimiter after day")?;
        let mut hour = two_digits(tokens.next(), KIND, text, "Hour")?;
        if hour > 24 {
            return Err(bad(KIND, text, "Hour is out of range"));
        }
        expect_delimiter(tokens.next(), ":", KIND, text, "Wrong delimiter after hour")?;
        let minute = two_digits(tokens.next(), KIND, text, "Minute")?;
        if minute > 59 {
            return Err(bad(KIND, text, "Minute is out of range"));
        }
        if hour == 24 && minute != 0 {
            return Err(bad(KIND, text, "If hour is 24, minute must be 00"));
        }
        expect_delimiter(
            tokens.next(),
            ":",
            KIND,
            text,
            "Wrong delimiter after minute",
        )?;
        let second = two_digits(tokens.next(), KIND, text, "Second")?;
        if second > 59 {
            return Err(bad(KIND, text, "Second is out of range"));
        }
        if hour == 24 && second != 0 {
            return Err(bad(KIND, text, "If hour is 24, second must be 00"));
        }
        let mut nanosecond = 0_u32;
        let mut timezone: Option<i16> = None;
        let mut tz_minutes = 0_i32;
        let mut negative_tz = false;
        let mut state = 0_u8;
        while let Some(&delimiter) = tokens.next() {
            if state == 9 {
                return Err(bad(KIND, text, "Characters after the end"));
            }
            match delimiter {
                "." => {
                    if state != 0 {
                        return Err(bad(KIND, text, "Decimal separator occurs twice"));
                    }
                    let mut fraction = *tokens.next().ok_or_else(|| {
                        bad(KIND, text, "Decimal point must be followed by digits")
                    })?;
                    if fraction.chars().count() > 9 && fraction.bytes().all(|b| b.is_ascii_digit())
                    {
                        fraction = &fraction[..9];
                    }
                    if simple_integer(fraction).is_err() {
                        return Err(bad(KIND, text, "Non-numeric fractional seconds component"));
                    }
                    let seconds: f64 = format!(".{fraction}").parse().unwrap_or(0.0);
                    let mut nanos = java_math_round(seconds * 1_000_000_000.0);
                    if nanos == 1_000_000_000 {
                        nanos -= 1;
                    }
                    nanosecond = u32::try_from(nanos).unwrap_or(0);
                    if hour == 24 && nanosecond != 0 {
                        return Err(bad(
                            KIND,
                            text,
                            "If hour is 24, fractional seconds must be 0",
                        ));
                    }
                    state = 1;
                }
                "Z" => {
                    if state > 1 {
                        return Err(bad(KIND, text, "Z cannot occur here"));
                    }
                    state = 9;
                    timezone = Some(0);
                }
                "+" | "-" => {
                    if state > 1 {
                        return Err(bad(KIND, text, &format!("{delimiter} cannot occur here")));
                    }
                    state = 2;
                    let hour_part = *tokens
                        .next()
                        .ok_or_else(|| bad(KIND, text, "Missing timezone"))?;
                    if hour_part.chars().count() != 2 {
                        return Err(bad(KIND, text, "Timezone hour must be two digits"));
                    }
                    let tz_hour = simple_integer(hour_part)
                        .map_err(|_| bad(KIND, text, "Non-numeric timezone hour component"))?;
                    if tz_hour > 14 {
                        return Err(bad(
                            KIND,
                            text,
                            "Timezone is out of range (-14:00 to +14:00)",
                        ));
                    }
                    tz_minutes = tz_hour * 60;
                    negative_tz = delimiter == "-";
                }
                ":" => {
                    if state != 2 {
                        return Err(bad(KIND, text, "Misplaced ':'"));
                    }
                    state = 9;
                    // Saxon reads the next token without checking that there is one
                    // and fails with an internal error; this reports a lexical error.
                    let minute_part = *tokens
                        .next()
                        .ok_or_else(|| bad(KIND, text, "No minutes in timezone"))?;
                    let tz_minute = simple_integer(minute_part)
                        .map_err(|_| bad(KIND, text, "Non-numeric timezone minute component"))?;
                    if minute_part.chars().count() != 2 {
                        return Err(bad(KIND, text, "Timezone minute must be two digits"));
                    }
                    if tz_minute > 59 {
                        return Err(bad(KIND, text, "Timezone minute is out of range"));
                    }
                    if tz_minutes == 14 * 60 && tz_minute != 0 {
                        return Err(bad(
                            KIND,
                            text,
                            "Timezone is out of range (-14:00 to +14:00)",
                        ));
                    }
                    let total = tz_minutes + tz_minute;
                    let total = if negative_tz { -total } else { total };
                    timezone = Some(i16::try_from(total).unwrap_or(0));
                }
                _ => return Err(bad(KIND, text, "Timezone format is incorrect")),
            }
        }
        if state == 2 || state == 3 {
            return Err(bad(KIND, text, "Timezone incomplete"));
        }
        let midnight = hour == 24;
        if midnight {
            hour = 0;
        }
        if !is_valid_date(year, month, day) {
            return Err(bad(KIND, text, "Non-existent date"));
        }
        let (mut year, mut month, mut day) = (year, month, day);
        if midnight {
            if is_valid_date(year, month, day + 1) {
                day += 1;
            } else if month < 12 {
                month += 1;
                day = 1;
            } else {
                year += 1;
                month = 1;
                day = 1;
            }
        }
        Ok(Self {
            date: Date {
                year,
                month,
                day,
                timezone,
            },
            hour,
            minute,
            second,
            nanosecond,
        })
    }

    /// Nanoseconds since 1970-01-01T00:00:00Z, applying the implicit timezone
    /// (minutes) when the value has none.
    fn instant(&self, implicit_timezone: i16) -> i128 {
        let days = i128::from(days_from_civil(
            self.date.year,
            self.date.month,
            self.date.day,
        ));
        let seconds = days * 86_400
            + i128::from(self.hour) * 3600
            + i128::from(self.minute) * 60
            + i128::from(self.second)
            - i128::from(self.date.timezone.unwrap_or(implicit_timezone)) * 60;
        seconds * 1_000_000_000 + i128::from(self.nanosecond)
    }

    /// Compares two date-times as instants, applying the implicit timezone (minutes)
    /// to a value without timezone.
    #[must_use]
    pub fn compare(&self, other: &Self, implicit_timezone: i16) -> Ordering {
        self.instant(implicit_timezone)
            .cmp(&other.instant(implicit_timezone))
    }
}

/// Java's `Math.round(double)`: the closest integer, ties towards positive infinity.
fn java_math_round(value: f64) -> i64 {
    let floor = value.floor();
    let rounded = if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Java's saturating conversion to long; the values here are below 1e9"
    )]
    let result = rounded as i64;
    result
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.date.fmt_date(f)?;
        write!(f, "T{:02}:{:02}:{:02}", self.hour, self.minute, self.second)?;
        if self.nanosecond != 0 {
            let digits = format!("{:09}", self.nanosecond);
            write!(f, ".{}", digits.trim_end_matches('0'))?;
        }
        fmt_timezone(self.date.timezone, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(
            Date::parse(" 2020-02-29 ").unwrap().to_string(),
            "2020-02-29"
        );
        assert!(Date::parse("2021-02-29").is_err());
        assert_eq!(Date::parse("0000-01-01").unwrap().to_string(), "0000-01-01");
        assert_eq!(
            Date::parse("-0001-01-01").unwrap().to_string(),
            "-0001-01-01"
        );
        assert_eq!(
            Date::parse("2020-01-01-00:00").unwrap().to_string(),
            "2020-01-01Z"
        );
        assert_eq!(
            Date::parse("12020-01-01+05:30").unwrap().to_string(),
            "12020-01-01+05:30"
        );
        assert!(Date::parse("02020-01-01").is_err());
        assert_eq!(
            Date::parse("99999999999-01-01").unwrap_err().code(),
            ErrorCode::Fodt0001
        );
        let a = Date::parse("2020-01-02+10:00").unwrap();
        let b = Date::parse("2020-01-01Z").unwrap();
        assert_eq!(a.compare(&b, 0), Ordering::Greater);
    }

    #[test]
    fn date_times() {
        assert_eq!(
            DateTime::parse("2020-12-31T24:00:00").unwrap().to_string(),
            "2021-01-01T00:00:00"
        );
        assert_eq!(
            DateTime::parse("2020-01-01T10:00:00.1234567891Z")
                .unwrap()
                .to_string(),
            "2020-01-01T10:00:00.123456789Z"
        );
        assert_eq!(
            DateTime::parse("2020-01-01T10:00:00.500+01:00")
                .unwrap()
                .to_string(),
            "2020-01-01T10:00:00.5+01:00"
        );
        assert!(DateTime::parse("2020-01-01T10:00:00+01:").is_err());
        assert!(DateTime::parse("2020-01-01T10:00").is_err());
    }
}

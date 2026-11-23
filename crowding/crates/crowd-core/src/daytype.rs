//! Day types used to group service days: weekday, Saturday, and Sunday or public holiday.

use chrono::{Datelike, NaiveDate, Weekday};
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

pub const N_DAYTYPES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DayType {
    Weekday,
    Saturday,
    /// Sundays and public holidays.
    Sunday,
}

impl DayType {
    pub const ALL: [DayType; N_DAYTYPES] = [DayType::Weekday, DayType::Saturday, DayType::Sunday];

    pub fn idx(self) -> usize {
        self as usize
    }

    pub fn from_idx(i: usize) -> DayType {
        Self::ALL[i]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DayType::Weekday => "weekday",
            DayType::Saturday => "saturday",
            DayType::Sunday => "sunday",
        }
    }

    pub fn parse(s: &str) -> Option<DayType> {
        match s.trim().to_ascii_lowercase().as_str() {
            "weekday" | "wd" | "mon-fri" => Some(DayType::Weekday),
            "saturday" | "sat" => Some(DayType::Saturday),
            "sunday" | "sun" | "holiday" => Some(DayType::Sunday),
            _ => None,
        }
    }
}

/// Maps calendar dates to day types, with a configurable holiday list.
#[derive(Debug, Clone, Default)]
pub struct DayCalendar {
    holidays: FxHashSet<NaiveDate>,
}

impl DayCalendar {
    pub fn new(holidays: &[String]) -> anyhow::Result<Self> {
        let holidays = holidays
            .iter()
            .map(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d"))
            .collect::<Result<_, _>>()?;
        Ok(Self { holidays })
    }

    pub fn is_holiday(&self, d: NaiveDate) -> bool {
        self.holidays.contains(&d)
    }

    pub fn day_type(&self, d: NaiveDate) -> DayType {
        if self.holidays.contains(&d) {
            return DayType::Sunday;
        }
        match d.weekday() {
            Weekday::Sat => DayType::Saturday,
            Weekday::Sun => DayType::Sunday,
            _ => DayType::Weekday,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holidays_count_as_sunday() {
        let cal = DayCalendar::new(&["2026-10-02".into()]).unwrap();
        let d = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert_eq!(cal.day_type(d("2026-10-01")), DayType::Weekday);
        assert_eq!(cal.day_type(d("2026-10-02")), DayType::Sunday);
        assert_eq!(cal.day_type(d("2026-10-03")), DayType::Saturday);
        assert_eq!(cal.day_type(d("2026-10-04")), DayType::Sunday);
    }
}

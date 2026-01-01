use chrono::NaiveDate;
use ical::parser::ical::component::IcalEvent;
use ical::IcalParser;
use std::io::BufReader;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WasteType {
    Bio,
    Rest,
    Paper,
    Yellow,
    ChristmasTree,
    Other(String),
}

impl WasteType {
    pub fn as_str(&self) -> &str {
        match self {
            WasteType::Bio => "Bio",
            WasteType::Rest => "Rest",
            WasteType::Paper => "Papier",
            WasteType::Yellow => "Gelb",
            WasteType::ChristmasTree => "Weihnachtsbaum",
            WasteType::Other(s) => s.as_str(),
        }
    }

    pub fn supported_types() -> Vec<WasteType> {
        vec![
            WasteType::Bio,
            WasteType::Rest,
            WasteType::Paper,
            WasteType::Yellow,
            WasteType::ChristmasTree,
        ]
    }

    pub fn default_subscriptions() -> Vec<WasteType> {
        vec![
            WasteType::Bio,
            WasteType::Rest,
            WasteType::Paper,
            WasteType::Yellow,
        ]
    }
}

impl FromStr for WasteType {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim();
        match normalized {
            "Bio" | "Biotonne" => Ok(WasteType::Bio),
            "Rest" | "Restmüll" | "Restabfall" => Ok(WasteType::Rest),
            "Papier" | "Pappe" | "Blaue Tonne" => Ok(WasteType::Paper),
            "Gelb" | "Gelbe Tonne" | "Gelber Sack" => Ok(WasteType::Yellow),
            "Weihnachtsbaum" | "Weihnachtsbäume" => Ok(WasteType::ChristmasTree),
            _ => Ok(WasteType::Other(normalized.to_string())),
        }
    }
}

impl std::fmt::Display for WasteType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickupEvent {
    pub date: NaiveDate,
    pub waste_types: Vec<WasteType>,
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Failed to parse iCal content")]
    IcalError(#[from] ical::parser::ParserError),
    #[error("Missing date in event")]
    MissingDate,
    #[error("Invalid date format: {0}")]
    InvalidDate(String),
    #[error("Missing summary in event")]
    MissingSummary,
}

pub fn normalize_waste_types(summary: &str) -> Vec<WasteType> {
    summary
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().expect("WasteType parsing is infallible"))
        .collect()
}

pub fn parse_ical(content: &str) -> Result<Vec<PickupEvent>, ParseError> {
    let buf = BufReader::new(content.as_bytes());
    let parser = IcalParser::new(buf);

    let mut events = Vec::new();

    for line in parser {
        let mut calendar = line?;

        // Optimization: consume events instead of iterating with reference
        for event in std::mem::take(&mut calendar.events) {
            let (date, summary) = extract_event_data(event)?;
            let waste_types = normalize_waste_types(&summary);

            events.push(PickupEvent { date, waste_types });
        }
    }

    Ok(events)
}

fn extract_event_data(event: IcalEvent) -> Result<(NaiveDate, String), ParseError> {
    let mut date = None;
    let mut summary = None;

    // Optimization: consume properties to move strings instead of cloning
    for prop in event.properties {
        match prop.name.as_str() {
            "DTSTART" => {
                if let Some(val) = prop.value {
                    // Handle YYYYMMDD
                    // Sometimes it might be longer or have timezone, but usually for city waste it's YYYYMMDD
                    // val is owned, but we need to split it.
                    let val_clean = val.split('T').next().unwrap_or(&val);
                    date = Some(
                        NaiveDate::parse_from_str(val_clean, "%Y%m%d")
                            .map_err(|_| ParseError::InvalidDate(val.clone()))?,
                    );
                }
            }
            "SUMMARY" => {
                // Move the value instead of cloning
                summary = prop.value;
            }
            _ => {}
        }
    }

    Ok((
        date.ok_or(ParseError::MissingDate)?,
        summary.ok_or(ParseError::MissingSummary)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_waste_types() {
        let input = "Bio, Rest";
        let output = normalize_waste_types(input);
        assert_eq!(output, vec![WasteType::Bio, WasteType::Rest]);

        let input = "Gelb";
        let output = normalize_waste_types(input);
        assert_eq!(output, vec![WasteType::Yellow]);

        let input = "Rest, Bio, Blaue Tonne";
        let output = normalize_waste_types(input);
        assert_eq!(
            output,
            vec![WasteType::Rest, WasteType::Bio, WasteType::Paper]
        );

        let input = "";
        let output = normalize_waste_types(input);
        assert!(output.is_empty());

        let input = "UnknownGarbage";
        let output = normalize_waste_types(input);
        assert_eq!(output, vec![WasteType::Other("UnknownGarbage".to_string())]);

        // Edge cases
        let input = " Bio ,  Rest ";
        let output = normalize_waste_types(input);
        assert_eq!(output, vec![WasteType::Bio, WasteType::Rest]);
    }

    #[test]
    fn test_parse_ical() {
        let ical_content = "BEGIN:VCALENDAR
BEGIN:VEVENT
DTSTART:20231027
SUMMARY:Bio, Rest
END:VEVENT
BEGIN:VEVENT
DTSTART:20231028
SUMMARY:Gelb
END:VEVENT
END:VCALENDAR";

        let events = parse_ical(ical_content).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].date,
            NaiveDate::from_ymd_opt(2023, 10, 27).unwrap()
        );
        assert_eq!(events[0].waste_types, vec![WasteType::Bio, WasteType::Rest]);
        assert_eq!(
            events[1].date,
            NaiveDate::from_ymd_opt(2023, 10, 28).unwrap()
        );
        assert_eq!(events[1].waste_types, vec![WasteType::Yellow]);
    }
}

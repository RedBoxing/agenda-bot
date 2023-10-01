use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Europe::Paris;
use chrono_tz::Tz;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashMap;

const ISO_8601: &str = "%Y%m%dT%H%M%SZ";

lazy_static! {
    static ref CALENDAR_URL: String = std::env::var("CALENDAR_URL").expect("CALENDAR_URL not set!");
    static ref ROLE_REGEX: Regex = Regex::new("[1-4]-[A-Z]*-[1-4][1-2]").unwrap();
    static ref CLASS_TYPE_REGEX: Regex =
        Regex::new("(S|R)[1-9].[0-9][0-9](-|_)(CM|TD|TP)").unwrap();
    static ref GROUP_REGEX: Regex =
        Regex::new("[1-4]-[A-Z]*-((S[1-4])|([1-4])|([1-4][1-2]))").unwrap();
}

static CALENDAR_CACHE: (i64, Vec<Event>) = (0, Vec::new());

#[derive(Debug, Clone)]
pub enum EventType {
    CM,
    TD,
    TP,
    OTHER,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub summary: String,
    pub start: DateTime<Tz>,
    pub end: DateTime<Tz>,
    pub location: String,
    pub lesson: String,
    pub group: String,
    pub teacher: Option<String>,
    pub event_type: EventType,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub enum Department {
    INFO,
    GEII,
    RT,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct Promo {
    pub year: i8,
    pub deparment: Department,
    pub group: i8,
}

impl std::fmt::Display for Promo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let department = match self.deparment {
            Department::INFO => "INFO",
            Department::GEII => "GEII",
            Department::RT => "RT",
        };

        write!(f, "{}-{}-{}", self.year, department, self.group)
    }
}

async fn fetch_events() -> Result<Vec<Event>, String> {
    let now = Utc::now().timestamp_millis();
    if now - CALENDAR_CACHE.0 < 1000 * 60 * 10 {
        return Ok(CALENDAR_CACHE.1.clone());
    }

    let body = reqwest::get(CALENDAR_URL.as_str())
        .await
        .expect("Failed to fetch calendar!")
        .text()
        .await
        .expect("Failed to read calendar!");
    let unfolded = icalendar::parser::unfold(&body);
    let res = icalendar::parser::read_calendar(&unfolded);
    let mut events: Vec<Event> = Vec::new();

    if let Ok(calendar) = res {
        calendar.components.iter().for_each(|c| {
            let summary = c
                .properties
                .iter()
                .find(|p| p.name == "SUMMARY")
                .expect("Failed to find summary");

            let start_datetime = c
                .properties
                .iter()
                .find(|p| p.name == "DTSTART")
                .expect("Failed to find start");

            let end_datetime = c
                .properties
                .iter()
                .find(|p| p.name == "DTEND")
                .expect("Failed to find end");

            let location = c
                .properties
                .iter()
                .find(|p| p.name == "LOCATION")
                .expect("Failed to find location");

            let description = c
                .properties
                .iter()
                .find(|p| p.name == "DESCRIPTION")
                .expect("Failed to find description");

            let start = NaiveDateTime::parse_from_str(start_datetime.val.as_str(), ISO_8601);
            let end = NaiveDateTime::parse_from_str(end_datetime.val.as_str(), ISO_8601);

            let split = description
                .val
                .as_str()
                .split("\\n\\n")
                .collect::<Vec<&str>>();
            let split2 = split[1].split("\\n").collect::<Vec<&str>>();

            let event = Event {
                summary: summary.val.as_str().to_string(),
                start: Paris.from_utc_datetime(&start.unwrap()),
                end: Paris.from_utc_datetime(&end.unwrap()),
                location: location.val.as_str().to_string(),
                lesson: split[0].to_string(),
                group: split2[0].to_string(),
                teacher: if split2.len() > 1 {
                    Some(split2[1].to_string())
                } else {
                    None
                },
                event_type: if CLASS_TYPE_REGEX.is_match(summary.val.as_str()) {
                    let event_type = &summary.val.as_str()[6..8];
                    match event_type {
                        "TD" => EventType::TD,
                        "TP" => EventType::TP,
                        "CM" => EventType::CM,
                        _ => EventType::OTHER,
                    }
                } else {
                    EventType::OTHER
                },
            };

            events.push(event);
        });

        Ok(events)
    } else {
        Err("Failed to parse calendar!".to_string())
    }
}

fn set_events(name: &str, event: Event, current_list: &mut HashMap<Promo, Vec<Event>>) {
    let split = name.split("-").collect::<Vec<&str>>();
    let promo = parse_promo_name(name);
    if promo.is_none() {
        println!("Failed to parse promo name: {}", name);
        return;
    }
    let promo = promo.unwrap();
    let group_name = split[2];

    if group_name.starts_with("S") {
        for i in 1..5 {
            for j in 1..3 {
                let new_group_name = format!("{}{}", i, j);
                let mut promo = promo.clone();
                promo.group = new_group_name.parse::<i8>().unwrap();
                let mut group_events = if let Some(events) = current_list.get(&promo.clone()) {
                    events.clone()
                } else {
                    Vec::new()
                };

                group_events.push(event.clone());
                group_events.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
                current_list.insert(promo, group_events);
            }

            let mut promo = promo.clone();
            promo.group = i;

            let mut group_events = if let Some(events) = current_list.get(&promo.clone()) {
                events.clone()
            } else {
                Vec::new()
            };

            group_events.push(event.clone());
            group_events.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
            current_list.insert(promo, group_events);
        }

        let mut group_events = if let Some(events) = current_list.get(&promo.clone()) {
            events.clone()
        } else {
            Vec::new()
        };

        group_events.push(event.clone());
        group_events.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
        current_list.insert(promo.clone(), group_events);
    } else if group_name.len() == 2 {
        let mut group_events = if let Some(events) = current_list.get(&promo.clone()) {
            events.clone()
        } else {
            Vec::new()
        };

        group_events.push(event.clone());
        group_events.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
        current_list.insert(promo.clone(), group_events);
    } else if group_name.len() == 1 {
        for i in 1..3 {
            let new_group_name = format!("{}{}", group_name, i);
            let mut promo = promo.clone();
            promo.group = new_group_name.parse::<i8>().unwrap();
            let mut group_events = if let Some(events) = current_list.get(&promo.clone()) {
                events.clone()
            } else {
                Vec::new()
            };

            group_events.push(event.clone());
            group_events.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
            current_list.insert(promo, group_events);
        }

        let mut group_events = if let Some(events) = current_list.get(&promo.clone()) {
            events.clone()
        } else {
            Vec::new()
        };

        group_events.push(event.clone());
        group_events.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
        current_list.insert(promo.clone(), group_events);
    }
}

pub async fn get_sorted_events(day: NaiveDate) -> Result<HashMap<Promo, Vec<Event>>, String> {
    let res = fetch_events().await;
    if let Ok(events) = res {
        let mut map: HashMap<Promo, Vec<Event>> = HashMap::new();

        // only show events for today
        for evt in events
            .iter()
            .filter(|e| {
                e.start.date_naive() >= day && e.end.date_naive() < day + chrono::Duration::days(1)
            })
            .collect::<Vec<&Event>>()
        {
            set_events(&evt.group, evt.clone(), &mut map);
        }

        Ok(map)
    } else {
        Err(res.unwrap_err())
    }
}

pub fn parse_promo_name(name: &str) -> Option<Promo> {
    if !GROUP_REGEX.is_match(name) {
        return None;
    }

    let split = name.split("-").collect::<Vec<&str>>();
    let year = split[0].parse::<i8>();
    if year.is_err() {
        return None;
    }

    let year = year.unwrap();
    let department = match split[1] {
        "INFO" => Department::INFO,
        "GEII" => Department::GEII,
        "RT" => Department::RT,
        _ => {
            return None;
        }
    };

    let group = if split[2].starts_with("S") {
        0
    } else {
        let parsed = split[2].parse::<i8>();
        if parsed.is_err() {
            return None;
        }

        parsed.unwrap()
    };

    let promo = Promo {
        year: year,
        deparment: department,
        group: group,
    };

    Some(promo)
}

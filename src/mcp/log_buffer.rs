use std::{collections::VecDeque, error::Error, fmt, str::FromStr};

use crate::{
    filter::Filter,
    log_entry::{LogEntry, LogLevel},
};

pub const DEFAULT_LOG_BUFFER_CAPACITY: usize = 50_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimestampParseError {
    raw: String,
    reason: &'static str,
}

impl TimestampParseError {
    fn new(raw: &str, reason: &'static str) -> Self {
        Self {
            raw: raw.to_string(),
            reason,
        }
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

impl fmt::Display for TimestampParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.reason, self.raw)
    }
}

impl Error for TimestampParseError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedTimestamp {
    pub canonical: String,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millisecond: u16,
    sort_key: u64,
}

impl NormalizedTimestamp {
    pub fn parse(raw: &str) -> Result<Self, TimestampParseError> {
        let raw = raw.trim();
        if raw.len() != 18 {
            return Err(TimestampParseError::new(
                raw,
                "expected MM-DD HH:MM:SS.mmm timestamp",
            ));
        }

        let bytes = raw.as_bytes();
        if bytes[2] != b'-'
            || bytes[5] != b' '
            || bytes[8] != b':'
            || bytes[11] != b':'
            || bytes[14] != b'.'
        {
            return Err(TimestampParseError::new(
                raw,
                "expected MM-DD HH:MM:SS.mmm timestamp",
            ));
        }

        let month = parse_u8_component(raw, 0, 2, "invalid month")?;
        let day = parse_u8_component(raw, 3, 5, "invalid day")?;
        let hour = parse_u8_component(raw, 6, 8, "invalid hour")?;
        let minute = parse_u8_component(raw, 9, 11, "invalid minute")?;
        let second = parse_u8_component(raw, 12, 14, "invalid second")?;
        let millisecond = parse_u16_component(raw, 15, 18, "invalid millisecond")?;

        if !(1..=12).contains(&month) {
            return Err(TimestampParseError::new(raw, "month out of range"));
        }
        if !(1..=31).contains(&day) {
            return Err(TimestampParseError::new(raw, "day out of range"));
        }
        if hour > 23 {
            return Err(TimestampParseError::new(raw, "hour out of range"));
        }
        if minute > 59 {
            return Err(TimestampParseError::new(raw, "minute out of range"));
        }
        if second > 59 {
            return Err(TimestampParseError::new(raw, "second out of range"));
        }

        let canonical =
            format!("{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millisecond:03}");
        let sort_key = (((((u64::from(month) * 100 + u64::from(day)) * 100 + u64::from(hour))
            * 100
            + u64::from(minute))
            * 100
            + u64::from(second))
            * 1000)
            + u64::from(millisecond);

        Ok(Self {
            canonical,
            month,
            day,
            hour,
            minute,
            second,
            millisecond,
            sort_key,
        })
    }

    pub fn sort_key(&self) -> u64 {
        self.sort_key
    }
}

impl fmt::Display for NormalizedTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical)
    }
}

impl FromStr for NormalizedTimestamp {
    type Err = TimestampParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[derive(Debug, Clone)]
pub struct BufferedLogEntry {
    pub seq: u64,
    pub normalized_timestamp: Option<NormalizedTimestamp>,
    pub entry: LogEntry,
}

impl BufferedLogEntry {
    fn new(seq: u64, entry: LogEntry) -> Self {
        let normalized_timestamp = NormalizedTimestamp::parse(&entry.timestamp).ok();
        Self {
            seq,
            normalized_timestamp,
            entry,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageOrder {
    Asc,
    #[default]
    Desc,
}

#[derive(Debug, Clone)]
pub struct LogQuery {
    /// Exclusive sequence cursor:
    /// - Asc returns entries with seq > cursor
    /// - Desc returns entries with seq < cursor
    pub cursor: Option<u64>,
    pub order: PageOrder,
    pub limit: usize,
    pub min_level: Option<LogLevel>,
    pub tag_query: Option<String>,
    pub text: Option<String>,
    pub process: Option<String>,
    pub subsystem: Option<String>,
    pub category: Option<String>,
    pub since: Option<NormalizedTimestamp>,
}

impl Default for LogQuery {
    fn default() -> Self {
        Self {
            cursor: None,
            order: PageOrder::Desc,
            limit: 100,
            min_level: None,
            tag_query: None,
            text: None,
            process: None,
            subsystem: None,
            category: None,
            since: None,
        }
    }
}

impl LogQuery {
    pub fn set_tag_query(&mut self, input: &str) {
        let trimmed = input.trim();
        self.tag_query = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }

    pub fn set_since_str(&mut self, raw: &str) -> Result<(), TimestampParseError> {
        self.since = Some(NormalizedTimestamp::parse(raw)?);
        Ok(())
    }

    fn matches_cursor(&self, seq: u64) -> bool {
        match self.cursor {
            Some(cursor) => match self.order {
                PageOrder::Asc => seq > cursor,
                PageOrder::Desc => seq < cursor,
            },
            None => true,
        }
    }

    fn build_filter(&self) -> Filter {
        let mut filter = Filter {
            min_level: self.min_level.unwrap_or(LogLevel::Verbose),
            package: None,
            ios_process: self.process.clone(),
            ios_subsystem: self.subsystem.clone(),
            ios_category: self.category.clone(),
            tag_filters: self
                .tag_query
                .as_deref()
                .map(Filter::parse_tag_filters)
                .unwrap_or_default(),
            search_query: String::new(),
            search_regex: None,
            hide_vendor_noise: false,
        };

        if let Some(text) = self
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            filter.set_search(text);
        }

        filter
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogBufferMeta {
    pub capacity: usize,
    pub len: usize,
    pub dropped: u64,
    pub next_seq: u64,
    pub oldest_seq: Option<u64>,
    pub newest_seq: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogPageMeta {
    pub buffer: LogBufferMeta,
    pub cursor: Option<u64>,
    pub first_seq: Option<u64>,
    pub last_seq: Option<u64>,
    pub next_cursor: Option<u64>,
    pub returned: usize,
    pub limit: usize,
    pub order: PageOrder,
    pub has_more: bool,
}

#[derive(Debug, Clone)]
pub struct LogPage {
    pub entries: Vec<BufferedLogEntry>,
    pub meta: LogPageMeta,
}

#[derive(Debug)]
pub struct LogBuffer {
    capacity: usize,
    entries: VecDeque<BufferedLogEntry>,
    next_seq: u64,
    dropped: u64,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            entries: VecDeque::with_capacity(capacity),
            next_seq: 1,
            dropped: 0,
        }
    }

    pub fn append(&mut self, entry: LogEntry) -> u64 {
        let seq = self.next_seq;
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .expect("log buffer sequence overflowed");

        if self.entries.len() == self.capacity {
            self.entries.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }

        self.entries.push_back(BufferedLogEntry::new(seq, entry));
        seq
    }

    /// Clears buffered entries and eviction counters while keeping sequence IDs monotonic.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.dropped = 0;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn meta(&self) -> LogBufferMeta {
        LogBufferMeta {
            capacity: self.capacity,
            len: self.entries.len(),
            dropped: self.dropped,
            next_seq: self.next_seq,
            oldest_seq: self.entries.front().map(|entry| entry.seq),
            newest_seq: self.entries.back().map(|entry| entry.seq),
        }
    }

    pub fn query(&self, query: &LogQuery) -> LogPage {
        let mut entries = Vec::with_capacity(query.limit.min(self.entries.len()));

        if query.limit == 0 {
            return LogPage {
                entries,
                meta: LogPageMeta {
                    buffer: self.meta(),
                    cursor: query.cursor,
                    first_seq: None,
                    last_seq: None,
                    next_cursor: None,
                    returned: 0,
                    limit: 0,
                    order: query.order,
                    has_more: false,
                },
            };
        }

        let filter = query.build_filter();
        let mut has_more = false;

        let iter: Box<dyn Iterator<Item = &BufferedLogEntry> + '_> = match query.order {
            PageOrder::Asc => Box::new(self.entries.iter()),
            PageOrder::Desc => Box::new(self.entries.iter().rev()),
        };

        for buffered in iter {
            if !query.matches_cursor(buffered.seq) || !matches_query(buffered, query, &filter) {
                continue;
            }

            if entries.len() < query.limit {
                entries.push(buffered.clone());
            } else {
                has_more = true;
                break;
            }
        }

        let first_seq = entries.first().map(|entry| entry.seq);
        let last_seq = entries.last().map(|entry| entry.seq);
        let returned = entries.len();

        LogPage {
            entries,
            meta: LogPageMeta {
                buffer: self.meta(),
                cursor: query.cursor,
                first_seq,
                last_seq,
                next_cursor: last_seq,
                returned,
                limit: query.limit,
                order: query.order,
                has_more,
            },
        }
    }
}

fn matches_query(buffered: &BufferedLogEntry, query: &LogQuery, filter: &Filter) -> bool {
    if !filter.matches(&buffered.entry, None) || !filter.matches_search(&buffered.entry) {
        return false;
    }

    if let Some(since) = &query.since {
        let Some(entry_timestamp) = buffered.normalized_timestamp.as_ref() else {
            return false;
        };

        if entry_timestamp.sort_key() < since.sort_key() {
            return false;
        }
    }

    true
}

fn parse_u8_component(
    raw: &str,
    start: usize,
    end: usize,
    reason: &'static str,
) -> Result<u8, TimestampParseError> {
    raw.get(start..end)
        .ok_or_else(|| TimestampParseError::new(raw, reason))?
        .parse()
        .map_err(|_| TimestampParseError::new(raw, reason))
}

fn parse_u16_component(
    raw: &str,
    start: usize,
    end: usize,
    reason: &'static str,
) -> Result<u16, TimestampParseError> {
    raw.get(start..end)
        .ok_or_else(|| TimestampParseError::new(raw, reason))?
        .parse()
        .map_err(|_| TimestampParseError::new(raw, reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_entry::LogPlatform;

    fn entry(timestamp: &str, level: LogLevel, tag: &str, message: &str) -> LogEntry {
        LogEntry {
            platform: LogPlatform::Android,
            timestamp: timestamp.to_string(),
            pid: Some(1234),
            tid: Some(5678),
            level,
            tag: tag.to_string(),
            process: None,
            subsystem: None,
            category: None,
            message: message.to_string(),
        }
    }

    #[test]
    fn evicts_oldest_entries_and_tracks_drops() {
        let mut buffer = LogBuffer::new(3);

        assert_eq!(
            buffer.append(entry("03-10 06:30:45.000", LogLevel::Info, "A", "one")),
            1
        );
        assert_eq!(
            buffer.append(entry("03-10 06:30:46.000", LogLevel::Info, "B", "two")),
            2
        );
        assert_eq!(
            buffer.append(entry("03-10 06:30:47.000", LogLevel::Warn, "C", "three")),
            3
        );
        assert_eq!(
            buffer.append(entry("03-10 06:30:48.000", LogLevel::Error, "D", "four")),
            4
        );

        let page = buffer.query(&LogQuery {
            order: PageOrder::Asc,
            limit: 10,
            ..LogQuery::default()
        });

        assert_eq!(buffer.len(), 3);
        assert_eq!(
            page.entries
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
        assert_eq!(buffer.meta().dropped, 1);
        assert_eq!(buffer.meta().oldest_seq, Some(2));
        assert_eq!(buffer.meta().newest_seq, Some(4));
    }

    #[test]
    fn desc_pagination_uses_exclusive_sequence_cursor() {
        let mut buffer = LogBuffer::new(8);
        for second in 0..5 {
            buffer.append(entry(
                &format!("03-10 06:30:{:02}.000", 45 + second),
                LogLevel::Info,
                "App",
                &format!("line {}", second + 1),
            ));
        }

        let first_page = buffer.query(&LogQuery {
            order: PageOrder::Desc,
            limit: 2,
            ..LogQuery::default()
        });

        assert_eq!(
            first_page
                .entries
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![5, 4]
        );
        assert!(first_page.meta.has_more);
        assert_eq!(first_page.meta.next_cursor, Some(4));

        let second_page = buffer.query(&LogQuery {
            cursor: first_page.meta.next_cursor,
            order: PageOrder::Desc,
            limit: 2,
            ..LogQuery::default()
        });

        assert_eq!(
            second_page
                .entries
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert!(second_page.meta.has_more);
        assert_eq!(second_page.meta.next_cursor, Some(2));

        let last_page = buffer.query(&LogQuery {
            cursor: second_page.meta.next_cursor,
            order: PageOrder::Desc,
            limit: 2,
            ..LogQuery::default()
        });

        assert_eq!(
            last_page
                .entries
                .iter()
                .map(|entry| entry.seq)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert!(!last_page.meta.has_more);
        assert_eq!(last_page.meta.next_cursor, Some(1));
    }

    #[test]
    fn combines_level_tag_text_and_since_filters() {
        let mut buffer = LogBuffer::new(8);
        buffer.append(entry(
            "03-10 06:30:45.000",
            LogLevel::Info,
            "App",
            "boot complete",
        ));
        buffer.append(entry(
            "03-10 06:30:46.000",
            LogLevel::Debug,
            "Noise",
            "heartbeat",
        ));
        buffer.append(entry(
            "03-10 06:30:47.000",
            LogLevel::Error,
            "AppWorker",
            "failed to open socket",
        ));
        buffer.append(entry(
            "03-10 06:30:48.000",
            LogLevel::Error,
            "Other",
            "failed to open socket",
        ));

        let mut query = LogQuery {
            order: PageOrder::Asc,
            limit: 10,
            min_level: Some(LogLevel::Warn),
            text: Some("failed".to_string()),
            ..LogQuery::default()
        };
        query.set_tag_query("tag-:Other tag~:^App");
        query.set_since_str("03-10 06:30:46.500").unwrap();

        let page = buffer.query(&query);

        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].seq, 3);
        assert_eq!(page.entries[0].entry.tag, "AppWorker");
        assert_eq!(
            page.entries[0]
                .normalized_timestamp
                .as_ref()
                .map(ToString::to_string),
            Some("03-10 06:30:47.000".to_string())
        );
    }

    #[test]
    fn clear_resets_entries_but_not_sequence_monotonicity() {
        let mut buffer = LogBuffer::new(4);

        assert_eq!(
            buffer.append(entry("03-10 06:30:45.000", LogLevel::Info, "A", "one")),
            1
        );
        assert_eq!(
            buffer.append(entry("03-10 06:30:46.000", LogLevel::Info, "A", "two")),
            2
        );
        buffer.clear();

        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.meta().dropped, 0);
        assert_eq!(
            buffer.append(entry("03-10 06:30:47.000", LogLevel::Warn, "B", "three")),
            3
        );
        assert_eq!(buffer.meta().oldest_seq, Some(3));
        assert_eq!(buffer.meta().newest_seq, Some(3));
    }
}
